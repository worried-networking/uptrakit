use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{
    PluginCapability, PluginError, Result, SudoCommandEntry,
};
use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::PacmanConfig;

const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 128,
    first_char_valid: |c| c.is_ascii_lowercase() || c.is_ascii_digit(),
    char_valid: |c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '@' | '.' | '_' | '+' | '-')
    },
    reject_double_dot: true,
};

/// Validate an Arch Linux Pacman package identifier.
///
/// Enforces Arch Linux PKGBUILD naming rules:
/// - Between 1 and 128 characters long.
/// - Must start with a lowercase letter or digit (`[a-z0-9]`).
/// - May only contain lowercase letters, digits, `@`, `.`, `_`, `+`, or `-`.
/// - Must not contain `..` (path traversal protection).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    IDENTIFIER_RULES.validate(value)
}

/// Validate a Pacman package version string before it is interpolated into
/// install commands.
///
/// Allows Arch Linux version characters (`[a-zA-Z0-9.+~:-]`). Rejects:
/// - Empty strings
/// - Strings starting with `-` (could be interpreted as a command-line flag)
/// - Strings exceeding 256 characters
pub fn validate_version(version: &str) -> std::result::Result<(), String> {
    if version.is_empty() {
        return Err("version must not be empty".to_string());
    }
    if version.len() > 256 {
        return Err("version must not exceed 256 characters".to_string());
    }
    if version.starts_with('-') {
        return Err("version must not start with '-' (would be interpreted as a flag)".to_string());
    }
    for ch in version.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '+' | '~' | ':' | '-') {
            return Err(format!("version contains invalid character: '{ch}'"));
        }
    }
    Ok(())
}

/// Plugin for Pacman (Arch Linux package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for Arch Linux packages managed by `pacman`.
///
/// The `package_identifier` in `SoftwareItem` is the Arch Linux package name
/// (e.g., `nginx`, `python`, `git`).
pub struct PacmanPlugin {
    pub(crate) config: PacmanConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
}

impl PacmanPlugin {
    /// Compile-time capabilities for the Pacman plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::RefreshPackageIndex,
        PluginCapability::DetectHostCompatibility,
    ];

    /// Create a new Pacman plugin with the given configuration.
    pub async fn new(config: PacmanConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| rootcause::report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Parse `pacman -Q` or `pacman -Qe` output.
    ///
    /// Each line has the format `<name> <version>`. Lines with missing fields
    /// are skipped.
    pub(crate) fn parse_query_output(output: &str) -> Vec<(String, String)> {
        output
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?;
                let version = parts.next()?;
                if name.is_empty() || version.is_empty() {
                    None
                } else {
                    Some((name.to_string(), version.to_string()))
                }
            })
            .collect()
    }

    /// Parse `pacman -Si <package>` output for a single package.
    ///
    /// Output consists of `Field           : value` lines. Returns the value
    /// of the `Version` field, or `None` if not found.
    pub(crate) fn parse_si_output(output: &str) -> Option<String> {
        for line in output.lines() {
            if let Some((key, value)) = line.split_once(':')
                && key.trim() == "Version"
            {
                let v = value.trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
        None
    }

    /// Parse `pacman -Si pkg1 pkg2 ...` output for a batch query.
    ///
    /// Output contains multiple blocks separated by blank lines. Each block
    /// corresponds to one package. For each block, both the `Name` and
    /// `Version` fields are extracted.
    ///
    /// Returns a map from package name to version string.
    pub(crate) fn parse_si_output_batch(output: &str) -> HashMap<String, String> {
        let mut results = HashMap::new();

        // Split into blocks on blank lines.
        let blocks = output.split("\n\n");
        for block in blocks {
            let block = block.trim();
            if block.is_empty() {
                continue;
            }

            let mut name: Option<String> = None;
            let mut version: Option<String> = None;

            for line in block.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let key = key.trim();
                    let value = value.trim();
                    if !value.is_empty() {
                        match key {
                            "Name" => name = Some(value.to_string()),
                            "Version" => version = Some(value.to_string()),
                            _ => {}
                        }
                    }
                }
            }

            if let (Some(n), Some(v)) = (name, version) {
                results.insert(n, v);
            }
        }

        results
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier)
            .map_err(|e| rootcause::report!(PluginError::Configuration(e)))
    }
}

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    PacmanPlugin,
    PacmanConfig,
    "package_manager_pacman",
    {
        fn capabilities(&self) -> Vec<PluginCapability> {
            Self::CAPABILITIES.to_vec()
        }
        fn required_sudo_commands(
            &self,
        ) -> Vec<uptrakit_plugin_infrastructure_core::SudoCommandEntry> {
            vec![
                // `-Sy` matches the exact refresh call (no extra args).
                SudoCommandEntry::new("pacman", "Synchronise the Pacman package database")
                    .with_args_suffix(Cow::Borrowed("-Sy")),
                // `-S --noconfirm *` covers single and batch installs.
                SudoCommandEntry::new("pacman", "Install or upgrade a Pacman package")
                    .with_args_suffix(Cow::Borrowed("-S --noconfirm *")),
            ]
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
        fn as_package_index(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::PackageIndexPlugin> {
            Some(self)
        }
        fn as_update_executor(
            &self,
        ) -> Option<&dyn uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin> {
            Some(self)
        }
    }
);

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use uptrakit_plugin_infrastructure_core::testing::FixedOutputExecutor;
    use uptrakit_plugin_infrastructure_core::{HostCompatibility, LocalCommandExecutor};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    // ── validate_identifier ──────────────────────────────────────────────────

    #[test]
    fn validate_identifier_valid_simple() {
        assert!(validate_identifier("nginx").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dash() {
        assert!(validate_identifier("python-pip").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_plus() {
        assert!(validate_identifier("lib32-glibc").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dot() {
        assert!(validate_identifier("python3.11").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_at() {
        assert!(validate_identifier("lib32@x64").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_underscore() {
        assert!(validate_identifier("my_package").is_ok());
    }

    #[test]
    fn validate_identifier_valid_starts_with_digit() {
        assert!(validate_identifier("2ping").is_ok());
    }

    #[test]
    fn validate_identifier_valid_single_char() {
        assert!(validate_identifier("a").is_ok());
    }

    #[test]
    fn validate_identifier_valid_max_length() {
        let name = "a".repeat(128);
        assert!(validate_identifier(&name).is_ok());
    }

    #[test]
    fn validate_identifier_empty_fails() {
        let err = validate_identifier("").expect_err("should fail");
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_identifier_too_long_fails() {
        let name = "a".repeat(129);
        let err = validate_identifier(&name).expect_err("should fail");
        assert!(err.contains("128"));
    }

    #[test]
    fn validate_identifier_uppercase_fails() {
        assert!(validate_identifier("Nginx").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dash_fails() {
        assert!(validate_identifier("-foo").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dot_fails() {
        assert!(validate_identifier(".foo").is_err());
    }

    #[test]
    fn validate_identifier_path_traversal_fails() {
        assert!(validate_identifier("foo..bar").is_err());
    }

    #[test]
    fn validate_identifier_slash_fails() {
        assert!(validate_identifier("foo/bar").is_err());
    }

    #[test]
    fn validate_identifier_whitespace_fails() {
        assert!(validate_identifier("foo bar").is_err());
    }

    // ── validate_version ────────────────────────────────────────────────────

    #[test]
    fn validate_version_arch_standard() {
        assert!(validate_version("1.26.3-1").is_ok());
        assert!(validate_version("8.12.1-1").is_ok());
        assert!(validate_version("2:1.0-1").is_ok()); // epoch format
    }

    #[test]
    fn validate_version_with_tilde() {
        assert!(validate_version("1.0~rc1-1").is_ok());
    }

    #[test]
    fn validate_version_empty_fails() {
        let err = validate_version("").expect_err("should fail");
        assert!(err.contains("empty"));
    }

    #[test]
    fn validate_version_too_long_fails() {
        let long = "1".repeat(257);
        assert!(validate_version(&long).is_err());
    }

    #[test]
    fn validate_version_leading_dash_fails() {
        let err = validate_version("--noconfirm").expect_err("should fail");
        assert!(err.contains("flag"));
    }

    #[test]
    fn validate_version_space_fails() {
        assert!(validate_version("1.0 --noconfirm").is_err());
    }

    #[test]
    fn validate_version_max_length_ok() {
        let v = "1".repeat(256);
        assert!(validate_version(&v).is_ok());
    }

    // ── parse_query_output ───────────────────────────────────────────────────

    #[test]
    fn parse_query_output_normal() {
        let output = "nginx 1.26.3-1\npython 3.12.4-1\n";
        let result = PacmanPlugin::parse_query_output(output);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("nginx".to_string(), "1.26.3-1".to_string()));
        assert_eq!(result[1], ("python".to_string(), "3.12.4-1".to_string()));
    }

    #[test]
    fn parse_query_output_single_field_line_skipped() {
        let output = "nginx\npython 3.12.4-1\n";
        let result = PacmanPlugin::parse_query_output(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "python");
    }

    #[test]
    fn parse_query_output_empty_input() {
        let result = PacmanPlugin::parse_query_output("");
        assert!(result.is_empty());
    }

    // ── parse_si_output ──────────────────────────────────────────────────────

    #[test]
    fn parse_si_output_extracts_version() {
        let output = concat!(
            "Repository      : extra\n",
            "Name            : nginx\n",
            "Version         : 1.26.3-1\n",
            "Description     : Lightweight HTTP server\n",
        );
        let version = PacmanPlugin::parse_si_output(output);
        assert_eq!(version, Some("1.26.3-1".to_string()));
    }

    #[test]
    fn parse_si_output_epoch_version() {
        let output = concat!(
            "Repository      : core\n",
            "Name            : curl\n",
            "Version         : 2:8.12.1-1\n",
        );
        let version = PacmanPlugin::parse_si_output(output);
        assert_eq!(version, Some("2:8.12.1-1".to_string()));
    }

    #[test]
    fn parse_si_output_missing_version_returns_none() {
        let output = "Repository      : extra\nName            : nginx\n";
        assert!(PacmanPlugin::parse_si_output(output).is_none());
    }

    #[test]
    fn parse_si_output_empty_returns_none() {
        assert!(PacmanPlugin::parse_si_output("").is_none());
    }

    // ── parse_si_output_batch ────────────────────────────────────────────────

    #[test]
    fn parse_si_output_batch_groups_by_package() {
        let output = concat!(
            "Repository      : extra\n",
            "Name            : nginx\n",
            "Version         : 1.26.3-1\n",
            "Description     : Lightweight HTTP server\n",
            "\n",
            "Repository      : core\n",
            "Name            : curl\n",
            "Version         : 8.12.1-1\n",
            "Description     : Multi-protocol file transfer\n",
        );
        let result = PacmanPlugin::parse_si_output_batch(output);
        assert_eq!(result.len(), 2);
        assert_eq!(result["nginx"], "1.26.3-1");
        assert_eq!(result["curl"], "8.12.1-1");
    }

    #[test]
    fn parse_si_output_batch_single_package() {
        let output = concat!(
            "Repository      : extra\n",
            "Name            : git\n",
            "Version         : 2.47.2-1\n",
        );
        let result = PacmanPlugin::parse_si_output_batch(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result["git"], "2.47.2-1");
    }

    #[test]
    fn parse_si_output_batch_empty_returns_empty() {
        let result = PacmanPlugin::parse_si_output_batch("");
        assert!(result.is_empty());
    }

    // ── required_sudo_commands ───────────────────────────────────────────────

    #[tokio::test]
    async fn pacman_plugin_required_sudo_commands() {
        use uptrakit_plugin_infrastructure_core::PluginBase;
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let entries = plugin.required_sudo_commands();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.command == "pacman"));
        assert!(entries.iter().all(|e| !e.needs_setenv));
        assert_eq!(entries[0].args_suffix.as_deref(), Some("-Sy"));
        assert_eq!(entries[1].args_suffix.as_deref(), Some("-S --noconfirm *"));
    }

    // ── capabilities ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn pacman_plugin_capabilities() {
        use uptrakit_plugin_infrastructure_core::PluginBase;
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));

        assert_eq!(plugin.capabilities().len(), 3);
    }

    // ── empty identifier guards ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_releases_empty_identifier_fails() {
        use uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin;
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let result = plugin.fetch_releases("").await;
        assert!(result.is_err());
    }

    // ── detect_host_compatibility ────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let plugin = PacmanPlugin::new(PacmanConfig::default(), FixedOutputExecutor::failure(0))
            .await
            .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let plugin = PacmanPlugin::new(PacmanConfig::default(), FixedOutputExecutor::failure(1))
            .await
            .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "pacman not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }
}
