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
    Plugin, PluginCapability, PluginError, PluginRole, PluginType, Result, SudoCommandEntry,
    UpdateCategory, UpstreamRelease, Version,
};

use crate::config::{PkgConfig, PkgDiscoveryFilter};

/// Validate a BSD pkg package identifier.
///
/// Enforces FreeBSD pkg naming conventions:
/// - Non-empty.
/// - At most 200 characters.
/// - Must start with `[a-zA-Z0-9]`.
/// - May only contain `[a-zA-Z0-9._+\-]`.
/// - Must not contain `..` (path traversal protection).
/// - Must not start or end with `-` or `.`.
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value.len() > 200 {
        return Err("package_identifier must not exceed 200 characters".to_string());
    }

    let first = value.chars().next().unwrap_or('\0');
    if !first.is_ascii_alphanumeric() {
        return Err(format!(
            "package_identifier must start with an alphanumeric character, found '{first}'"
        ));
    }

    let last = value.chars().next_back().unwrap_or('\0');
    if last == '-' || last == '.' {
        return Err(format!(
            "package_identifier must not end with '-' or '.', found '{last}'"
        ));
    }

    for ch in value.chars() {
        if !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '+' | '-' | '_') {
            return Err(format!(
                "package_identifier contains invalid character: '{ch}'"
            ));
        }
    }

    if value.contains("..") {
        return Err("package_identifier must not contain '..'".to_string());
    }

    Ok(())
}

/// Plugin for BSD pkg (FreeBSD's pkgng package manager).
///
/// Supports installed version detection, package index refresh, autodiscovery,
/// and updates for packages managed by `pkg` on FreeBSD, TrueNAS SCALE,
/// OPNsense, pfSense, and DragonFly BSD.
///
/// The `package_identifier` in `SoftwareItem` is the pkg package name
/// (e.g., `nginx`, `python39`, `curl`).
pub struct PkgPlugin {
    config: PkgConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl PkgPlugin {
    /// Compile-time capabilities for the BSD pkg plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::RefreshPackageIndex,
        PluginCapability::DetectHostCompatibility,
    ];

    /// Create a new BSD pkg plugin with the given configuration.
    pub async fn new(config: PkgConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Parse `pkg query -a "%n\t%v"` output.
    ///
    /// Each line is a tab-separated `name\tversion` pair. Lines with an
    /// empty name or version are skipped.
    fn parse_pkg_query_line(output: &str) -> Vec<(String, String)> {
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

    /// Parse `pkg query -a "%a\t%n\t%v"` output.
    ///
    /// Each line has the format `auto_flag\tname\tversion` where `auto_flag` is
    /// `"0"` for manually installed packages and `"1"` for automatically
    /// installed dependencies.
    ///
    /// Returns all lines as `(auto_flag, name, version)` triples. Lines that
    /// cannot be parsed are silently skipped.
    fn parse_pkg_query_with_auto_line(output: &str) -> Vec<(String, String, String)> {
        output
            .lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let auto_flag = parts.next()?.trim().to_string();
                let name = parts.next()?.trim().to_string();
                let version = parts.next()?.trim().to_string();
                if auto_flag.is_empty() || name.is_empty() || version.is_empty() {
                    None
                } else {
                    Some((auto_flag, name, version))
                }
            })
            .collect()
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        validate_identifier(package_identifier).map_err(|e| report!(PluginError::Configuration(e)))
    }
}

#[async_trait]
impl Plugin for PkgPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::PackageManagerPkg
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["pkg".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible("pkg not found".to_string())),
        }
    }

    fn required_sudo_commands(&self) -> Vec<SudoCommandEntry> {
        vec![
            SudoCommandEntry::new("pkg", "Package index refresh requires root privileges")
                .with_args_suffix(Cow::Borrowed("update *")),
            SudoCommandEntry::new("pkg", "Package installation requires root privileges")
                .with_args_suffix(Cow::Borrowed("install -y *")),
        ]
    }

    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing BSD pkg package index");
        let cmd_output = self
            .executor
            .execute_quiet(
                &CommandSpec::exec("pkg", ["update".to_string(), "-q".to_string()]).privileged(),
            )
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pkg update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("BSD pkg package index refreshed");
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::info!("discovering BSD pkg-managed software");

        let packages: Vec<DiscoveredSoftware> = match self.config.effective_filter() {
            PkgDiscoveryFilter::All => {
                // Query all installed packages.
                let cmd_output = self
                    .executor
                    .execute_quiet(&CommandSpec::exec(
                        "pkg",
                        ["query".to_string(), "-a".to_string(), "%n\t%v".to_string()],
                    ))
                    .await
                    .map_err(|e| {
                        report!(PluginError::PluginInternal(format!(
                            "pkg query failed: {e}"
                        )))
                    })?;

                if cmd_output.exit_code != 0 {
                    bail!(PluginError::CommandFailed(cmd_output.exit_code));
                }

                Self::parse_pkg_query_line(&cmd_output.output)
                    .into_iter()
                    .map(|(name, version)| build_discovered(name, version))
                    .collect()
            }
            PkgDiscoveryFilter::Manual => {
                // Query all installed packages with the auto-install flag.
                let cmd_output = self
                    .executor
                    .execute_quiet(&CommandSpec::exec(
                        "pkg",
                        [
                            "query".to_string(),
                            "-a".to_string(),
                            "%a\t%n\t%v".to_string(),
                        ],
                    ))
                    .await
                    .map_err(|e| {
                        report!(PluginError::PluginInternal(format!(
                            "pkg query failed: {e}"
                        )))
                    })?;

                if cmd_output.exit_code != 0 {
                    bail!(PluginError::CommandFailed(cmd_output.exit_code));
                }

                Self::parse_pkg_query_with_auto_line(&cmd_output.output)
                    .into_iter()
                    .filter(|(auto_flag, _, _)| auto_flag == "0")
                    .map(|(_, name, version)| build_discovered(name, version))
                    .collect()
            }
        };

        tracing::debug!(
            count = packages.len(),
            "BSD pkg software discovery complete"
        );
        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting BSD pkg installed version");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pkg",
                [
                    "query".to_string(),
                    "%v".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pkg query failed: {e}"
                )))
            })?;

        match cmd_output.exit_code {
            0 => {
                let version = cmd_output.output.trim().to_string();
                if version.is_empty() {
                    return Ok(None);
                }
                tracing::debug!(version = %version, "BSD pkg installed version detected");
                Ok(Some(Version::new(&version)))
            }
            // Exit code 70 means no packages matched.
            70 => {
                tracing::debug!(
                    package = %package_identifier,
                    "package not found in pkg database"
                );
                Ok(None)
            }
            code => bail!(PluginError::CommandFailed(code)),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching BSD pkg releases via pkg rquery");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pkg",
                [
                    "rquery".to_string(),
                    "%v".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pkg rquery failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            // rquery returns non-zero when the package is not in the repo.
            return Ok(vec![]);
        }

        let version = cmd_output.output.trim().to_string();
        if version.is_empty() {
            return Ok(vec![]);
        }

        tracing::debug!(version = %version, "BSD pkg upstream version resolved");

        Ok(vec![{
            let mut r = UpstreamRelease::new(Version::new(&version), version, false, "");
            r.category = Some(UpdateCategory::Unknown);
            r
        }])
    }

    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&uptrakit_plugin_infrastructure_core::ReleaseInfo>,
        output_tx: &mpsc::Sender<uptrakit_plugin_infrastructure_core::UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;

        let args = vec![
            "install".to_string(),
            "-y".to_string(),
            package_identifier.to_string(),
        ];

        tracing::debug!(
            package = %package_identifier,
            "running pkg install"
        );

        let display_args = std::iter::once("pkg")
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
            .execute(&CommandSpec::exec("pkg", args).privileged(), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::InstallFailed(format!(
                "pkg install failed with exit code {}",
                cmd_output.exit_code
            )));
        }

        output.push_str(&cmd_output.output);
        Ok(output)
    }

    /// Detect installed versions for multiple packages using a single `pkg query` call.
    ///
    /// Runs:
    /// ```text
    /// pkg query -a "%n\t%v"
    /// ```
    ///
    /// Then filters the output in memory to match the requested packages.
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> Result<Vec<BatchDetectResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting BSD pkg installed versions"
        );

        let stdout = match self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "pkg",
                ["query".to_string(), "-a".to_string(), "%n\t%v".to_string()],
            ))
            .await
        {
            Ok(o) => o.output,
            Err(e) => {
                let error_str = format!("pkg query failed: {e}");
                return Ok(items
                    .iter()
                    .map(|item| {
                        BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                    })
                    .collect());
            }
        };

        let pkg_map: HashMap<String, String> =
            Self::parse_pkg_query_line(&stdout).into_iter().collect();

        let results = items
            .iter()
            .map(|item| {
                let installed_version = pkg_map.get(&item.package_identifier).map(Version::new);
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(
            count = items.len(),
            "BSD pkg batch version detection complete"
        );
        Ok(results)
    }

    /// Fetch available releases for multiple packages using a single `pkg rquery` call.
    ///
    /// Runs:
    /// ```text
    /// pkg rquery "%n\t%v" pkg1 pkg2 ...
    /// ```
    ///
    /// Output lines are matched by package name; packages absent from the output
    /// have empty releases.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch_releases(
        &self,
        items: &[BatchFetchItem],
    ) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["rquery".to_string(), "%n\t%v".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch fetching BSD pkg releases via pkg rquery"
        );

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("pkg", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "pkg rquery failed: {e}"
                )))
            })?;

        // rquery exits non-zero when no packages are found; treat output as best-effort.
        let parsed: HashMap<String, String> = Self::parse_pkg_query_line(&cmd_output.output)
            .into_iter()
            .collect();

        let results = items
            .iter()
            .map(|item| {
                let Some(version) = parsed.get(&item.package_identifier) else {
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                BatchFetchResult::found(
                    item.package_identifier.clone(),
                    vec![{
                        let mut r =
                            UpstreamRelease::new(Version::new(version), version.clone(), false, "");
                        r.category = Some(UpdateCategory::Unknown);
                        r
                    }],
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "BSD pkg batch fetch complete");
        Ok(results)
    }

    /// Execute batch updates using a single `pkg install -y` call.
    ///
    /// Runs:
    /// ```text
    /// pkg install -y pkg1 pkg2 ...
    /// ```
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<uptrakit_plugin_infrastructure_core::UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            validate_identifier(&item.package_identifier)
                .map_err(|e| report!(PluginError::Configuration(e)))?;
        }

        let mut args = vec!["install".to_string(), "-y".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let pkg_list: Vec<&str> = items
            .iter()
            .map(|i| i.package_identifier.as_str())
            .collect();

        let display_args = std::iter::once("pkg")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");

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
            "running BSD pkg batch install"
        );

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("pkg", args).privileged(), output_tx)
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

/// Build a [`DiscoveredSoftware`] entry for a package.
fn build_discovered(name: String, version: String) -> DiscoveredSoftware {
    let targets = vec![DiscoveryTarget {
        plugin_type: PluginType::PackageManagerPkg,
        plugin_config: serde_json::json!({}),
        plugin_config_name: "BSD pkg".to_string(),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    // ── validate_identifier ───────────────────────────────────────────────────

    #[test]
    fn validate_identifier_valid_curl() {
        assert!(validate_identifier("curl").is_ok());
    }

    #[test]
    fn validate_identifier_valid_python39() {
        assert!(validate_identifier("python39").is_ok());
    }

    #[test]
    fn validate_identifier_valid_nginx() {
        assert!(validate_identifier("nginx").is_ok());
    }

    #[test]
    fn validate_identifier_valid_php82_extensions() {
        assert!(validate_identifier("php82-extensions").is_ok());
    }

    #[test]
    fn validate_identifier_valid_p5_net_sslkey() {
        assert!(validate_identifier("p5-Net-SSLeay").is_ok());
    }

    #[test]
    fn validate_identifier_valid_libstdcxx() {
        assert!(validate_identifier("libstdc++").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_dot() {
        assert!(validate_identifier("python3.11").is_ok());
    }

    #[test]
    fn validate_identifier_valid_with_underscore() {
        assert!(validate_identifier("py39_setuptools").is_ok());
    }

    #[test]
    fn validate_identifier_valid_single_char() {
        // Single alphanumeric is valid (no minimum length for pkg).
        assert!(validate_identifier("a").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_empty() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_rejects_too_long() {
        let long = "a".repeat(201);
        assert!(validate_identifier(&long).is_err());
    }

    #[test]
    fn validate_identifier_accepts_max_length() {
        let max = "a".repeat(200);
        assert!(validate_identifier(&max).is_ok());
    }

    #[test]
    fn validate_identifier_rejects_invalid_chars() {
        assert!(validate_identifier("nginx/config").is_err());
        assert!(validate_identifier("nginx@latest").is_err());
        assert!(validate_identifier("pkg name").is_err());
    }

    #[test]
    fn validate_identifier_rejects_leading_hyphen() {
        assert!(validate_identifier("-nginx").is_err());
    }

    #[test]
    fn validate_identifier_rejects_leading_dot() {
        assert!(validate_identifier(".nginx").is_err());
    }

    #[test]
    fn validate_identifier_rejects_trailing_hyphen() {
        assert!(validate_identifier("nginx-").is_err());
    }

    #[test]
    fn validate_identifier_rejects_trailing_dot() {
        assert!(validate_identifier("nginx.").is_err());
    }

    #[test]
    fn validate_identifier_rejects_path_traversal() {
        assert!(validate_identifier("pkg..conf").is_err());
        assert!(validate_identifier("../etc/passwd").is_err());
    }

    // ── parse_pkg_query_line ──────────────────────────────────────────────────

    #[test]
    fn parse_pkg_query_line_basic() {
        let output = "curl\t7.87.0\nnginx\t1.24.0\n";
        let result = PkgPlugin::parse_pkg_query_line(output);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], ("curl".to_string(), "7.87.0".to_string()));
        assert_eq!(result[1], ("nginx".to_string(), "1.24.0".to_string()));
    }

    #[test]
    fn parse_pkg_query_line_skips_empty_lines() {
        let output = "curl\t7.87.0\n\nnginx\t1.24.0\n";
        let result = PkgPlugin::parse_pkg_query_line(output);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn parse_pkg_query_line_skips_missing_version() {
        let output = "curl\t\nnginx\t1.24.0\n";
        let result = PkgPlugin::parse_pkg_query_line(output);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "nginx");
    }

    #[test]
    fn parse_pkg_query_line_empty_output() {
        let result = PkgPlugin::parse_pkg_query_line("");
        assert!(result.is_empty());
    }

    // ── parse_pkg_query_with_auto_line ────────────────────────────────────────

    #[test]
    fn parse_pkg_query_with_auto_line_basic() {
        let output = "0\tcurl\t7.87.0\n1\tlibbsd\t0.11.7\n0\tnginx\t1.24.0\n";
        let result = PkgPlugin::parse_pkg_query_with_auto_line(output);
        assert_eq!(result.len(), 3);
        assert_eq!(
            result[0],
            ("0".to_string(), "curl".to_string(), "7.87.0".to_string())
        );
        assert_eq!(
            result[1],
            ("1".to_string(), "libbsd".to_string(), "0.11.7".to_string())
        );
    }

    #[test]
    fn parse_pkg_query_with_auto_line_filters_manual() {
        let output = "0\tcurl\t7.87.0\n1\tlibbsd\t0.11.7\n0\tnginx\t1.24.0\n";
        let manual: Vec<_> = PkgPlugin::parse_pkg_query_with_auto_line(output)
            .into_iter()
            .filter(|(flag, _, _)| flag == "0")
            .collect();
        assert_eq!(manual.len(), 2);
        assert_eq!(manual[0].1, "curl");
        assert_eq!(manual[1].1, "nginx");
    }

    #[test]
    fn parse_pkg_query_with_auto_line_skips_incomplete() {
        let output = "0\tcurl\n1\tnginx\t1.24.0\n";
        let result = PkgPlugin::parse_pkg_query_with_auto_line(output);
        // First line has no version field (only 2 tab-separated parts).
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].1, "nginx");
    }

    #[test]
    fn parse_pkg_query_with_auto_line_empty_output() {
        let result = PkgPlugin::parse_pkg_query_with_auto_line("");
        assert!(result.is_empty());
    }

    // ── required_sudo_commands ────────────────────────────────────────────────

    #[tokio::test]
    async fn pkg_plugin_required_sudo_commands() {
        let plugin = PkgPlugin::new(PkgConfig::default(), test_executor())
            .await
            .expect("create plugin");
        let entries = plugin.required_sudo_commands();
        assert_eq!(entries.len(), 2);
        assert!(entries.iter().all(|e| e.command == "pkg"));
        assert!(entries.iter().all(|e| !e.needs_setenv));
        assert_eq!(entries[0].args_suffix.as_deref(), Some("update *"));
        assert_eq!(entries[1].args_suffix.as_deref(), Some("install -y *"));
    }
}
