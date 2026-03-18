use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, BatchUpdateItem,
    BatchUpdateResult, ConfigModel, ConfigTestKind, DiscoveredSoftware, DiscoveryTarget,
    HostCompatibility, HostRequirements, HostRuntime, OutputStreamType, PluginError, PluginFamily,
    PluginRole, ReleaseInfo, Result, SudoCommandEntry, UpdateCategory, UpdateOutputSender,
    UpstreamRelease, Version, declare_plugin, execute_and_capture, plugin_ids,
    require_posix_executor,
};
// Subtrait imports -- needed so `use super::*` in tests brings these methods into scope.
#[cfg(test)]
use uptrakit_plugin_infrastructure_core::{Discoverer, ReleaseFetcher, VersionDetector};

use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::{DnfConfig, DnfDiscoveryFilter};

const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 1,
    max_len: 128,
    first_char_valid: |c| c.is_ascii_alphanumeric(),
    char_valid: |c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'),
    reject_double_dot: true,
};

/// Validate an RPM package identifier.
///
/// Enforces RPM package naming rules:
/// - Between 1 and 128 characters long.
/// - Must start with a letter or digit (`[a-zA-Z0-9]`).
/// - May only contain letters, digits, `.`, `_`, and `-`.
/// - Must not contain `..` (path traversal protection).
/// - Must not start with `-`.
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    IDENTIFIER_RULES.validate(value)
}

/// Validate an RPM version string before it is interpolated into install commands.
///
/// Allows RPM version characters (`[a-zA-Z0-9.+~:_-]`). Rejects:
/// - Empty strings
/// - Strings starting with `-` (could be interpreted as a command-line flag by dnf)
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
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '+' | '~' | ':' | '_' | '-') {
            return Err(format!("version contains invalid character: '{ch}'"));
        }
    }
    Ok(())
}

/// Plugin for DNF (Fedora/RHEL/Rocky/AlmaLinux package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for RPM packages managed by `dnf`.
///
/// The `package_identifier` in `SoftwareItem` is the RPM package name
/// (e.g., `nginx`, `python3`, `httpd`).
pub struct DnfPlugin {
    config: DnfConfig,
    executor: Arc<dyn CommandExecutor>,
}

/// Parsed result from a single `dnf check-update` output line.
struct CheckUpdateEntry {
    version: String,
    repo: String,
}

impl DnfPlugin {
    /// Create a new DNF plugin with the given configuration and host runtime.
    pub fn new(
        config: DnfConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = require_posix_executor(runtime.as_ref()).map_err(|e| format!("{e}"))?;
        Ok(Self { config, executor })
    }

    /// Sudo commands required by this plugin.
    fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        vec![
            // Restrict to `dnf makecache` only (with optional flags such as `-q`).
            SudoCommandEntry::new("dnf", "Refresh the DNF package cache")
                .with_args_suffix(Cow::Borrowed("makecache *")),
            // Restrict to `dnf install -y` only; covers single and batch installs.
            SudoCommandEntry::new("dnf", "Install or upgrade a DNF package")
                .with_args_suffix(Cow::Borrowed("install -y *")),
        ]
    }

    /// Parse `rpm -qa --queryformat '%{NAME}\t%{VERSION}-%{RELEASE}\n'` output.
    ///
    /// Each line is a tab-separated `name\tversion-release` pair. Lines with an
    /// empty version are skipped.
    fn parse_rpm_output(output: &str) -> Vec<(String, String)> {
        output
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(2, '\t');
                let name = parts.next()?.trim();
                let version = parts.next()?.trim();
                if name.is_empty() || version.is_empty() {
                    None
                } else {
                    Some((name.to_string(), version.to_string()))
                }
            })
            .collect()
    }

    /// Parse `dnf check-update --quiet <package>` output for a single package.
    ///
    /// Output format when an update is available:
    /// ```text
    /// nginx.x86_64    1.26.0-1.fc40    updates
    /// ```
    ///
    /// - Exit code 0: no updates available (empty output).
    /// - Exit code 100: updates available (output contains lines).
    /// - Exit code 1: fatal error.
    ///
    /// Returns `None` when no update is available (up to date).
    fn parse_check_update_output(output: &str) -> Option<CheckUpdateEntry> {
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut cols = line.split_whitespace();
            // Column 1: name.arch -- must contain a '.' to be a valid package line.
            // Informational lines (e.g. "Last metadata expiration check: ...") do not
            // have an arch-suffixed first token and are skipped here.
            let Some(name_arch) = cols.next() else {
                continue;
            };
            if !name_arch.contains('.') {
                continue;
            }
            // Column 2: version-release
            let Some(version) = cols.next() else {
                continue;
            };
            // Column 3: repository
            let repo = cols.next().unwrap_or("").to_string();

            return Some(CheckUpdateEntry {
                version: version.to_string(),
                repo,
            });
        }
        None
    }

    /// Parse `dnf check-update --quiet <pkg1> <pkg2> ...` output for a batch query.
    ///
    /// Output lines are of the form:
    /// ```text
    /// nginx.x86_64    1.26.0-1.fc40    updates
    /// curl.x86_64     8.0.1-1.fc40     updates
    /// ```
    ///
    /// Groups lines by package name (extracted from `name.arch`). Only the
    /// *first* line per package is kept.
    fn parse_check_update_output_batch(output: &str) -> HashMap<String, CheckUpdateEntry> {
        let mut results: HashMap<String, CheckUpdateEntry> = HashMap::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut cols = line.split_whitespace();
            // Column 1: name.arch -- must contain '.' to be a valid package line.
            let Some(name_arch) = cols.next() else {
                continue;
            };
            if !name_arch.contains('.') {
                continue;
            }
            let Some(version) = cols.next() else {
                continue;
            };
            let repo = cols.next().unwrap_or("").to_string();

            // Extract package name: everything before the last '.'.
            let pkg_name = name_arch
                .rfind('.')
                .map(|i| name_arch[..i].to_string())
                .unwrap_or_else(|| name_arch.to_string());

            results.entry(pkg_name).or_insert_with(|| CheckUpdateEntry {
                version: version.to_string(),
                repo,
            });
        }
        results
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier).map_err(|e| report!(PluginError::Configuration(e)))
    }
}

// ── declare_plugin! ───────────────────────────────────────────────────────

declare_plugin!(DnfPlugin, DnfConfig, "package_manager_dnf", {
    display_name: "DNF Package Manager",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    roles: [
        Discoverer,
        VersionDetector,
        ReleaseFetcher,
        PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },
        UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED },
    ],
    sudo: DnfPlugin::required_sudo_commands,
});

// ── Role trait implementations ────────────────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for DnfPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering DNF-managed software");

        // Step 1: Query all installed packages from rpm.
        let rpm_stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec(
                "rpm",
                [
                    "-qa".to_string(),
                    "--queryformat".to_string(),
                    "%{NAME}\\t%{VERSION}-%{RELEASE}\\n".to_string(),
                ],
            ),
            "rpm -qa",
        )
        .await?;

        let all_packages = Self::parse_rpm_output(&rpm_stdout);

        // Step 2: For the UserInstalled filter, build a set of user-installed packages.
        let user_installed_set: Option<HashSet<String>> = match self.config.effective_filter() {
            DnfDiscoveryFilter::UserInstalled => {
                let repoquery_stdout = execute_and_capture(
                    self.executor.as_ref(),
                    CommandSpec::exec(
                        "dnf",
                        [
                            "repoquery".to_string(),
                            "--userinstalled".to_string(),
                            "--queryformat".to_string(),
                            "%{name}".to_string(),
                        ],
                    ),
                    "dnf repoquery --userinstalled",
                )
                .await?;

                let set: HashSet<String> = repoquery_stdout
                    .lines()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty())
                    .collect();
                Some(set)
            }
            DnfDiscoveryFilter::All => None,
        };

        // Step 3: Filter by user-installed set (if applicable) and build results.
        let packages: Vec<DiscoveredSoftware> = all_packages
            .into_iter()
            .filter(|(name, _)| {
                user_installed_set
                    .as_ref()
                    .is_none_or(|set| set.contains(name.as_str()))
            })
            .map(|(name, version)| {
                let targets = vec![DiscoveryTarget {
                    plugin_type: plugin_ids::PACKAGE_MANAGER_DNF.clone(),
                    plugin_config: serde_json::json!({}),
                    plugin_config_name: "DNF".to_string(),
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

        tracing::debug!(count = packages.len(), "DNF software discovery complete");
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["dnf".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible("dnf not found".to_string())),
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for DnfPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting DNF installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "rpm",
                [
                    "-q".to_string(),
                    "--queryformat".to_string(),
                    "%{VERSION}-%{RELEASE}".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| report!(PluginError::PluginInternal(format!("rpm -q failed: {e}"))))?;

        match cmd_output.exit_code {
            0 => {
                let version = cmd_output.output.trim().to_string();
                if version.is_empty() {
                    return Ok(None);
                }
                tracing::debug!(version = %version, "DNF installed version detected");
                Ok(Some(Version::new(&version)))
            }
            // Exit code 1 means the package was not found.
            1 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in RPM database"
                );
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Detect installed versions for multiple packages using a single `rpm -qa` call.
    ///
    /// Runs:
    /// ```text
    /// rpm -qa pkg1 pkg2 pkg3 --queryformat '%{NAME}\t%{VERSION}-%{RELEASE}\n'
    /// ```
    ///
    /// The exit code may be non-zero when any package is not found; packages that
    /// *are* found still appear in stdout. Packages absent from stdout are treated
    /// as not installed (`None` with no error).
    #[tracing::instrument(skip_all)]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args: Vec<String> = vec![
            "-qa".to_string(),
            "--queryformat".to_string(),
            "%{NAME}\\t%{VERSION}-%{RELEASE}\\n".to_string(),
        ];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting DNF installed versions"
        );

        // Non-zero exit is expected when any package is unknown; ignore it.
        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec("rpm", args))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                let error_str = format!("rpm -qa failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        let rpm_map: HashMap<String, String> =
            Self::parse_rpm_output(&stdout).into_iter().collect();

        let results = items
            .iter()
            .map(|item| {
                let installed_version = rpm_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(count = items.len(), "DNF batch version detection complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for DnfPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching DNF releases via dnf check-update");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "dnf",
                [
                    "check-update".to_string(),
                    "--quiet".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "dnf check-update failed: {e}"
                )))
            })?;

        // Exit code 0 = up to date (no updates), 100 = update available, 1 = error.
        match cmd_output.exit_code {
            0 => {
                // Package is up to date -- no upstream releases available.
                Ok(vec![])
            }
            100 => {
                let Some(entry) = Self::parse_check_update_output(&cmd_output.output) else {
                    return Ok(vec![]);
                };

                let category = if entry.repo.to_ascii_lowercase().contains("security") {
                    Some(UpdateCategory::Security)
                } else {
                    None
                };

                tracing::debug!(
                    version = %entry.version,
                    ?category,
                    repo = %entry.repo,
                    "DNF upstream version resolved"
                );
                Ok(vec![{
                    let mut r = UpstreamRelease::new(
                        Version::new(&entry.version),
                        entry.version,
                        false,
                        "",
                    );
                    r.category = category;
                    r
                }])
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    /// Fetch available releases for multiple packages using a single `dnf check-update` call.
    ///
    /// Runs:
    /// ```text
    /// dnf check-update --quiet pkg1 pkg2 pkg3
    /// ```
    ///
    /// - Exit code 0: all packages are up to date (empty result per package).
    /// - Exit code 100: updates available (parse output per package).
    /// - Exit code 1: error.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        // Validate all identifiers up front.
        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["check-update".to_string(), "--quiet".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch fetching DNF releases via dnf check-update"
        );

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("dnf", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "dnf check-update failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                // All packages up to date -- return empty releases for each.
                let results = items
                    .iter()
                    .map(|item| BatchFetchResult::empty(item.package_identifier.clone()))
                    .collect();
                return Ok(results);
            }
            100 => {
                // Updates available -- parse output.
            }
            code => bail!(PluginError::CommandFailed(code)),
        }

        let parsed = Self::parse_check_update_output_batch(&cmd_output.output);

        let results = items
            .iter()
            .map(|item| {
                let Some(entry) = parsed.get(&item.package_identifier) else {
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                let category = if entry.repo.to_ascii_lowercase().contains("security") {
                    Some(UpdateCategory::Security)
                } else {
                    None
                };

                BatchFetchResult::found(
                    item.package_identifier.clone(),
                    vec![{
                        let mut r = UpstreamRelease::new(
                            Version::new(&entry.version),
                            entry.version.clone(),
                            false,
                            "",
                        );
                        r.category = category;
                        r
                    }],
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "DNF batch fetch complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexer for DnfPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing DNF package index");
        execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("dnf", ["makecache".to_string(), "-q".to_string()]).privileged(),
            "dnf makecache",
        )
        .await?;

        tracing::info!("DNF package index refreshed");
        Ok(())
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for DnfPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version).map_err(|e| report!(PluginError::Configuration(e)))?;

        // DNF install accepts `pkg-version-release` to pin to a specific build.
        let pkg_version = format!("{package_identifier}-{to_version}");
        let args = vec!["install".to_string(), "-y".to_string(), pkg_version];

        tracing::debug!(
            package = %package_identifier,
            version = %to_version,
            "running dnf install"
        );

        let display_args = std::iter::once("dnf")
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
            .execute(&CommandSpec::exec("dnf", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "dnf install failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Execute batch updates using a single `dnf install -y` invocation.
    ///
    /// DNF handles atomic multi-package installs natively, so all packages
    /// are installed in one command: `sudo dnf install -y pkg1-ver1 pkg2-ver2 ...`.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &UpdateOutputSender,
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

        let mut args = vec!["install".to_string(), "-y".to_string()];
        for item in items {
            args.push(format!("{}-{}", item.package_identifier, item.to_version));
        }

        let display_args = std::iter::once("dnf")
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
            "running dnf batch install"
        );

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("dnf", args).privileged(), output_tx)
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
    use uptrakit_plugin_infrastructure_core::testing::{FixedOutputExecutor, RoutedOutputExecutor};
    use uptrakit_plugin_infrastructure_core::{
        CommandOutput, HostCapabilities, LocalCommandExecutor, PosixHostRuntime, UpdateOutputLine,
    };

    fn test_runtime() -> Arc<dyn HostRuntime> {
        Arc::new(PosixHostRuntime::new(
            Arc::new(LocalCommandExecutor),
            HostCapabilities::default(),
        ))
    }

    fn runtime_from_executor(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
        Arc::new(PosixHostRuntime::new(executor, HostCapabilities::default()))
    }

    // ── validate_identifier ──────────────────────────────────────────────

    #[test]
    fn validate_identifier_valid_simple() {
        assert!(validate_identifier("nginx").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dash() {
        assert!(validate_identifier("httpd-devel").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dot() {
        assert!(validate_identifier("python3.11").is_ok());
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
        let s = "a".repeat(128);
        assert!(validate_identifier(&s).is_ok());
    }

    #[test]
    fn validate_identifier_empty() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_too_long() {
        let s = "a".repeat(129);
        assert!(validate_identifier(&s).is_err());
    }

    #[test]
    fn validate_identifier_starts_with_dash() {
        assert!(validate_identifier("-badname").is_err());
    }

    #[test]
    fn validate_identifier_starts_with_underscore() {
        assert!(validate_identifier("_bad").is_err());
    }

    #[test]
    fn validate_identifier_contains_slash() {
        assert!(validate_identifier("pkg/bad").is_err());
    }

    #[test]
    fn validate_identifier_contains_double_dot() {
        assert!(validate_identifier("pkg..bad").is_err());
    }

    #[test]
    fn validate_identifier_contains_space() {
        assert!(validate_identifier("pkg bad").is_err());
    }

    // ── validate_version ─────────────────────────────────────────────────

    #[test]
    fn validate_version_valid_simple() {
        assert!(validate_version("1.24.0-1.fc40").is_ok());
    }

    #[test]
    fn validate_version_valid_epoch() {
        assert!(validate_version("2:8.1.2269-1").is_ok());
    }

    #[test]
    fn validate_version_valid_tilde() {
        assert!(validate_version("1.0~beta1-1").is_ok());
    }

    #[test]
    fn validate_version_empty() {
        assert!(validate_version("").is_err());
    }

    #[test]
    fn validate_version_starts_with_dash() {
        assert!(validate_version("-1.0").is_err());
    }

    #[test]
    fn validate_version_too_long() {
        let s = "1".repeat(257);
        assert!(validate_version(&s).is_err());
    }

    #[test]
    fn validate_version_contains_space() {
        assert!(validate_version("1.0 bad").is_err());
    }

    #[test]
    fn validate_version_contains_slash() {
        assert!(validate_version("1.0/bad").is_err());
    }

    // ── parse_rpm_output ─────────────────────────────────────────────────

    #[test]
    fn parse_rpm_output_single_line() {
        let output = "nginx\t1.24.0-1.fc40\n";
        let result = DnfPlugin::parse_rpm_output(output);
        assert_eq!(
            result,
            vec![("nginx".to_string(), "1.24.0-1.fc40".to_string())]
        );
    }

    #[test]
    fn parse_rpm_output_multiple_lines() {
        let output = "nginx\t1.24.0-1.fc40\ncurl\t8.0.1-2.fc40\n";
        let result = DnfPlugin::parse_rpm_output(output);
        assert_eq!(result.len(), 2);
        assert!(result.contains(&("nginx".to_string(), "1.24.0-1.fc40".to_string())));
        assert!(result.contains(&("curl".to_string(), "8.0.1-2.fc40".to_string())));
    }

    #[test]
    fn parse_rpm_output_empty() {
        let result = DnfPlugin::parse_rpm_output("");
        assert!(result.is_empty());
    }

    #[test]
    fn parse_rpm_output_skips_empty_version() {
        let output = "nginx\t\ncurl\t8.0.1-2.fc40\n";
        let result = DnfPlugin::parse_rpm_output(output);
        assert_eq!(result.len(), 1);
        assert!(result.contains(&("curl".to_string(), "8.0.1-2.fc40".to_string())));
    }

    // ── parse_check_update_output ────────────────────────────────────────

    #[test]
    fn parse_check_update_output_with_update() {
        let output = "nginx.x86_64    1.26.0-1.fc40    updates\n";
        let result = DnfPlugin::parse_check_update_output(output);
        assert!(result.is_some());
        let entry = result.unwrap();
        assert_eq!(entry.version, "1.26.0-1.fc40");
        assert_eq!(entry.repo, "updates");
    }

    #[test]
    fn parse_check_update_output_security_repo() {
        let output = "nginx.x86_64    1.26.0-1.fc40    updates-security\n";
        let result = DnfPlugin::parse_check_update_output(output);
        assert!(result.is_some());
        let entry = result.unwrap();
        assert!(entry.repo.to_ascii_lowercase().contains("security"));
    }

    #[test]
    fn parse_check_update_output_empty() {
        let result = DnfPlugin::parse_check_update_output("");
        assert!(result.is_none());
    }

    #[test]
    fn parse_check_update_output_skips_metadata_line() {
        let output = "Last metadata expiration check: 0:01:23 ago on Mon Mar 10 12:00:00 2026.\nnginx.x86_64    1.26.0-1.fc40    updates\n";
        let result = DnfPlugin::parse_check_update_output(output);
        assert!(result.is_some());
        let entry = result.unwrap();
        assert_eq!(entry.version, "1.26.0-1.fc40");
    }

    // ── parse_check_update_output_batch ──────────────────────────────────

    #[test]
    fn parse_check_update_output_batch_multiple() {
        let output =
            "nginx.x86_64    1.26.0-1.fc40    updates\ncurl.x86_64     8.1.0-1.fc40     updates\n";
        let result = DnfPlugin::parse_check_update_output_batch(output);
        assert_eq!(result.len(), 2);
        assert_eq!(result["nginx"].version, "1.26.0-1.fc40");
        assert_eq!(result["curl"].version, "8.1.0-1.fc40");
    }

    #[test]
    fn parse_check_update_output_batch_deduplicates() {
        let output =
            "nginx.x86_64    1.26.0-1.fc40    updates\nnginx.x86_64    1.25.0-1.fc40    base\n";
        let result = DnfPlugin::parse_check_update_output_batch(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result["nginx"].version, "1.26.0-1.fc40");
    }

    #[test]
    fn parse_check_update_output_batch_empty() {
        let result = DnfPlugin::parse_check_update_output_batch("");
        assert!(result.is_empty());
    }

    // ── plugin construction ──────────────────────────────────────────────

    #[test]
    fn plugin_new_succeeds() {
        let plugin = DnfPlugin::new(DnfConfig::default(), test_runtime());
        assert!(plugin.is_ok());
    }

    // ── host compatibility ────────────────────────────────────────────────

    #[tokio::test]
    async fn host_compat_compatible_when_dnf_found() {
        let executor = RoutedOutputExecutor::new([("which", "", 0)]);
        let plugin = DnfPlugin::new(DnfConfig::default(), runtime_from_executor(executor)).unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert!(matches!(result, HostCompatibility::Compatible));
    }

    #[tokio::test]
    async fn host_compat_incompatible_when_dnf_not_found() {
        let executor = FixedOutputExecutor::failure(1);
        let plugin = DnfPlugin::new(DnfConfig::default(), runtime_from_executor(executor)).unwrap();
        let result = plugin.detect_host_compatibility().await.unwrap();
        assert!(matches!(result, HostCompatibility::Incompatible(_)));
    }

    // ── discover_software ────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets() {
        // Targets are always emitted regardless of filter.
        let rpm_output = "nginx\t1.24.0-1.fc40\ncurl\t8.0.1-1.fc40\n";
        let executor = RoutedOutputExecutor::new([("rpm", rpm_output, 0)]);
        let plugin = DnfPlugin::new(DnfConfig::default(), runtime_from_executor(executor)).unwrap();
        let result = plugin.discover_software().await.unwrap();
        assert_eq!(result.len(), 2);
        for item in &result {
            assert_eq!(item.targets.len(), 1);
            assert_eq!(
                item.targets[0].plugin_type,
                plugin_ids::PACKAGE_MANAGER_DNF.clone()
            );
        }
    }

    #[tokio::test]
    async fn discover_software_emits_targets_with_explicit_all_filter() {
        let rpm_output = "nginx\t1.24.0-1.fc40\n";
        let executor = RoutedOutputExecutor::new([("rpm", rpm_output, 0)]);
        let config = DnfConfig {
            discovery_filter: DnfDiscoveryFilter::All,
        };
        let plugin = DnfPlugin::new(config, runtime_from_executor(executor)).unwrap();
        let result = plugin.discover_software().await.unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].targets.len(),
            1,
            "explicit All filter must still emit targets"
        );
    }

    // ── batch_detect ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_empty_input() {
        let plugin = DnfPlugin::new(DnfConfig::default(), test_runtime()).unwrap();
        let result = plugin.batch_detect(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn batch_detect_finds_installed_packages() {
        let rpm_output = "nginx\t1.24.0-1.fc40\ncurl\t8.0.1-1.fc40\n";
        let executor = RoutedOutputExecutor::new([("rpm", rpm_output, 0)]);
        let plugin = DnfPlugin::new(DnfConfig::default(), runtime_from_executor(executor)).unwrap();
        let items = vec![
            BatchDetectItem::new("nginx".to_string()),
            BatchDetectItem::new("curl".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.unwrap();
        assert_eq!(results.len(), 2);
        let nginx = results
            .iter()
            .find(|r| r.package_identifier == "nginx")
            .unwrap();
        assert!(nginx.installed_version.is_some());
    }

    // ── batch_fetch ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_empty_input() {
        let plugin = DnfPlugin::new(DnfConfig::default(), test_runtime()).unwrap();
        let result = plugin.batch_fetch(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn batch_fetch_exit_0_all_up_to_date() {
        let executor = RoutedOutputExecutor::new([("dnf", "", 0)]);
        let plugin = DnfPlugin::new(DnfConfig::default(), runtime_from_executor(executor)).unwrap();
        let items = vec![BatchFetchItem::new("nginx".to_string())];
        let results = plugin.batch_fetch(&items).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].releases.is_empty());
    }

    #[tokio::test]
    async fn batch_fetch_exit_100_parses_updates() {
        // RoutedOutputExecutor returns exit_code=0 for dnf, but we need 100.
        // Use a custom executor for this test.
        struct Exit100Executor {
            output: String,
        }
        #[async_trait::async_trait]
        impl CommandExecutor for Exit100Executor {
            async fn execute(
                &self,
                _spec: &CommandSpec,
                _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
            ) -> uptrakit_command::Result<CommandOutput> {
                Ok(CommandOutput {
                    output: self.output.clone(),
                    exit_code: 100,
                })
            }
            async fn execute_quiet(
                &self,
                _spec: &CommandSpec,
            ) -> uptrakit_command::Result<CommandOutput> {
                Ok(CommandOutput {
                    output: self.output.clone(),
                    exit_code: 100,
                })
            }
        }
        let executor = Arc::new(Exit100Executor {
            output: "nginx.x86_64    1.26.0-1.fc40    updates\n".to_string(),
        }) as Arc<dyn CommandExecutor>;
        let plugin = DnfPlugin::new(DnfConfig::default(), runtime_from_executor(executor)).unwrap();
        let items = vec![BatchFetchItem::new("nginx".to_string())];
        let results = plugin.batch_fetch(&items).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].releases.len(), 1);
        assert_eq!(results[0].releases[0].tag, "1.26.0-1.fc40");
    }

    // ── security category ─────────────────────────────────────────────────

    #[test]
    fn security_category_from_security_repo() {
        let output = "nginx.x86_64    1.26.0-1.fc40    updates-security\n";
        let entry = DnfPlugin::parse_check_update_output(output).unwrap();
        let category = if entry.repo.to_ascii_lowercase().contains("security") {
            Some(UpdateCategory::Security)
        } else {
            None
        };
        assert_eq!(category, Some(UpdateCategory::Security));
    }

    #[test]
    fn no_security_category_from_regular_repo() {
        let output = "nginx.x86_64    1.26.0-1.fc40    updates\n";
        let entry = DnfPlugin::parse_check_update_output(output).unwrap();
        let category: Option<UpdateCategory> =
            if entry.repo.to_ascii_lowercase().contains("security") {
                Some(UpdateCategory::Security)
            } else {
                None
            };
        assert!(category.is_none());
    }
}
