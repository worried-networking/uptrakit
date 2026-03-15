use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, BatchUpdateItem,
    BatchUpdateResult, DiscoveredSoftware, DiscoveryTarget, HostCompatibility, OutputStreamType,
    PluginCapability, PluginError, PluginRole, PluginType, ReleaseInfo, Result, SudoCommandEntry,
    UpdateOutputLine, UpstreamRelease, Version,
};

// Subtrait imports needed by tests (via `use super::*`) to resolve method calls.
#[cfg(test)]
use uptrakit_plugin_infrastructure_core::{
    DiscoveryPlugin, PluginBase, ReleaseFetcherPlugin, VersionDetectorPlugin,
};

use crate::config::{PacmanConfig, PacmanDiscoveryFilter};

/// Validate an Arch Linux Pacman package identifier.
///
/// Enforces Arch Linux PKGBUILD naming rules:
/// - Between 1 and 128 characters long.
/// - Must start with a lowercase letter or digit (`[a-z0-9]`).
/// - May only contain lowercase letters, digits, `@`, `.`, `_`, `+`, or `-`.
/// - Must not contain `..` (path traversal protection).
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value.len() > 128 {
        return Err("package_identifier must not exceed 128 characters".to_string());
    }

    // Must start with [a-z0-9].
    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "package_identifier must start with a lowercase letter or digit, found '{first}'"
        ));
    }

    // All characters must be in [a-z0-9@._+-].
    for ch in value.chars() {
        if !ch.is_ascii_lowercase()
            && !ch.is_ascii_digit()
            && !matches!(ch, '@' | '.' | '_' | '+' | '-')
        {
            return Err(format!(
                "package_identifier contains invalid character: '{ch}'"
            ));
        }
    }

    // No path traversal via '..'.
    if value.contains("..") {
        return Err("package_identifier must not contain '..'".to_string());
    }

    Ok(())
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
    config: PacmanConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl PacmanPlugin {
    /// Compile-time capabilities for the Pacman plugin.
    ///
    /// Note: No `PostUpdateHook` — Arch Linux does not use
    /// `/var/run/reboot-required`.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::RefreshPackageIndex,
        PluginCapability::DetectHostCompatibility,
    ];

    /// Create a new Pacman plugin with the given configuration.
    pub async fn new(config: PacmanConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Parse `pacman -Q` or `pacman -Qe` output.
    ///
    /// Each line has the format `<name> <version>`. Lines with missing fields
    /// are skipped.
    fn parse_query_output(output: &str) -> Vec<(String, String)> {
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
    fn parse_si_output(output: &str) -> Option<String> {
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
    fn parse_si_output_batch(output: &str) -> HashMap<String, String> {
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

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier).map_err(|e| report!(PluginError::Configuration(e)))
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
                SudoCommandEntry::new("pacman", "Package database sync requires root privileges")
                    .with_args_suffix(Cow::Borrowed("-Sy")),
                // `-S --noconfirm *` covers single and batch installs.
                SudoCommandEntry::new("pacman", "Package installation requires root privileges")
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

#[async_trait]
impl uptrakit_plugin_infrastructure_core::DiscoveryPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering Pacman-managed software");

        // Choose command based on effective filter.
        let args = match self.config.effective_filter() {
            PacmanDiscoveryFilter::Explicit => vec!["-Qe".to_string()],
            PacmanDiscoveryFilter::All => vec!["-Q".to_string()],
        };

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("pacman", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pacman query failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let all_packages = Self::parse_query_output(&cmd_output.output);

        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: PluginType::PackageManagerPacman,
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "Pacman".to_string(),
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
                    featured: false,
                    installed_display_version: None,
                }
            })
            .collect();

        tracing::debug!(count = packages.len(), "Pacman software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["pacman".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "pacman not found".to_string(),
            )),
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting Pacman installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pacman",
                ["-Q".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pacman -Q failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                // Output is "name version\n".
                let version = cmd_output.output.split_whitespace().nth(1).map(|v| {
                    tracing::debug!(version = %v, "Pacman installed version detected");
                    Version::new(v)
                });
                Ok(version)
            }
            // Exit code 1 means the package was not found.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in pacman database"
                );
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Detect installed versions for multiple packages using a single
    /// `pacman -Q` call.
    ///
    /// Runs:
    /// ```text
    /// pacman -Q pkg1 pkg2 pkg3
    /// ```
    ///
    /// The exit code is intentionally ignored: `pacman -Q` exits non-zero when
    /// any requested package is not installed, but packages that *are* installed
    /// still appear in stdout. Packages absent from stdout are treated as not
    /// installed (`None` with no error).
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

        let mut args = vec!["-Q".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting Pacman installed versions"
        );

        // Non-zero exit is expected when any package is unknown; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("pacman", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                // pacman completely failed (e.g., not found on PATH).
                let error_str = format!("pacman -Q failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        // Parse output into a map for O(1) lookup.
        let query_map: HashMap<String, String> =
            Self::parse_query_output(&stdout).into_iter().collect();

        let results = items
            .iter()
            .map(|item| {
                let installed_version = query_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(
            count = items.len(),
            "Pacman batch version detection complete"
        );
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching Pacman releases via pacman -Si");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pacman",
                ["-Si".to_string(), package_identifier.to_string()],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pacman -Si failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {}
            // Exit code 1 means the package was not found in any repository.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in any configured repository"
                );
                return Ok(vec![]);
            }
            code => bail!(PluginError::CommandFailed(code)),
        }

        let Some(version) = Self::parse_si_output(&cmd_output.output) else {
            return Ok(vec![]);
        };

        tracing::debug!(
            version = %version,
            "Pacman upstream version resolved"
        );
        Ok(vec![UpstreamRelease::new(
            Version::new(&version),
            version,
            false,
            "",
        )])
    }

    /// Fetch available releases for multiple packages using a single
    /// `pacman -Si` call.
    ///
    /// Runs:
    /// ```text
    /// pacman -Si pkg1 pkg2 pkg3
    /// ```
    ///
    /// Output blocks (one per package) are separated by blank lines. Only
    /// packages present in the output have releases; absent packages have
    /// empty releases. The exit code is intentionally ignored because
    /// `pacman -Si` exits non-zero when any package is not in the repos.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["-Si".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch fetching Pacman releases via pacman -Si"
        );

        // Non-zero exit is expected when any package is not in repos; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("pacman", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                let error_str = format!("pacman -Si failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchFetchResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        let parsed = Self::parse_si_output_batch(&stdout);

        let results = items
            .iter()
            .map(|item| {
                let Some(version) = parsed.get(&item.package_identifier) else {
                    // Package not found in any configured repository.
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                BatchFetchResult::found(
                    item.package_identifier.clone(),
                    vec![UpstreamRelease::new(
                        Version::new(version),
                        version.clone(),
                        false,
                        "",
                    )],
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "Pacman batch fetch complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Pacman package database");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("pacman", ["-Sy".to_string()]).privileged())
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pacman -Sy failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("Pacman package database refreshed");
        Ok(())
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutorPlugin for PacmanPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version).map_err(|e| report!(PluginError::Configuration(e)))?;

        // Pacman always installs the latest version from the repository;
        // version pinning is not supported. The `to_version` argument is
        // validated for safety but not passed to the command.
        let args = vec![
            "-S".to_string(),
            "--noconfirm".to_string(),
            package_identifier.to_string(),
        ];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running pacman -S --noconfirm"
        );

        let display_args = std::iter::once("pacman")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        send_output(
            output_tx,
            &format!("Running: {display_args}"),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("pacman", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "pacman -S failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Execute batch updates by installing all targeted packages in a single
    /// `pacman -S --noconfirm` invocation.
    ///
    /// All packages are installed or none — pacman treats the batch as a
    /// single transaction.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all package identifiers and versions up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
            validate_version(&item.to_version)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["-S".to_string(), "--noconfirm".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let display_args = std::iter::once("pacman")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages: {}\nRunning: {display_args}",
                items.len(),
                pkg_list.join(", ")
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_args}\n");

        tracing::debug!(
            count = items.len(),
            packages = ?pkg_list,
            "running pacman batch install"
        );

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("pacman", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        output.push_str(&cmd_output.output);

        let success = cmd_output.exit_code == 0;
        let results = items
            .iter()
            .map(|item| {
                BatchUpdateResult::new(item.package_identifier.clone(), success, output.clone())
            })
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{CommandOutput, LocalCommandExecutor};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    /// Mock executor that routes `execute_quiet` output by the command program
    /// name.
    ///
    /// Matches the program name from `CommandSpec::mode` (Exec variant only).
    /// Falls back to an empty-output success for Shell-mode or unrecognised
    /// programs.
    struct RoutedOutputExecutor {
        /// `(program_name, output_to_return)` entries checked in order.
        routes: Vec<(&'static str, String)>,
    }

    impl RoutedOutputExecutor {
        /// Create an executor from a list of `(program, output)` pairs.
        fn with_routes(routes: Vec<(&'static str, &'static str)>) -> Arc<dyn CommandExecutor> {
            Arc::new(Self {
                routes: routes
                    .into_iter()
                    .map(|(p, o)| (p, o.to_string()))
                    .collect(),
            })
        }

        fn output_for(&self, spec: &CommandSpec) -> String {
            use uptrakit_plugin_infrastructure_core::CommandMode;
            if let CommandMode::Exec { program, .. } = &spec.mode {
                for (name, out) in &self.routes {
                    if program == *name {
                        return out.clone();
                    }
                }
            }
            String::new()
        }
    }

    #[async_trait]
    impl CommandExecutor for RoutedOutputExecutor {
        async fn execute(
            &self,
            spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output_for(spec),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.output_for(spec),
                exit_code: 0,
            })
        }
    }

    /// Mock executor that returns a configurable exit code for `execute_quiet`.
    struct FixedExitCodeExecutor {
        exit_code: i32,
    }

    impl FixedExitCodeExecutor {
        fn with_exit_code(exit_code: i32) -> Arc<dyn CommandExecutor> {
            Arc::new(Self { exit_code })
        }
    }

    #[async_trait]
    impl CommandExecutor for FixedExitCodeExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: String::new(),
                exit_code: self.exit_code,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            if self.exit_code == 0 {
                Ok(CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                })
            } else {
                use rootcause::prelude::*;
                bail!(uptrakit_command::CommandError::CommandFailed(
                    self.exit_code
                ))
            }
        }
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
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
        assert!(!plugin.has_capability(PluginCapability::PostUpdateHook));
        assert_eq!(plugin.capabilities().len(), 3);
    }

    // ── empty identifier guards ──────────────────────────────────────────────

    #[tokio::test]
    async fn detect_installed_version_empty_identifier_fails() {
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn fetch_releases_empty_identifier_fails() {
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let result = plugin.fetch_releases("").await;
        assert!(result.is_err());
    }

    // ── detect_host_compatibility ────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        let plugin = PacmanPlugin::new(
            PacmanConfig::default(),
            FixedExitCodeExecutor::with_exit_code(0),
        )
        .await
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        let plugin = PacmanPlugin::new(
            PacmanConfig::default(),
            FixedExitCodeExecutor::with_exit_code(1),
        )
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

    // ── discover_software target emission ────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets() {
        // Targets are always emitted regardless of filter.
        let executor = RoutedOutputExecutor::with_routes(vec![("pacman", "nginx 1.26.3-1\n")]);
        let plugin = PacmanPlugin::new(PacmanConfig::default(), executor)
            .await
            .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);

        let target = &discoveries[0].targets[0];
        assert_eq!(target.plugin_type, PluginType::PackageManagerPacman);
        assert_eq!(target.plugin_config_name, "Pacman");
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_default_config_discovers_all_packages() {
        let executor = RoutedOutputExecutor::with_routes(vec![(
            "pacman",
            "nginx 1.26.3-1\npython 3.12.4-1\n",
        )]);
        let plugin = PacmanPlugin::new(PacmanConfig::default(), executor)
            .await
            .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 2, "all packages must be discovered");
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_all_filter() {
        let executor = RoutedOutputExecutor::with_routes(vec![("pacman", "nginx 1.26.3-1\n")]);
        let plugin = PacmanPlugin::new(
            PacmanConfig {
                discovery_filter: PacmanDiscoveryFilter::All,
            },
            executor,
        )
        .await
        .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "explicit All filter must still emit targets"
        );
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_filter() {
        let executor = RoutedOutputExecutor::with_routes(vec![("pacman", "nginx 1.26.3-1\n")]);
        let plugin = PacmanPlugin::new(
            PacmanConfig {
                discovery_filter: PacmanDiscoveryFilter::Explicit,
            },
            executor,
        )
        .await
        .expect("create plugin");

        let discoveries = plugin.discover_software().await.expect("discover");
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].package_identifier, "nginx");
        assert_eq!(
            discoveries[0].targets.len(),
            1,
            "explicit filter must still emit targets"
        );
    }

    // ── batch_detect_installed_version ───────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_version_found_packages() {
        let executor = RoutedOutputExecutor::with_routes(vec![(
            "pacman",
            "nginx 1.26.3-1\npython 3.12.4-1\n",
        )]);
        let plugin = PacmanPlugin::new(PacmanConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("python".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 2);
        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(nginx.installed_version, Some(Version::new("1.26.3-1")));
        assert!(nginx.error.is_none());

        let python = results
            .iter()
            .find(|r| r.package_identifier == "python")
            .unwrap();
        assert_eq!(python.installed_version, Some(Version::new("3.12.4-1")));
        assert!(python.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_package_not_in_output_is_not_installed() {
        let executor = RoutedOutputExecutor::with_routes(vec![("pacman", "nginx 1.26.3-1\n")]);
        let plugin = PacmanPlugin::new(PacmanConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("curl".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 2);
        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.installed_version.is_none());
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_detect_installed_version_empty_items_returns_empty() {
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin
            .batch_detect_installed_version(&[])
            .await
            .expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_invalid_identifier_fails() {
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create");
        let items = vec![BatchDetectItem::new("INVALID_UPPERCASE".to_string())];
        let result = plugin.batch_detect_installed_version(&items).await;
        assert!(result.is_err());
    }

    // ── batch_fetch_releases ─────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_releases_mixed_packages() {
        let executor = RoutedOutputExecutor::with_routes(vec![(
            "pacman",
            concat!(
                "Repository      : extra\n",
                "Name            : nginx\n",
                "Version         : 1.26.3-1\n",
                "\n",
                "Repository      : core\n",
                "Name            : git\n",
                "Version         : 2.47.2-1\n",
            ),
        )]);
        let plugin = PacmanPlugin::new(PacmanConfig::default(), executor)
            .await
            .expect("create");

        let items = vec![
            BatchFetchItem::new("nginx".to_string()),
            BatchFetchItem::new("git".to_string()),
            BatchFetchItem::new("curl".to_string()),
        ];
        let results = plugin.batch_fetch_releases(&items).await.expect("ok");

        assert_eq!(results.len(), 3);

        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert_eq!(nginx.releases.len(), 1);
        assert_eq!(nginx.releases[0].tag, "1.26.3-1");
        assert!(nginx.error.is_none());

        let git = results
            .iter()
            .find(|r| r.package_identifier == "git")
            .unwrap();
        assert_eq!(git.releases.len(), 1);
        assert_eq!(git.releases[0].tag, "2.47.2-1");
        assert!(git.error.is_none());

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(curl.releases.is_empty(), "absent package has no releases");
        assert!(curl.error.is_none(), "absent package is not an error");
    }

    #[tokio::test]
    async fn batch_fetch_releases_empty_items_returns_empty() {
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin.batch_fetch_releases(&[]).await.expect("ok");
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_fetch_releases_invalid_identifier_fails() {
        let plugin = PacmanPlugin::new(PacmanConfig::default(), test_executor())
            .await
            .expect("create");
        let items = vec![BatchFetchItem::new("INVALID".to_string())];
        let result = plugin.batch_fetch_releases(&items).await;
        assert!(result.is_err());
    }
}
