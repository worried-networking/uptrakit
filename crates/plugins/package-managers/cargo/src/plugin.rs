use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::StreamExt as _;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, DiscoveredSoftware,
    DiscoveryTarget, HostCompatibility, OutputStreamType, PluginCapability, PluginError,
    PluginRole, PluginType, ReleaseInfo, Result, UpdateOutputLine, UpstreamRelease, Version,
};
use uptrakit_shared_types::ssrf::{SsrfSafeResolver, webpki_client_config};

use crate::config::CargoConfig;
use crate::error::CargoError;

/// Validate a Cargo crate package identifier.
///
/// Enforces Cargo crate naming rules used by `cargo install`:
/// - Between 1 and 64 characters long.
/// - Must start with an ASCII letter (`A-Za-z`) or underscore (`_`).
/// - Remaining characters: `[A-Za-z0-9_-]` only.
/// - Must not contain `..` or path separators (`/`, `\`).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value.len() > 64 {
        return Err("package_identifier must not exceed 64 characters".to_string());
    }

    // Must start with ASCII letter or underscore.
    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(format!(
            "package_identifier must start with a letter or underscore, found '{first}'"
        ));
    }

    // All characters must be in [A-Za-z0-9_-].
    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-' {
            return Err(format!(
                "package_identifier contains invalid character: '{ch}' \
                 (only ASCII letters, digits, underscores, and hyphens are allowed)"
            ));
        }
    }

    // Belt-and-suspenders: reject path traversal and separators.
    // These are already rejected by the character set above, but we keep
    // the checks explicit so the error message is unambiguous.
    if value.contains("..") {
        return Err("package_identifier must not contain '..'".to_string());
    }
    if value.contains('/') || value.contains('\\') {
        return Err("package_identifier must not contain path separators".to_string());
    }

    Ok(())
}

/// Returns `true` if the given version string is a semver pre-release.
///
/// A version is considered pre-release when it contains `-` after the numeric
/// parts (e.g. `1.0.0-alpha.1`, `2.0.0-beta`, `0.9.0-rc.1`).
fn is_prerelease_version(version: &str) -> bool {
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

/// Compute the sparse index URL path fragment for a crate name.
///
/// Implements the standard Cargo sparse index path computation:
/// - 1 char:  `1/{name}`
/// - 2 chars: `2/{name}`
/// - 3 chars: `3/{first}/{name}`
/// - 4+ chars: `{first_two}/{next_two}/{name}`
///
/// The name is lowercased, as required by the index format.
fn sparse_index_url(registry_base: &str, crate_name: &str) -> String {
    let name = crate_name.to_lowercase();
    let prefix = match name.len() {
        1 => "1".to_string(),
        2 => "2".to_string(),
        3 => format!("3/{}", &name[..1]),
        _ => format!("{}/{}", &name[..2], &name[2..4]),
    };
    format!(
        "{}/{}/{}",
        registry_base.trim_end_matches('/'),
        prefix,
        name
    )
}

/// Fetch upstream releases for a single crate from the sparse registry index.
///
/// Makes a single HTTP `GET` request to the sparse index URL, parses the
/// newline-delimited JSON response with `tame_index::IndexKrate::from_slice`,
/// and returns filtered [`UpstreamRelease`] entries sorted in **descending
/// semver order** (newest first), so callers can simply use `.find()` to
/// obtain the latest release.
async fn fetch_crate_releases(
    client: &reqwest::Client,
    registry_base: &str,
    include_prereleases: bool,
    crate_name: &str,
) -> crate::error::Result<Vec<UpstreamRelease>> {
    let url = sparse_index_url(registry_base, crate_name);
    tracing::debug!(crate_name, %url, "fetching crate releases from sparse index");

    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "text/plain")
        .send()
        .await
        .map_err(|e| report!(CargoError::Request(e.to_string())))?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        tracing::debug!(crate_name, "crate not found in registry index");
        return Ok(vec![]);
    }

    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        bail!(CargoError::ApiError { status, message });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| report!(CargoError::Request(e.to_string())))?;

    let krate = tame_index::IndexKrate::from_slice(&bytes).map_err(|e| {
        report!(CargoError::Request(format!(
            "failed to parse sparse index response for '{crate_name}': {e}"
        )))
    })?;

    let mut releases: Vec<UpstreamRelease> = krate
        .versions
        .iter()
        .filter(|v| {
            if v.yanked {
                return false;
            }
            let prerelease = is_prerelease_version(v.version.as_str());
            !prerelease || include_prereleases
        })
        .map(|v| {
            let version_str = v.version.as_str().to_string();
            let release_url = format!("https://crates.io/crates/{crate_name}/{version_str}");
            let is_pre = is_prerelease_version(&version_str);
            UpstreamRelease::new(Version::new(&version_str), version_str, is_pre, release_url)
        })
        .collect();

    // The sparse index stores versions in chronological (oldest-first) order.
    // Sort descending so the scheduler's `.find(|r| !r.is_prerelease)` picks
    // the newest stable release instead of the oldest.
    releases.sort_by(|a, b| b.version.cmp(&a.version));

    tracing::debug!(
        crate_name,
        count = releases.len(),
        "fetched crate releases from sparse index"
    );
    Ok(releases)
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
    config: CargoConfig,
    executor: Arc<dyn CommandExecutor>,
    client: reqwest::Client,
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
        let ssrf_resolver: Arc<dyn reqwest::dns::Resolve> = if config.registry_url.is_some() {
            Arc::new(SsrfSafeResolver::permissive())
        } else {
            Arc::new(SsrfSafeResolver::new())
        };

        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-package-manager-cargo/",
                env!("CARGO_PKG_VERSION")
            ))
            .use_preconfigured_tls(webpki_client_config())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .dns_resolver(ssrf_resolver)
            .build()
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        Ok(Self {
            config,
            executor,
            client,
        })
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
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

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for CargoPlugin {
    /// Detect the installed version of a single crate.
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting cargo-installed version");

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
        let version = installed.get(package_identifier).map(Version::new);

        if let Some(ref v) = version {
            tracing::debug!(version = %v, "cargo installed version detected");
        } else {
            tracing::debug!(package = %package_identifier, "crate not found in cargo install list");
        }

        Ok(version)
    }

    /// Detect installed versions for multiple crates using a single `cargo install --list` call.
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting cargo-installed versions"
        );

        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "cargo",
                ["install".to_string(), "--list".to_string()],
            ))
            .await
        {
            Ok(o) => {
                if o.exit_code != 0 {
                    let error_str =
                        format!("cargo install --list failed with exit code {}", o.exit_code);
                    return Ok(items
                        .iter()
                        .map(|item| {
                            BatchDetectResult::error(
                                item.package_identifier.clone(),
                                error_str.clone(),
                            )
                        })
                        .collect());
                }
                o.output
            }
            Err(e) => {
                let error_str = format!("cargo install --list failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        let installed = parse_cargo_install_list(&stdout);

        Ok(items
            .iter()
            .map(|item| {
                let installed_version = installed.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect())
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for CargoPlugin {
    /// Fetch available releases for a single crate from the sparse registry index.
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;

        fetch_crate_releases(
            &self.client,
            self.config.effective_registry_url(),
            self.config.include_prereleases,
            package_identifier,
        )
        .await
        .map_err(|e| report!(PluginError::PluginInternal(e.to_string())))
    }

    /// Fetch releases for multiple crates in parallel, bounded to 10 concurrent requests.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        tracing::debug!(count = items.len(), "batch fetching cargo crate releases");

        // Clone cheap handles before moving into stream closures.
        let client = self.client.clone();
        let registry_base = self.config.effective_registry_url().to_string();
        let include_prereleases = self.config.include_prereleases;

        // Pre-collect owned identifiers so each future can own its data (`'static`).
        let ids: Vec<String> = items.iter().map(|i| i.package_identifier.clone()).collect();

        let results = futures_util::stream::iter(ids)
            .map(|id| {
                let client = client.clone();
                let registry_base = registry_base.clone();
                async move {
                    match fetch_crate_releases(&client, &registry_base, include_prereleases, &id)
                        .await
                    {
                        Ok(releases) => BatchFetchResult::found(id, releases),
                        Err(e) => BatchFetchResult::error(id, e.to_string()),
                    }
                }
            })
            .buffer_unordered(10)
            .collect::<Vec<_>>()
            .await;

        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for CargoPlugin {
    /// Execute a `cargo install` update for a single crate.
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;

        let mut args = vec![
            "install".to_string(),
            package_identifier.to_string(),
            "--version".to_string(),
            to_version.to_string(),
        ];
        if self.config.use_locked {
            args.push("--locked".to_string());
        }

        let display_cmd = if self.config.use_locked {
            format!(
                "cargo install {} --version {} --locked",
                package_identifier, to_version
            )
        } else {
            format!(
                "cargo install {} --version {}",
                package_identifier, to_version
            )
        };
        tracing::debug!(
            package = %package_identifier,
            to_version = %to_version,
            "running cargo install"
        );

        send_output(
            output_tx,
            &format!("Updating {package_identifier} to {to_version}\nRunning: {display_cmd}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_cmd}\n");

        // No `.privileged()` -- cargo install does not require sudo.
        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("cargo", args), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "cargo install failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::CommandOutput;

    /// Mock executor that returns a fixed output and exit code for all commands.
    struct FixedOutputExecutor {
        output: String,
        exit_code: i32,
    }

    #[async_trait]
    impl CommandExecutor for FixedOutputExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output.clone(),
                exit_code: self.exit_code,
            })
        }
    }

    fn make_executor(stdout: &str, exit_code: i32) -> Arc<dyn CommandExecutor> {
        Arc::new(FixedOutputExecutor {
            output: stdout.to_string(),
            exit_code,
        })
    }

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

    // ── sparse_index_url ──────────────────────────────────────────────────────

    #[test]
    fn sparse_index_url_one_char() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "a"),
            "https://index.crates.io/1/a"
        );
    }

    #[test]
    fn sparse_index_url_two_chars() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "ab"),
            "https://index.crates.io/2/ab"
        );
    }

    #[test]
    fn sparse_index_url_three_chars() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "bat"),
            "https://index.crates.io/3/b/bat"
        );
    }

    #[test]
    fn sparse_index_url_four_plus_chars() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "ripgrep"),
            "https://index.crates.io/ri/pg/ripgrep"
        );
        assert_eq!(
            sparse_index_url("https://index.crates.io", "cargo-nextest"),
            "https://index.crates.io/ca/rg/cargo-nextest"
        );
    }

    #[test]
    fn sparse_index_url_uppercase_lowercased() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "MyTool"),
            "https://index.crates.io/my/to/mytool"
        );
    }

    #[test]
    fn sparse_index_url_trailing_slash_stripped() {
        assert_eq!(
            sparse_index_url("https://index.crates.io/", "bat"),
            "https://index.crates.io/3/b/bat"
        );
    }

    // ── detect_host_compatibility ─────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_cargo_found() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor("", 0))
            .await
            .unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_cargo_missing() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        // Exit code != 0 from `which cargo` -> incompatible.
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor("", 1))
            .await
            .unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert!(matches!(result, HostCompatibility::Incompatible(_)));
    }

    // ── required_sudo_commands ────────────────────────────────────────────────

    #[tokio::test]
    async fn required_sudo_commands_empty() {
        use uptrakit_plugin_infrastructure_core::PluginBase;
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor("", 0))
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

    // ── detect_installed_version ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_found() {
        use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor(output, 0))
            .await
            .unwrap();

        let result = plugin.detect_installed_version("bat").await.unwrap();
        assert_eq!(result, Some(Version::new("0.24.0")));
    }

    #[tokio::test]
    async fn detect_installed_version_not_found() {
        use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
        let output = "bat v0.24.0:\n    bat\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor(output, 0))
            .await
            .unwrap();

        let result = plugin.detect_installed_version("ripgrep").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_installed_version_invalid_identifier_fails() {
        use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor("", 0))
            .await
            .unwrap();

        assert!(plugin.detect_installed_version("1invalid").await.is_err());
        assert!(plugin.detect_installed_version("owner/repo").await.is_err());
    }

    // ── batch_detect_installed_version ────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_version_basic() {
        use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor(output, 0))
            .await
            .unwrap();

        let items = vec![
            BatchDetectItem::new("bat".to_string()),
            BatchDetectItem::new("ripgrep".to_string()),
            BatchDetectItem::new("notinstalled".to_string()),
        ];

        let results = plugin.batch_detect_installed_version(&items).await.unwrap();
        assert_eq!(results.len(), 3);

        let bat = results
            .iter()
            .find(|r| r.package_identifier == "bat")
            .unwrap();
        assert_eq!(bat.installed_version, Some(Version::new("0.24.0")));

        let rg = results
            .iter()
            .find(|r| r.package_identifier == "ripgrep")
            .unwrap();
        assert_eq!(rg.installed_version, Some(Version::new("14.1.1")));

        let missing = results
            .iter()
            .find(|r| r.package_identifier == "notinstalled")
            .unwrap();
        assert!(missing.installed_version.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_empty_returns_empty() {
        use uptrakit_plugin_infrastructure_core::VersionDetectorPlugin;
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor("", 0))
            .await
            .unwrap();

        let results = plugin.batch_detect_installed_version(&[]).await.unwrap();
        assert!(results.is_empty());
    }

    // ── discover_software ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_always_emits_targets() {
        use uptrakit_plugin_infrastructure_core::DiscoveryPlugin;
        let output = "bat v0.24.0:\n    bat\nripgrep v14.1.1:\n    rg\n";
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor(output, 0))
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
            make_executor(output, 0),
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
        let plugin = CargoPlugin::new(CargoConfig::default(), make_executor("", 0))
            .await
            .unwrap();

        let discovered = plugin.discover_software().await.unwrap();
        assert!(discovered.is_empty());
    }

    // ── fetch_releases sort order ─────────────────────────────────────────────

    /// Verify that `fetch_crate_releases` returns versions in descending semver
    /// order (newest first), matching the contract expected by the scheduler's
    /// `.find(|r| !r.is_prerelease)` logic.
    #[test]
    fn fetch_releases_sorted_newest_first() {
        // Simulate the chronological (oldest-first) order from the sparse index.
        let mut releases: Vec<UpstreamRelease> = [
            UpstreamRelease::new(
                Version::new("0.1.0"),
                "0.1.0".to_string(),
                false,
                "https://crates.io/crates/example/0.1.0".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("0.9.0"),
                "0.9.0".to_string(),
                false,
                "https://crates.io/crates/example/0.9.0".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("1.0.0-alpha"),
                "1.0.0-alpha".to_string(),
                true,
                "https://crates.io/crates/example/1.0.0-alpha".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("1.0.0"),
                "1.0.0".to_string(),
                false,
                "https://crates.io/crates/example/1.0.0".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("1.2.3"),
                "1.2.3".to_string(),
                false,
                "https://crates.io/crates/example/1.2.3".to_string(),
            ),
        ]
        .into();

        // Apply the same sort used in `fetch_crate_releases`.
        releases.sort_by(|a, b| b.version.cmp(&a.version));

        // Newest must be first.
        assert_eq!(releases[0].version, Version::new("1.2.3"));
        // Oldest must be last.
        assert_eq!(releases[releases.len() - 1].version, Version::new("0.1.0"));

        // The scheduler's "find latest stable" logic must now pick 1.2.3, not 0.1.0.
        let latest_stable = releases.iter().find(|r| !r.is_prerelease);
        assert_eq!(
            latest_stable.map(|r| r.version.clone()),
            Some(Version::new("1.2.3")),
        );
    }
}
