use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::helpers::validation_error_message;
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, BatchUpdateItem,
    BatchUpdateResult, ConfigModel, ConfigTestKind, DiscoveredSoftware, DiscoveryTarget,
    ExecuteUpdateResult, HostCompatibility, HostRequirements, HostRuntime,
    PluginConfigValidationError, PluginError, PluginFamily, PluginRole, ReleaseInfo, Result,
    SudoCommandEntry, UpdateOutputLine, UpstreamRelease, Version, declare_plugin,
    execute_and_capture, plugin_ids,
};

use uptrakit_shared_types::PackageIdentifierRules;

use crate::config::{ApkConfig, ApkDiscoveryFilter};

const IDENTIFIER_RULES: PackageIdentifierRules = PackageIdentifierRules {
    min_len: 2,
    max_len: 100,
    first_char_valid: |c| c.is_ascii_lowercase() || c.is_ascii_digit(),
    char_valid: |c| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '+' | '-')
    },
    reject_double_dot: true,
};

/// Validate an Alpine APK package identifier.
///
/// Alpine Linux package naming rules:
/// - Between 2 and 100 characters long.
/// - Must start with a lowercase letter or digit (`[a-z0-9]`).
/// - May only contain lowercase letters, digits, `.`, `_`, `+`, `-`.
/// - Must not contain `..` (path traversal protection).
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    IDENTIFIER_RULES
        .validate(value)
        .map_err(PluginConfigValidationError::InvalidIdentifier)
}

/// Validate an APK version string before it is interpolated into install commands.
///
/// APK versions (e.g. `1.2.3-r0`, `20230506-r0`):
/// - Non-empty
/// - At most 256 characters
/// - Must not start with `-` (would be interpreted as a CLI flag)
/// - Characters: `[a-zA-Z0-9._\-+~:]`
pub fn validate_version(version: &str) -> std::result::Result<(), PluginConfigValidationError> {
    if version.is_empty() {
        return Err(PluginConfigValidationError::Contract(
            "version must not be empty".to_string(),
        ));
    }
    if version.len() > 256 {
        return Err(PluginConfigValidationError::Contract(
            "version must not exceed 256 characters".to_string(),
        ));
    }
    if version.starts_with('-') {
        return Err(PluginConfigValidationError::Contract(
            "version must not start with '-' (would be interpreted as a flag)".to_string(),
        ));
    }
    for ch in version.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '_' | '-' | '+' | '~' | ':') {
            return Err(PluginConfigValidationError::Contract(format!(
                "version contains invalid character: '{ch}'"
            )));
        }
    }
    Ok(())
}

/// Splits `name-version` at the first `-` followed by an ASCII digit.
///
/// APK package names may contain hyphens (e.g. `ca-certificates`). The version
/// always starts with a digit after the separating `-`. Example:
/// `ca-certificates-20230506-r0` → `("ca-certificates", "20230506-r0")`
fn split_name_version(name_version: &str) -> Option<(&str, &str)> {
    let bytes = name_version.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i - 1] == b'-' && bytes[i].is_ascii_digit() {
            return Some((&name_version[..i - 1], &name_version[i..]));
        }
    }
    None
}

/// Parses a single line from `apk list --installed` output.
///
/// Format: `<name>-<version> <arch> {<origin>} (<license>) [installed]`
///
/// Returns `(name, version)` or `None` if the line is malformed or not
/// flagged as `[installed]`.
fn parse_apk_list_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // Must be marked as [installed]
    if !line.contains("[installed]") {
        return None;
    }
    // First whitespace-separated token is `name-version`
    let name_version = line.split_whitespace().next()?;
    let (name, version) = split_name_version(name_version)?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name.to_string(), version.to_string()))
}

/// Parses `apk info -v pkg1 pkg2 ...` output into a map of package name -> installed version.
///
/// Each output line is `<name>-<version>`. Only lines whose package name is in
/// `pkg_names` are included.
fn parse_apk_info_output(output: &str, pkg_names: &HashSet<&str>) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, version)) = split_name_version(line) else {
            continue;
        };
        if pkg_names.contains(name) {
            result.insert(name.to_string(), version.to_string());
        }
    }
    result
}

/// Parses a single line from `apk version pkg1 pkg2 ...` output.
///
/// Format: `<name>-<installed_ver> <cmp_op> <latest_ver>`
/// where `cmp_op` is `=`, `<`, or `>`.
///
/// Returns `(raw_name_version_token, latest_version)` or `None` for non-package lines
/// (e.g. `fetch ...` lines, empty lines).
fn parse_apk_version_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    // Skip "fetch ..." lines (header lines from apk version)
    if line.starts_with("fetch") {
        return None;
    }
    // Expected format: `<name-installed_ver> <cmp_op> <latest_ver>`
    let mut parts = line.splitn(3, ' ');
    let name_ver = parts.next()?.trim();
    let _ = parts.next()?;
    let latest_ver = parts.next()?.trim();

    if latest_ver.is_empty() {
        return None;
    }
    Some((name_ver.to_string(), latest_ver.to_string()))
}

/// Parses `apk version pkg1 pkg2 ...` output into a map of package name -> latest version.
fn parse_apk_version_output(output: &str, pkg_names: &HashSet<&str>) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for line in output.lines() {
        let Some((name_ver, latest)) = parse_apk_version_line(line) else {
            continue;
        };
        let Some((name, _installed)) = split_name_version(&name_ver) else {
            continue;
        };
        if pkg_names.contains(name) {
            result.insert(name.to_string(), latest);
        }
    }
    result
}

/// Parses a single line from `/etc/apk/world`.
///
/// The world file contains one package atom per line. Atoms may have:
/// - A `~flag:` prefix (e.g. `~edge:`)
/// - A `repo/` prefix (e.g. `community/nodejs`)
/// - A combined `~flag:repo/` prefix (e.g. `~edge:community/nodejs`)
/// - A version constraint suffix (e.g. `openssl>=3.0`, `busybox=1.36.1-r5`)
///
/// Returns the bare package name with all prefixes and version constraints stripped,
/// or `None` for empty/comment lines.
fn parse_world_line(line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    // Strip `~flag:` prefix
    let line = if let Some(colon_pos) = line.find(':') {
        &line[colon_pos + 1..]
    } else {
        line
    };

    // Strip `repo/` prefix
    let line = if let Some(slash_pos) = line.find('/') {
        &line[slash_pos + 1..]
    } else {
        line
    };

    // Strip version constraint suffix: anything starting with `=`, `>`, `<`, `~`, `!`
    let name = line
        .find(['=', '>', '<', '~', '!'])
        .map_or(line, |pos| &line[..pos]);

    let name = name.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Plugin for APK (Alpine Linux package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for Alpine Linux packages managed by `apk`.
///
/// The `package_identifier` in `SoftwareItem` is the Alpine package name
/// (e.g., `nginx`, `openssl`, `ca-certificates`).
pub struct ApkPlugin {
    config: ApkConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl ApkPlugin {
    /// Create a new APK plugin with the given configuration and host runtime.
    pub fn new(
        config: ApkConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self { config, executor })
    }

    /// Sudo commands required by this plugin.
    fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("apk", "Refresh the APK package index")
                .with_args_suffix("update"),
            SudoCommandEntry::new("apk", "Install or upgrade an APK package")
                .with_args_suffix("add *"),
        ]
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier,
        )
    }
}

// ── Plugin descriptor ─────────────────────────────────────────────────────

declare_plugin!(ApkPlugin, ApkConfig, "package_manager_apk", {
    display_name: "APK",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    type_settings: true,
    roles: [Discoverer, VersionDetector, ReleaseFetcher,
            PackageIndexer { host_requirements: HostRequirements::POSIX_PRIVILEGED },
            UpdateExecutor { host_requirements: HostRequirements::POSIX_PRIVILEGED }],
    sudo: ApkPlugin::required_sudo_commands,
});

#[async_trait]
impl uptrakit_plugin_infrastructure_core::Discoverer for ApkPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering APK-managed software");

        match self.config.effective_filter() {
            ApkDiscoveryFilter::All => {
                // Run `apk list --installed` to get all installed packages.
                let list_stdout = execute_and_capture(
                    self.executor.as_ref(),
                    CommandSpec::exec("apk", ["list".to_string(), "--installed".to_string()]),
                    "apk list --installed",
                )
                .await?;

                let packages: Vec<DiscoveredSoftware> = list_stdout
                    .lines()
                    .filter_map(parse_apk_list_line)
                    .map(|(name, version)| {
                        let targets = vec![DiscoveryTarget {
                            plugin_type: plugin_ids::PACKAGE_MANAGER_APK.clone(),
                            plugin_config: serde_json::json!({}),
                            plugin_config_name: "APK".to_string(),
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

                tracing::debug!(count = packages.len(), "APK software discovery complete");
                Ok(packages)
            }

            ApkDiscoveryFilter::World => {
                // Read /etc/apk/world to get explicitly installed package names.
                let world_stdout = execute_and_capture(
                    self.executor.as_ref(),
                    CommandSpec::exec("cat", ["/etc/apk/world".to_string()]),
                    "cat /etc/apk/world",
                )
                .await?;

                let pkg_names: Vec<String> =
                    world_stdout.lines().filter_map(parse_world_line).collect();

                if pkg_names.is_empty() {
                    tracing::debug!("APK world file is empty; no packages to discover");
                    return Ok(vec![]);
                }

                // Run `apk info -v pkg1 pkg2 ...` to get installed versions.
                let mut info_args = vec!["info".to_string(), "-v".to_string()];
                info_args.extend(pkg_names.iter().cloned());

                let info_output = self
                    .executor
                    .execute_quiet(&CommandSpec::exec("apk", info_args))
                    .await
                    .map_err(|e| {
                        report!(PluginError::PluginInternal(format!(
                            "apk info -v failed: {e}"
                        )))
                    })?;

                // apk info exits non-zero if any package is not found; treat as partial result.
                let pkg_name_set: HashSet<&str> = pkg_names.iter().map(String::as_str).collect();
                let version_map = parse_apk_info_output(&info_output.output, &pkg_name_set);

                let packages: Vec<DiscoveredSoftware> = pkg_names
                    .iter()
                    .filter_map(|name| {
                        let version = version_map.get(name)?.clone();
                        let targets = vec![DiscoveryTarget {
                            plugin_type: plugin_ids::PACKAGE_MANAGER_APK.clone(),
                            plugin_config: serde_json::json!({}),
                            plugin_config_name: "APK".to_string(),
                            roles: vec![
                                PluginRole::DetectVersion,
                                PluginRole::FetchReleases,
                                PluginRole::ExecuteUpdate,
                            ],
                            package_identifier: None,
                            config_override: None,
                            execution_site: None,
                        }];
                        Some(DiscoveredSoftware {
                            package_identifier: name.clone(),
                            name: name.clone(),
                            installed_version: version,
                            targets,
                            extra: None,
                            qualifier: None,
                            plugin_package_identifier: None,
                            featured: false,
                            installed_display_version: None,
                        })
                    })
                    .collect();

                tracing::debug!(
                    count = packages.len(),
                    "APK world-mode software discovery complete"
                );
                Ok(packages)
            }
        }
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["apk".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible("apk not found".to_string())),
        }
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for ApkPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting APK installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "apk",
                [
                    "info".to_string(),
                    "-v".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "apk info -v failed: {e}"
                )))
            })?;

        // apk info exits non-zero when package is not installed.
        if cmd_output.exit_code != 0 {
            return Ok(None);
        }

        let pkg_names: HashSet<&str> = [package_identifier].into_iter().collect();
        let version_map = parse_apk_info_output(&cmd_output.output, &pkg_names);

        let version = version_map.get(package_identifier).map(Version::new);
        tracing::debug!(
            package = %package_identifier,
            version = ?version,
            "APK version detection result"
        );
        Ok(version)
    }

    #[tracing::instrument(skip_all)]
    async fn batch_detect(&self, items: &[BatchDetectItem]) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting APK installed versions"
        );

        let mut args = vec!["info".to_string(), "-v".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("apk", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "apk info -v failed: {e}"
                )))
            })?;

        // apk info exits non-zero if any requested package is not installed.
        // We still parse the output to get versions for the ones that are installed.
        let pkg_names: HashSet<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        let version_map = parse_apk_info_output(&cmd_output.output, &pkg_names);

        let results = items
            .iter()
            .map(|item| {
                let installed_version = version_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(count = items.len(), "APK batch version detection complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for ApkPlugin {
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching APK releases");

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec(
                "apk",
                ["version".to_string(), package_identifier.to_string()],
            ),
            "apk version",
        )
        .await?;

        let pkg_names: HashSet<&str> = [package_identifier].into_iter().collect();
        let version_map = parse_apk_version_output(&stdout, &pkg_names);

        let Some(latest_version) = version_map.get(package_identifier) else {
            bail!(PluginError::PluginInternal(format!(
                "package not installed: {package_identifier}"
            )));
        };

        let release_url =
            format!("https://pkgs.alpinelinux.org/packages?name={package_identifier}");

        tracing::debug!(
            package = %package_identifier,
            version = %latest_version,
            "APK upstream version resolved"
        );

        Ok(vec![{
            let mut r = UpstreamRelease::new(
                Version::new(latest_version),
                latest_version.clone(),
                false,
                "",
            );
            r.release_url = release_url;
            r
        }])
    }

    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;
        }

        tracing::debug!(count = items.len(), "batch fetching APK releases");

        let mut args = vec!["version".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let stdout = execute_and_capture(
            self.executor.as_ref(),
            CommandSpec::exec("apk", args),
            "apk version",
        )
        .await?;

        let pkg_names: HashSet<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();
        let version_map = parse_apk_version_output(&stdout, &pkg_names);

        let results = items
            .iter()
            .map(|item| {
                let id = &item.package_identifier;
                match version_map.get(id) {
                    Some(latest_version) => {
                        let release_url =
                            format!("https://pkgs.alpinelinux.org/packages?name={id}");
                        BatchFetchResult::found(
                            id.clone(),
                            vec![{
                                let mut r = UpstreamRelease::new(
                                    Version::new(latest_version),
                                    latest_version.clone(),
                                    false,
                                    "",
                                );
                                r.release_url = release_url;
                                r
                            }],
                        )
                    }
                    None => {
                        BatchFetchResult::error(id.clone(), format!("package not installed: {id}"))
                    }
                }
            })
            .collect();

        tracing::debug!(count = items.len(), "APK batch fetch complete");
        Ok(results)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::PackageIndexer for ApkPlugin {
    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        uptrakit_plugin_infrastructure_core::refresh_package_index_command(
            self.executor.as_ref(),
            CommandSpec::exec("apk", ["update".to_string()]).privileged(),
            "APK package index",
        )
        .await
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for ApkPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<ExecuteUpdateResult> {
        self.require_package_identifier(package_identifier)?;
        validate_version(to_version)
            .map_err(|e| report!(PluginError::Configuration(validation_error_message(e))))?;

        tracing::debug!(package = %package_identifier, version = %to_version, "running apk add");

        let output = uptrakit_plugin_infrastructure_core::execute_command_update(
            uptrakit_plugin_infrastructure_core::CommandUpdateParams {
                executor: self.executor.as_ref(),
                binary: "apk",
                args: vec![
                    "add".to_string(),
                    format!("{package_identifier}={to_version}"),
                ],
                privileged: true,
                spec_modifier: None,
                exit_code_success: None,
                exit_code_error: Some(|_, code| PluginError::CommandFailed(code)),
            },
            output_tx,
        )
        .await?;
        Ok(ExecuteUpdateResult::new(output, false))
    }

    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        tracing::debug!(count = items.len(), "running APK batch update");
        let result = uptrakit_plugin_infrastructure_core::execute_batch_versioned_command(
            uptrakit_plugin_infrastructure_core::BatchVersionedParams {
                executor: self.executor.as_ref(),
                binary: "apk",
                prefix_args: vec!["add".to_string()],
                privileged: true,
                format_item: |id, ver| format!("{id}={ver}"),
                validate_identifier,
                validate_version,
            },
            items,
            output_tx,
        )
        .await;
        if let Ok(ref results) = result {
            let success = results.first().is_none_or(|r| r.success);
            tracing::debug!(count = items.len(), success, "APK batch update complete");
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{
        CommandOutput, Discoverer, ReleaseFetcher, UpdateOutputLine, VersionDetector, mpsc::Sender,
    };

    // ── Mock executor ───────────────────────────────────────────────────────

    struct MockApkExecutor {
        list_output: String,
        info_output: String,
        version_output: String,
        world_output: String,
        which_exit_code: i32,
        info_exit_code: i32,
    }

    impl MockApkExecutor {
        fn new(
            list_output: &str,
            info_output: &str,
            version_output: &str,
            world_output: &str,
        ) -> Self {
            Self {
                list_output: list_output.to_string(),
                info_output: info_output.to_string(),
                version_output: version_output.to_string(),
                world_output: world_output.to_string(),
                which_exit_code: 0,
                info_exit_code: 0,
            }
        }

        fn incompatible() -> Self {
            Self {
                list_output: String::new(),
                info_output: String::new(),
                version_output: String::new(),
                world_output: String::new(),
                which_exit_code: 1,
                info_exit_code: 0,
            }
        }
    }

    #[async_trait::async_trait]
    impl CommandExecutor for MockApkExecutor {
        async fn execute(
            &self,
            spec: &CommandSpec,
            _output_tx: &Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            self.execute_quiet(spec).await
        }

        async fn execute_quiet(
            &self,
            spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            use uptrakit_plugin_infrastructure_core::command::CommandMode;
            let (program, args) = match &spec.mode {
                CommandMode::Exec { program, args } => (
                    program.as_str(),
                    args.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                ),
                _ => {
                    return Ok(CommandOutput {
                        output: String::new(),
                        exit_code: 127,
                    });
                }
            };
            let output = match (program, args.as_slice()) {
                ("which", ["apk"]) => {
                    if self.which_exit_code != 0 {
                        return Err(report!(
                            uptrakit_command::error::CommandError::CommandFailed(
                                self.which_exit_code
                            )
                        ));
                    }
                    CommandOutput {
                        output: "/sbin/apk\n".to_string(),
                        exit_code: 0,
                    }
                }
                ("apk", ["list", "--installed"]) => CommandOutput {
                    output: self.list_output.clone(),
                    exit_code: 0,
                },
                ("apk", ["info", "-v", ..]) => CommandOutput {
                    output: self.info_output.clone(),
                    exit_code: self.info_exit_code,
                },
                ("apk", ["version", ..]) => CommandOutput {
                    output: self.version_output.clone(),
                    exit_code: 0,
                },
                ("apk", ["add", ..]) => CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                },
                ("apk", ["update"]) => CommandOutput {
                    output: String::new(),
                    exit_code: 0,
                },
                ("cat", ["/etc/apk/world"]) => CommandOutput {
                    output: self.world_output.clone(),
                    exit_code: 0,
                },
                _ => CommandOutput {
                    output: String::new(),
                    exit_code: 127,
                },
            };
            Ok(output)
        }
    }

    fn make_plugin_all(list_output: &str, info_output: &str, version_output: &str) -> ApkPlugin {
        ApkPlugin {
            config: ApkConfig::default(),
            executor: Arc::new(MockApkExecutor::new(
                list_output,
                info_output,
                version_output,
                "",
            )),
        }
    }

    fn make_plugin_world(world_output: &str, info_output: &str, version_output: &str) -> ApkPlugin {
        ApkPlugin {
            config: ApkConfig {
                discovery_filter: ApkDiscoveryFilter::World,
            },
            executor: Arc::new(MockApkExecutor::new(
                "",
                info_output,
                version_output,
                world_output,
            )),
        }
    }

    const SAMPLE_LIST: &str = "\
busybox-1.36.1-r5 x86_64 {busybox} (GPL-2.0-only) [installed]
openssl-3.1.4-r5 x86_64 {openssl} (OpenSSL) [installed]
ca-certificates-20230506-r0 x86_64 {ca-certificates} (MPL-2.0 AND MIT) [installed]
";

    const SAMPLE_INFO: &str = "\
busybox-1.36.1-r5
openssl-3.1.4-r5
ca-certificates-20230506-r0
";

    const SAMPLE_VERSION: &str = "\
busybox-1.36.1-r5 = 1.36.1-r5
openssl-3.1.4-r5 < 3.3.2-r0
";

    const SAMPLE_WORLD: &str = "\
busybox
openssl>=3.0
~edge:community/nodejs
";

    // ── split_name_version ─────────────────────────────────────────────────

    #[test]
    fn split_name_version_simple() {
        assert_eq!(
            split_name_version("busybox-1.36.1-r5"),
            Some(("busybox", "1.36.1-r5"))
        );
    }

    #[test]
    fn split_name_version_hyphenated_name() {
        assert_eq!(
            split_name_version("ca-certificates-20230506-r0"),
            Some(("ca-certificates", "20230506-r0"))
        );
    }

    #[test]
    fn split_name_version_no_version() {
        assert_eq!(split_name_version("busybox"), None);
    }

    #[test]
    fn split_name_version_hyphen_not_followed_by_digit() {
        // "abc-def" -- no digit after the hyphen -> None
        assert_eq!(split_name_version("abc-def"), None);
    }

    // ── parse_apk_list_line ────────────────────────────────────────────────

    #[test]
    fn parse_apk_list_line_standard() {
        let result =
            parse_apk_list_line("busybox-1.36.1-r5 x86_64 {busybox} (GPL-2.0-only) [installed]");
        assert_eq!(
            result,
            Some(("busybox".to_string(), "1.36.1-r5".to_string()))
        );
    }

    #[test]
    fn parse_apk_list_line_hyphenated_name() {
        let result = parse_apk_list_line(
            "ca-certificates-20230506-r0 x86_64 {ca-certificates} (MPL-2.0 AND MIT) [installed]",
        );
        assert_eq!(
            result,
            Some(("ca-certificates".to_string(), "20230506-r0".to_string()))
        );
    }

    #[test]
    fn parse_apk_list_line_not_installed_skipped() {
        let result = parse_apk_list_line("busybox-1.36.1-r5 x86_64 {busybox} (GPL-2.0-only)");
        assert_eq!(result, None);
    }

    #[test]
    fn parse_apk_list_line_empty_skipped() {
        assert_eq!(parse_apk_list_line(""), None);
    }

    // ── parse_apk_info_output ───────────────────────────────────────────────

    #[test]
    fn parse_apk_info_output_standard() {
        let pkg_names: HashSet<&str> = ["busybox", "openssl"].into_iter().collect();
        let map = parse_apk_info_output(SAMPLE_INFO, &pkg_names);
        assert_eq!(map.get("busybox").map(String::as_str), Some("1.36.1-r5"));
        assert_eq!(map.get("openssl").map(String::as_str), Some("3.1.4-r5"));
        // ca-certificates not in pkg_names -> not included
        assert!(!map.contains_key("ca-certificates"));
    }

    #[test]
    fn parse_apk_info_output_empty() {
        let pkg_names: HashSet<&str> = ["busybox"].into_iter().collect();
        let map = parse_apk_info_output("", &pkg_names);
        assert!(map.is_empty());
    }

    // ── parse_apk_version_output ────────────────────────────────────────────

    #[test]
    fn parse_apk_version_output_up_to_date() {
        let pkg_names: HashSet<&str> = ["busybox"].into_iter().collect();
        let map = parse_apk_version_output(SAMPLE_VERSION, &pkg_names);
        assert_eq!(map.get("busybox").map(String::as_str), Some("1.36.1-r5"));
    }

    #[test]
    fn parse_apk_version_output_upgrade_available() {
        let pkg_names: HashSet<&str> = ["openssl"].into_iter().collect();
        let map = parse_apk_version_output(SAMPLE_VERSION, &pkg_names);
        assert_eq!(map.get("openssl").map(String::as_str), Some("3.3.2-r0"));
    }

    #[test]
    fn parse_apk_version_output_not_in_output() {
        let pkg_names: HashSet<&str> = ["curl"].into_iter().collect();
        let map = parse_apk_version_output(SAMPLE_VERSION, &pkg_names);
        assert!(!map.contains_key("curl"));
    }

    // ── parse_world_line ────────────────────────────────────────────────────

    #[test]
    fn parse_world_line_simple() {
        assert_eq!(parse_world_line("busybox"), Some("busybox".to_string()));
    }

    #[test]
    fn parse_world_line_version_constraint() {
        assert_eq!(
            parse_world_line("openssl>=3.0"),
            Some("openssl".to_string())
        );
    }

    #[test]
    fn parse_world_line_edge_prefix() {
        assert_eq!(
            parse_world_line("~edge:community/nodejs"),
            Some("nodejs".to_string())
        );
    }

    #[test]
    fn parse_world_line_empty() {
        assert_eq!(parse_world_line(""), None);
    }

    #[test]
    fn parse_world_line_comment() {
        assert_eq!(parse_world_line("# comment"), None);
    }

    // ── validate_identifier ────────────────────────────────────────────────

    #[test]
    fn validate_identifier_accepts_valid() {
        assert!(validate_identifier("busybox").is_ok());
        assert!(validate_identifier("ca-certificates").is_ok());
        assert!(validate_identifier("py3-pip").is_ok());
        assert!(validate_identifier("libssl3").is_ok());
        assert!(validate_identifier("ab").is_ok()); // minimum length
    }

    #[test]
    fn validate_identifier_rejects_empty() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_rejects_single_char() {
        assert!(validate_identifier("a").is_err());
    }

    #[test]
    fn validate_identifier_rejects_too_long() {
        assert!(validate_identifier(&"a".repeat(101)).is_err());
    }

    #[test]
    fn validate_identifier_rejects_uppercase() {
        assert!(validate_identifier("BusyBox").is_err());
    }

    #[test]
    fn validate_identifier_rejects_path_traversal() {
        assert!(validate_identifier("foo..bar").is_err());
    }

    #[test]
    fn validate_identifier_rejects_invalid_char() {
        assert!(validate_identifier("foo bar").is_err());
        assert!(validate_identifier("foo/bar").is_err());
    }

    // ── validate_version ────────────────────────────────────────────────────

    #[test]
    fn validate_version_accepts_valid() {
        assert!(validate_version("1.36.1-r5").is_ok());
        assert!(validate_version("20230506-r0").is_ok());
        assert!(validate_version("3.3.2-r0").is_ok());
        assert!(validate_version("1.0.0~beta1").is_ok());
    }

    #[test]
    fn validate_version_rejects_empty() {
        assert!(validate_version("").is_err());
    }

    #[test]
    fn validate_version_rejects_leading_dash() {
        assert!(validate_version("-r0").is_err());
    }

    #[test]
    fn validate_version_rejects_invalid_char() {
        assert!(validate_version("1.0 bad").is_err());
    }

    #[test]
    fn validate_version_rejects_too_long() {
        assert!(validate_version(&"1".repeat(257)).is_err());
    }

    // ── discover_software (all mode) ───────────────────────────────────────

    #[tokio::test]
    async fn discover_software_all_mode() {
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let discovered = plugin.discover_software().await.expect("discover");
        assert_eq!(discovered.len(), 3);

        let busybox = discovered
            .iter()
            .find(|d| d.package_identifier == "busybox")
            .expect("busybox");
        assert_eq!(busybox.installed_version, "1.36.1-r5");
    }

    #[tokio::test]
    async fn discover_software_emits_discovery_target_in_all_mode() {
        // Targets are always emitted regardless of filter.
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let discovered = plugin.discover_software().await.expect("discover");
        assert!(!discovered.is_empty());
        for item in &discovered {
            assert_eq!(item.targets.len(), 1, "item '{}' missing target", item.name);
            assert_eq!(
                item.targets[0].plugin_type,
                plugin_ids::PACKAGE_MANAGER_APK.clone()
            );
            assert_eq!(item.targets[0].plugin_config_name, "APK");
        }
    }

    #[tokio::test]
    async fn discover_software_explicit_all_emits_targets() {
        // Explicit All filter -> targets still emitted.
        let plugin = ApkPlugin {
            config: ApkConfig {
                discovery_filter: ApkDiscoveryFilter::All,
            },
            executor: Arc::new(MockApkExecutor::new(
                SAMPLE_LIST,
                SAMPLE_INFO,
                SAMPLE_VERSION,
                "",
            )),
        };
        let discovered = plugin.discover_software().await.expect("discover");
        assert_eq!(discovered.len(), 3);
        for item in &discovered {
            assert_eq!(
                item.targets.len(),
                1,
                "item '{}' should have one target",
                item.name
            );
        }
    }

    #[tokio::test]
    async fn discover_software_world_mode() {
        let plugin = make_plugin_world(SAMPLE_WORLD, SAMPLE_INFO, SAMPLE_VERSION);
        let discovered = plugin.discover_software().await.expect("discover");
        // busybox and openssl parsed from world; nodejs is also parsed but apk info won't have it
        // The mock returns SAMPLE_INFO which has busybox and openssl.
        assert_eq!(discovered.len(), 2);

        let pkg_ids: Vec<&str> = discovered
            .iter()
            .map(|d| d.package_identifier.as_str())
            .collect();
        assert!(pkg_ids.contains(&"busybox"));
        assert!(pkg_ids.contains(&"openssl"));
    }

    // ── batch_detect ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_detect_installed_versions() {
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let items = vec![
            BatchDetectItem::new("busybox".to_string()),
            BatchDetectItem::new("openssl".to_string()),
        ];
        let results = plugin.batch_detect(&items).await.expect("batch detect");
        assert_eq!(results.len(), 2);

        let busybox_r = results
            .iter()
            .find(|r| r.package_identifier == "busybox")
            .expect("busybox");
        assert_eq!(
            busybox_r.installed_version.as_ref().map(|v| v.as_str()),
            Some("1.36.1-r5")
        );
        assert!(busybox_r.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_unknown_package_returns_none() {
        let plugin = make_plugin_all(SAMPLE_LIST, "", SAMPLE_VERSION);
        let items = vec![BatchDetectItem::new("curl".to_string())];
        let results = plugin.batch_detect(&items).await.expect("batch detect");
        assert_eq!(results.len(), 1);
        assert!(results[0].installed_version.is_none());
        assert!(results[0].error.is_none());
    }

    // ── batch_fetch ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_up_to_date() {
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let items = vec![BatchFetchItem::new("busybox".to_string())];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_none());
        assert_eq!(results[0].releases.len(), 1);
        assert_eq!(results[0].releases[0].tag, "1.36.1-r5");
        assert_eq!(
            results[0].releases[0].release_url,
            "https://pkgs.alpinelinux.org/packages?name=busybox"
        );
    }

    #[tokio::test]
    async fn batch_fetch_upgrade_available() {
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let items = vec![BatchFetchItem::new("openssl".to_string())];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_none());
        assert_eq!(results[0].releases.len(), 1);
        assert_eq!(results[0].releases[0].tag, "3.3.2-r0");
    }

    #[tokio::test]
    async fn batch_fetch_missing_package_returns_error() {
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let items = vec![BatchFetchItem::new("curl".to_string())];
        let results = plugin.batch_fetch(&items).await.expect("batch fetch");
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_some());
        assert!(results[0].releases.is_empty());
    }

    // ── execute_update ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_constructs_correct_command() {
        use uptrakit_plugin_infrastructure_core::UpdateExecutor;
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let (tx, _rx) = mpsc::channel(16);
        let result = plugin
            .execute_update("busybox", "1.36.1-r5", None, &tx)
            .await
            .expect("execute update");
        assert!(result.output.contains("apk add busybox=1.36.1-r5"));
    }

    // ── detect_host_compatibility ───────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible() {
        let plugin = make_plugin_all(SAMPLE_LIST, SAMPLE_INFO, SAMPLE_VERSION);
        let compat = plugin
            .detect_host_compatibility()
            .await
            .expect("compatibility");
        assert!(matches!(compat, HostCompatibility::Compatible));
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible() {
        let plugin = ApkPlugin {
            config: ApkConfig::default(),
            executor: Arc::new(MockApkExecutor::incompatible()),
        };
        let compat = plugin
            .detect_host_compatibility()
            .await
            .expect("compatibility");
        assert!(matches!(compat, HostCompatibility::Incompatible(_)));
    }

    // ── required_sudo_commands ───────────────────────────────────────────────

    #[test]
    fn apk_plugin_required_sudo_commands() {
        assert!(DESCRIPTOR.sudo.is_some());
        let entries = (DESCRIPTOR.sudo.unwrap())(&serde_json::json!({}));
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.command == "apk"));
        assert!(entries.iter().all(|e| !e.needs_setenv));
        assert_eq!(entries[0].args_suffix.as_deref(), Some("update"));
        assert_eq!(entries[1].args_suffix.as_deref(), Some("add *"));
    }

    // ── capabilities ─────────────────────────────────────────────────────────

    #[test]
    fn apk_plugin_capabilities() {
        use uptrakit_plugin_infrastructure_core::PluginCapability;
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::RefreshPackageIndex)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DetectHostCompatibility)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::VersionDetection)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ReleaseFetching)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::UpdateExecution)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
        assert_eq!(DESCRIPTOR.capabilities.len(), 7);
    }
}
