use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, DiscoveryTarget, HostCompatibility, PluginCapability, PluginError,
    PluginRole, PluginType, Result,
};
use uptrakit_plugin_infrastructure_core::{
    PluginHttpClientConfig, SsrfMode, build_plugin_http_client,
};
use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::CargoConfig;

const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 64,
    first_char_valid: |c| c.is_ascii_alphabetic() || c == '_',
    char_valid: |c| c.is_ascii_alphanumeric() || c == '_' || c == '-',
    reject_double_dot: true,
};

/// Validate a Cargo crate package identifier.
///
/// Enforces Cargo crate naming rules used by `cargo install`:
/// - Between 1 and 64 characters long.
/// - Must start with an ASCII letter (`A-Za-z`) or underscore (`_`).
/// - Remaining characters: `[A-Za-z0-9_-]` only.
/// - Must not contain `..` or path separators (`/`, `\`).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    IDENTIFIER_RULES.validate(value)
}

/// Returns `true` if the given version string is a semver pre-release.
///
/// A version is considered pre-release when it contains `-` after the numeric
/// parts (e.g. `1.0.0-alpha.1`, `2.0.0-beta`, `0.9.0-rc.1`).
pub(crate) fn is_prerelease_version(version: &str) -> bool {
    version.contains('-')
}

/// Parse `cargo install --list` output into a crate name -> version map.
///
/// The output format is:
/// ```text
/// bat v0.24.0:
///     bat
/// ripgrep v14.1.1:
///     rg
/// cargo-nextest v0.9.85:
///     cargo-nextest
/// local-crate v0.1.0 (/path/to/source):
///     local-crate
/// ```
///
/// Lines not starting with whitespace are crate headers. Each header matches
/// `<name> v<version>[optional (path)]:`. Binary listing lines (indented) are
/// ignored -- the crate name is tracked, not the binary name.
///
/// **Scope:** `cargo install --list` reads `$CARGO_HOME/.crates.toml` (default
/// `~/.cargo`). Crates installed with `--root /custom/path` are stored in a
/// separate `.crates.toml` and are not visible here.
pub fn parse_cargo_install_list(output: &str) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in output.lines() {
        // Skip indented lines (binary name lines).
        if line.starts_with(|c: char| c.is_ascii_whitespace()) || line.is_empty() {
            continue;
        }
        // Must end with ':'.
        let Some(line) = line.strip_suffix(':') else {
            continue;
        };
        // Find " v" to split name from version.
        let Some(v_pos) = line.find(" v") else {
            continue;
        };
        let name = line[..v_pos].trim();
        let after_v = &line[v_pos + 2..]; // skip " v"
        // Version is the first whitespace-delimited token (stops before optional " (path)").
        let version = after_v.split_whitespace().next().unwrap_or("");
        if name.is_empty() || version.is_empty() {
            continue;
        }
        result.insert(name.to_string(), version.to_string());
    }
    result
}

/// Plugin for tracking and updating Rust binaries installed via `cargo install`.
///
/// - **Discovery**: `cargo install --list` -- finds all installed crates and their versions.
/// - **Version detection**: `cargo install --list` -- single call, looked up in the map.
/// - **Release fetching**: crates.io sparse index (`https://index.crates.io`) -- controller-side,
///   bounded parallel HTTP lookups via `buffer_unordered(10)`.
/// - **Updates**: `cargo install <crate> --version <ver> --locked` (default) -- no `sudo` needed.
///
/// A [`DiscoveryTarget`] is always emitted per installed crate so the controller
/// can find-or-create plugin config and role assignments.
pub struct CargoPlugin {
    pub(crate) config: CargoConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
    pub(crate) client: reqwest::Client,
}

impl CargoPlugin {
    /// Compile-time capabilities for the Cargo install plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::DetectHostCompatibility,
        PluginCapability::ControllerSideFetchReleases,
    ];

    /// Create a new Cargo plugin with the given configuration and command executor.
    pub async fn new(config: CargoConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;

        // Use a permissive SSRF resolver for custom (potentially private/LAN)
        // registries and a strict resolver for the default crates.io index.
        let ssrf_mode = if config.registry_url.is_some() {
            SsrfMode::Permissive
        } else {
            SsrfMode::Strict
        };

        let client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-package-manager-cargo/",
                env!("CARGO_PKG_VERSION")
            ),
            ssrf_mode,
            redirect_policy: reqwest::redirect::Policy::limited(10),
            ..Default::default()
        })
        .map_err(|e| report!(PluginError::PluginInternal(e)))?;

        Ok(Self {
            config,
            executor,
            client,
        })
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier).map_err(|e| report!(PluginError::Configuration(e)))
    }
}

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    CargoPlugin,
    CargoConfig,
    "package_manager_cargo",
    {
        fn capabilities(&self) -> Vec<PluginCapability> {
            Self::CAPABILITIES.to_vec()
        }

        fn as_discovery(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::DiscoveryPlugin> {
            Some(self)
        }
        fn as_version_detector(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::VersionDetectorPlugin> {
            Some(self)
        }
        fn as_release_fetcher(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin> {
            Some(self)
        }
        fn as_update_executor(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin> {
            Some(self)
        }
    }
);

#[async_trait]
impl uptrakit_plugin_infrastructure_core::DiscoveryPlugin for CargoPlugin {
    /// Discover crates installed via `cargo install` on the local system.
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering cargo-installed software");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cargo",
                ["install".to_string(), "--list".to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "cargo install --list failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let installed = parse_cargo_install_list(&cmd_output.output);

        let packages: Vec<DiscoveredSoftware> = installed
            .into_iter()
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: PluginType::PackageManagerCargo,
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "cargo".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }];
                DiscoveredSoftware {
                    package_identifier: name.clone(),
                    name,
                    installed_version: version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: true,
                    installed_display_version: None,
                }
            })
            .collect();

        tracing::debug!(count = packages.len(), "cargo software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["cargo".to_string()]))
            .await
        {
            Ok(out) if out.exit_code == 0 => Ok(HostCompatibility::Compatible),
            _ => Ok(HostCompatibility::Incompatible(
                "cargo not found in PATH".to_string(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::FixedOutputExecutor;

    // ── validate_identifier ───────────────────────────────────────────────────

    #[test]
    fn validate_identifier_simple_valid() {
        assert!(validate_identifier("ripgrep").is_ok());
        assert!(validate_identifier("bat").is_ok());
        assert!(validate_identifier("cargo_nextest").is_ok());
        assert!(validate_identifier("cargo-nextest").is_ok());
        assert!(validate_identifier("_helper").is_ok());
        assert!(validate_identifier("a").is_ok());
    }

    #[test]
    fn validate_identifier_with_digits() {
        assert!(validate_identifier("crate1").is_ok());
        assert!(validate_identifier("my_tool2").is_ok());
    }

    #[test]
    fn validate_identifier_uppercase_valid() {
        // Cargo crate names can be mixed-case (unlike snap)
        assert!(validate_identifier("MyTool").is_ok());
        assert!(validate_identifier("RustTool").is_ok());
    }

    #[test]
    fn validate_identifier_empty_fails() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_too_long_fails() {
        assert!(validate_identifier(&"a".repeat(65)).is_err());
    }

    #[test]
    fn validate_identifier_exactly_64_valid() {
        assert!(validate_identifier(&"a".repeat(64)).is_ok());
    }

    #[test]
    fn validate_identifier_leading_digit_fails() {
        assert!(validate_identifier("1crate").is_err());
        assert!(validate_identifier("0tool").is_err());
    }

    #[test]
    fn validate_identifier_leading_hyphen_fails() {
        assert!(validate_identifier("-crate").is_err());
    }

    #[test]
    fn validate_identifier_dot_fails() {
        assert!(validate_identifier("my.crate").is_err());
    }

    #[test]
    fn validate_identifier_space_fails() {
        assert!(validate_identifier("my crate").is_err());
    }

    #[test]
    fn validate_identifier_slash_fails() {
        assert!(validate_identifier("owner/crate").is_err());
    }

    #[test]
    fn validate_identifier_at_sign_fails() {
        assert!(validate_identifier("@scope/crate").is_err());
    }

    // ── is_prerelease_version ─────────────────────────────────────────────────

    #[test]
    fn stable_versions_not_prerelease() {
        assert!(!is_prerelease_version("1.0.0"));
        assert!(!is_prerelease_version("14.1.1"));
        assert!(!is_prerelease_version("0.9.85"));
        assert!(!is_prerelease_version("2.0.0"));
    }

    #[test]
    fn prerelease_versions_detected() {
        assert!(is_prerelease_version("1.0.0-alpha.1"));
        assert!(is_prerelease_version("2.0.0-beta"));
        assert!(is_prerelease_version("0.9.0-rc.1"));
        assert!(is_prerelease_version("1.0.0-alpha"));
    }

    // ── parse_cargo_install_list ──────────────────────────────────────────────

    #[test]
    fn parse_cargo_install_list_basic() {
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let map = parse_cargo_install_list(output);
        assert_eq!(map.get("bat"), Some(&"0.24.0".to_string()));
        assert_eq!(map.get("ripgrep"), Some(&"14.1.1".to_string()));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn parse_cargo_install_list_hyphenated_name() {
        let output = "cargo-nextest v0.9.85:\n    cargo-nextest\n";
        let map = parse_cargo_install_list(output);
        assert_eq!(map.get("cargo-nextest"), Some(&"0.9.85".to_string()));
    }

    #[test]
    fn parse_cargo_install_list_local_crate_with_path() {
        let output = "local-crate v0.1.0 (/home/user/dev/local-crate):\n    local-crate\n";
        let map = parse_cargo_install_list(output);
        assert_eq!(map.get("local-crate"), Some(&"0.1.0".to_string()));
    }

    #[test]
    fn parse_cargo_install_list_multiple_binaries() {
        let output = "ripgrep v14.1.1:\n    rg\n    ripgrep\n";
        let map = parse_cargo_install_list(output);
        // Only one entry for the crate, regardless of how many binaries.
        assert_eq!(map.get("ripgrep"), Some(&"14.1.1".to_string()));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_cargo_install_list_empty_output() {
        let map = parse_cargo_install_list("");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_cargo_install_list_whitespace_only() {
        let map = parse_cargo_install_list("   \n\t\n");
        assert!(map.is_empty());
    }

    #[test]
    fn parse_cargo_install_list_binary_lines_skipped() {
        // Only header lines (no leading whitespace) should contribute entries.
        let output = "    bat\n    rg\nbat v0.24.0:\n    bat\n";
        let map = parse_cargo_install_list(output);
        assert_eq!(map.get("bat"), Some(&"0.24.0".to_string()));
        // The initial "    bat" line must be skipped.
        assert!(!map.contains_key("    bat"));
    }

    // ── detect_host_compatibility ─────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_cargo_found() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(""))
            .await
            .unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_cargo_missing() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        // Exit code != 0 from `which cargo` -> incompatible.
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::failure(1))
            .await
            .unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert!(matches!(result, HostCompatibility::Incompatible(_)));
    }

    // ── required_sudo_commands ────────────────────────────────────────────────

    #[tokio::test]
    async fn required_sudo_commands_empty() {
        use uptrakit_plugin_infrastructure_core::PluginBase;
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(""))
            .await
            .unwrap();
        assert!(plugin.required_sudo_commands().is_empty());
    }

    // ── capabilities ─────────────────────────────────────────────────────────

    #[test]
    fn cargo_capabilities_declared() {
        assert!(CargoPlugin::CAPABILITIES.contains(&PluginCapability::DiscoverLocalSoftware));
        assert!(CargoPlugin::CAPABILITIES.contains(&PluginCapability::DetectHostCompatibility));
        assert!(CargoPlugin::CAPABILITIES.contains(&PluginCapability::ControllerSideFetchReleases));
    }

    // ── discover_software ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_always_emits_targets() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(output))
            .await
            .unwrap();

        let discovered = plugin.discover_software().await.unwrap();
        assert_eq!(discovered.len(), 2);
        for item in &discovered {
            assert_eq!(item.targets.len(), 1);
            assert_eq!(item.targets[0].plugin_type, PluginType::PackageManagerCargo);
            assert_eq!(item.targets[0].plugin_config_name, "cargo");
        }
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_config() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let output = "bat v0.24.0:\n    bat\n";
        let plugin = CargoPlugin::new(
            CargoConfig {
                include_prereleases: true,
                registry_url: None,
                use_locked: true,
            },
            FixedOutputExecutor::success(output),
        )
        .await
        .unwrap();

        let discovered = plugin.discover_software().await.unwrap();
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].targets.len(), 1);
    }

    #[tokio::test]
    async fn discover_software_empty_install_list() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let plugin = CargoPlugin::new(CargoConfig::default(), FixedOutputExecutor::success(""))
            .await
            .unwrap();

        let discovered = plugin.discover_software().await.unwrap();
        assert!(discovered.is_empty());
    }
}
