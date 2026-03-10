use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, BatchFetchItem, BatchFetchResult, BatchUpdateItem,
    BatchUpdateResult, DiscoveredSoftware, DiscoveryTarget, HostCompatibility, OutputStreamType,
    Plugin, PluginCapability, PluginError, PluginRole, PluginType, ReleaseInfo, Result,
    UpdateOutputLine, UpstreamRelease, Version,
};

use crate::config::{HomebrewConfig, HomebrewPackageType};

/// Validate a Homebrew package identifier.
///
/// Rejects empty values, leading/trailing whitespace, embedded whitespace, path-traversal
/// segments (`..`, `.`), empty path segments (`//`), and any characters outside the
/// allowed set `[A-Za-z0-9\-_.@+/]`.
pub fn validate_identifier(value: &str) -> std::result::Result<(), String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err("package_identifier must not be empty".to_string());
    }
    if value != trimmed {
        return Err(
            "package_identifier must not include leading or trailing whitespace".to_string(),
        );
    }
    if value.chars().any(char::is_whitespace) {
        return Err("package_identifier must not contain whitespace".to_string());
    }
    if value.len() > 200 {
        return Err("package_identifier is too long".to_string());
    }
    for ch in value.chars() {
        let valid = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@' | '+' | '/');
        if !valid {
            return Err(format!(
                "package_identifier contains invalid character: {ch}"
            ));
        }
    }
    if value.split('/').any(|segment| segment.is_empty()) {
        return Err("package_identifier contains an empty segment".to_string());
    }
    if value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("package_identifier contains invalid segment".to_string());
    }
    Ok(())
}

/// Plugin for Homebrew (macOS/Linux package manager).
///
/// Supports both formulae and casks. The `package_identifier` in `SoftwareItem`
/// is the Homebrew formula/cask name (e.g., `wget`, `firefox`).
pub struct HomebrewPlugin {
    config: HomebrewConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl HomebrewPlugin {
    /// Compile-time capabilities for the Homebrew plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::RefreshPackageIndex,
        PluginCapability::DetectHostCompatibility,
    ];

    /// Create a new Homebrew plugin with the given configuration.
    pub async fn new(config: HomebrewConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Parse the installed version from `brew info --json=v2` output for a
    /// specific package.
    fn parse_installed_version(
        json: &serde_json::Value,
        pkg: &str,
        is_cask: bool,
    ) -> Option<String> {
        if is_cask {
            let casks = json.get("casks")?.as_array()?;
            let cask = casks
                .iter()
                .find(|c| c.get("token").and_then(|t| t.as_str()) == Some(pkg))?;
            cask.get("installed")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        } else {
            let formulae = json.get("formulae")?.as_array()?;
            let formula = formulae
                .iter()
                .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(pkg))?;
            let installed = formula.get("installed")?.as_array()?;
            installed
                .first()?
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
    }

    /// Parse the latest available version from `brew info --json=v2` output for
    /// a specific package.
    fn parse_latest_version(json: &serde_json::Value, pkg: &str, is_cask: bool) -> Option<String> {
        if is_cask {
            let casks = json.get("casks")?.as_array()?;
            let cask = casks
                .iter()
                .find(|c| c.get("token").and_then(|t| t.as_str()) == Some(pkg))?;
            cask.get("version")
                .and_then(|v| v.as_str())
                .filter(|v| *v != "latest")
                .map(|s| s.to_string())
        } else {
            let formulae = json.get("formulae")?.as_array()?;
            let formula = formulae
                .iter()
                .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(pkg))?;
            let versions = formula.get("versions")?;
            versions
                .get("stable")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
    }

    /// Parse installed formulae from `brew info --installed --json=v2` output.
    ///
    /// Emits items only for packages with a known installed version.
    /// When `emit_targets` is true, each item carries a `DiscoveryTarget` with
    /// `{"package_type": "formula"}` config so the controller can auto-create
    /// the correct Homebrew plugin config.
    fn parse_installed_formulae(
        json: &serde_json::Value,
        emit_targets: bool,
    ) -> Vec<DiscoveredSoftware> {
        let mut result = Vec::new();
        if let Some(formulae) = json.get("formulae").and_then(|f| f.as_array()) {
            for formula in formulae {
                let Some(name) = formula.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                let full_name = formula
                    .get("full_name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(name);
                let Some(installed_version) = formula
                    .get("installed")
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|obj| obj.get("version"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                else {
                    // Skip packages without a known installed version.
                    continue;
                };

                // Formulae pinned to "latest" have no deterministic version
                // and cannot be meaningfully tracked or upgraded.
                if installed_version == "latest" {
                    tracing::debug!(name, "skipping formula with version=latest from discovery");
                    continue;
                }

                let targets = if emit_targets {
                    vec![DiscoveryTarget {
                        plugin_type: PluginType::PackageManagerHomebrew,
                        plugin_config: serde_json::json!({"package_type": "formula"}),
                        plugin_config_name: "Homebrew (Formulae)".to_string(),
                        roles: vec![
                            PluginRole::DetectVersion,
                            PluginRole::FetchReleases,
                            PluginRole::ExecuteUpdate,
                        ],
                        package_identifier: None,
                        config_override: None,
                        execution_site: None,
                    }]
                } else {
                    vec![]
                };

                result.push(DiscoveredSoftware {
                    package_identifier: name.to_string(),
                    name: full_name.to_string(),
                    installed_version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                });
            }
        }
        result
    }

    /// Parse installed casks from `brew info --installed --json=v2` output.
    ///
    /// Emits items only for packages with a known installed version.
    /// When `emit_targets` is true, each item carries a `DiscoveryTarget` with
    /// `{"package_type": "cask"}` config so the controller can auto-create
    /// the correct Homebrew plugin config.
    fn parse_installed_casks(
        json: &serde_json::Value,
        emit_targets: bool,
    ) -> Vec<DiscoveredSoftware> {
        let mut result = Vec::new();
        if let Some(casks) = json.get("casks").and_then(|c| c.as_array()) {
            for cask in casks {
                let Some(token) = cask.get("token").and_then(|t| t.as_str()) else {
                    continue;
                };
                let name = cask
                    .get("name")
                    .and_then(|n| n.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|n| n.as_str())
                    .unwrap_or(token);
                let Some(installed_version) = cask
                    .get("installed")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                else {
                    // Skip casks without a known installed version.
                    continue;
                };

                // Casks with version "latest" have no deterministic version
                // and cannot be meaningfully tracked or upgraded.
                if installed_version == "latest" {
                    tracing::debug!(token, "skipping cask with version=latest from discovery");
                    continue;
                }

                // Casks with `auto_updates: true` manage their own update
                // mechanism (e.g. Google Chrome) and cannot be upgraded via
                // `brew upgrade`. Exclude them from discovery so they don't
                // appear in the UI.
                if cask
                    .get("auto_updates")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    tracing::debug!(
                        token,
                        "skipping auto-updating cask from discovery (auto_updates=true)"
                    );
                    continue;
                }

                let targets = if emit_targets {
                    vec![DiscoveryTarget {
                        plugin_type: PluginType::PackageManagerHomebrew,
                        plugin_config: serde_json::json!({"package_type": "cask"}),
                        plugin_config_name: "Homebrew (Casks)".to_string(),
                        roles: vec![
                            PluginRole::DetectVersion,
                            PluginRole::FetchReleases,
                            PluginRole::ExecuteUpdate,
                        ],
                        package_identifier: None,
                        config_override: None,
                        execution_site: None,
                    }]
                } else {
                    vec![]
                };

                result.push(DiscoveredSoftware {
                    package_identifier: token.to_string(),
                    name: name.to_string(),
                    installed_version,
                    targets,
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                });
            }
        }
        result
    }

    /// Returns `true` if this instance is configured to track casks.
    ///
    /// Returns `false` for `None` (discover-all) and formula configs, so
    /// version-check operations default to formula behaviour when no explicit
    /// type is set.
    fn is_cask(&self) -> bool {
        matches!(self.config.package_type, Some(HomebrewPackageType::Cask))
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        if package_identifier.trim().is_empty() {
            bail!(PluginError::Configuration(
                "package_identifier must not be empty".to_string()
            ));
        }
        Ok(())
    }

    /// Find the homepage URL of a formula by name in `brew info --json=v2` output.
    fn find_formula_homepage(json: &serde_json::Value, pkg_id: &str) -> String {
        json.get("formulae")
            .and_then(|f| f.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(pkg_id))
            })
            .and_then(|f| f.get("homepage"))
            .and_then(|h| h.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Find the homepage URL of a cask by token in `brew info --json=v2` output.
    fn find_cask_homepage(json: &serde_json::Value, pkg_id: &str) -> String {
        json.get("casks")
            .and_then(|c| c.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("token").and_then(|t| t.as_str()) == Some(pkg_id))
            })
            .and_then(|c| c.get("homepage"))
            .and_then(|h| h.as_str())
            .unwrap_or("")
            .to_string()
    }
}

#[async_trait]
impl Plugin for HomebrewPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::PackageManagerHomebrew
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }

    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
        match self
            .executor
            .execute_quiet(&CommandSpec::exec("which", ["brew".to_string()]))
            .await
        {
            Ok(_) => Ok(HostCompatibility::Compatible),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "brew not found".to_string(),
            )),
        }
    }

    #[tracing::instrument(skip_all)]
    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Homebrew package index");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", ["update".to_string()]))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("Homebrew package index refreshed");
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "brew",
                [
                    "info".to_string(),
                    "--installed".to_string(),
                    "--json=v2".to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let packages = match &self.config.package_type {
            None => {
                // Discover-all mode: return both formulae and casks, each tagged
                // with extra metadata so the controller can route them to the
                // correct auto-created plugin configs.
                tracing::debug!("discovering all installed Homebrew packages (formulae + casks)");
                let mut all = Self::parse_installed_formulae(&json, true);
                all.extend(Self::parse_installed_casks(&json, true));
                all
            }
            Some(HomebrewPackageType::Formula) => {
                tracing::debug!("discovering installed Homebrew formulae");
                Self::parse_installed_formulae(&json, false)
            }
            Some(HomebrewPackageType::Cask) => {
                tracing::debug!("discovering installed Homebrew casks");
                Self::parse_installed_casks(&json, false)
            }
        };

        Ok(packages)
    }

    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "detecting installed Homebrew version");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "brew",
                [
                    "info".to_string(),
                    "--json=v2".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let version = Self::parse_installed_version(&json, package_identifier, self.is_cask())
            .map(|v| Version::new(&v));
        tracing::debug!(version = ?version, "Homebrew version detection result");
        Ok(version)
    }

    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
        tracing::debug!(package = %package_identifier, "fetching Homebrew releases");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec(
                "brew",
                [
                    "info".to_string(),
                    "--json=v2".to_string(),
                    package_identifier.to_string(),
                ],
            ))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let Some(version_str) =
            Self::parse_latest_version(&json, package_identifier, self.is_cask())
        else {
            return Ok(vec![]);
        };

        let homepage = if self.is_cask() {
            json.get("casks")
                .and_then(|c| c.as_array())
                .and_then(|arr| arr.first())
                .and_then(|c| c.get("homepage"))
                .and_then(|h| h.as_str())
                .unwrap_or("")
        } else {
            json.get("formulae")
                .and_then(|f| f.as_array())
                .and_then(|arr| arr.first())
                .and_then(|f| f.get("homepage"))
                .and_then(|h| h.as_str())
                .unwrap_or("")
        };

        let releases = vec![{
            let mut r = UpstreamRelease::new(Version::new(&version_str), version_str, false, "");
            r.release_url = homepage.to_string();
            r
        }];
        tracing::debug!(count = releases.len(), "Homebrew releases fetched");
        Ok(releases)
    }

    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        _to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        self.require_package_identifier(package_identifier)?;
        let pkg = package_identifier;
        let mut output = String::new();

        let args: Vec<String> = if self.is_cask() {
            vec!["upgrade".to_string(), "--cask".to_string(), pkg.to_string()]
        } else {
            vec!["upgrade".to_string(), pkg.to_string()]
        };

        tracing::debug!(package = %pkg, "running brew upgrade");
        send_output(
            output_tx,
            &format!("Running: brew {}", args.join(" ")),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Running: brew {}\n", args.join(" ")));

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("brew", args), output_tx)
            .await
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        Ok(output)
    }

    /// Execute batch updates using a single `brew upgrade pkg1 pkg2 ...` command.
    #[tracing::instrument(skip_all)]
    async fn execute_batch_update(
        &self,
        items: &[BatchUpdateItem],
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<Vec<BatchUpdateResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        for item in items {
            self.require_package_identifier(&item.package_identifier)?;
        }

        let mut args: Vec<String> = vec!["upgrade".to_string()];
        if self.is_cask() {
            args.push("--cask".to_string());
        }
        for item in items {
            args.push(item.package_identifier.clone());
        }

        let display_cmd = format!("brew {}", args.join(" "));
        send_output(
            output_tx,
            &format!(
                "Batch updating {} packages\nRunning: {display_cmd}",
                items.len()
            ),
            OutputStreamType::Stdout,
        )
        .await;
        let mut output = format!("Running: {display_cmd}\n");

        tracing::debug!(count = items.len(), "running brew batch upgrade");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("brew", args), output_tx)
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

    /// Detect installed versions for multiple packages using a single `brew info` call.
    ///
    /// Runs:
    /// ```text
    /// brew info --json=v2 pkg1 pkg2 pkg3
    /// ```
    ///
    /// Parses the returned JSON once and looks up each package individually using the
    /// existing [`parse_installed_version`](Self::parse_installed_version) helper. If
    /// the command fails, all items receive the same error.
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

        let mut args = vec!["info".to_string(), "--json=v2".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(
            count = items.len(),
            "batch detecting Homebrew installed versions"
        );

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            // Fail all items with the same error.
            let error_str = format!("brew info exited with code {}", cmd_output.exit_code);
            return Ok(items
                .iter()
                .map(|item| {
                    BatchDetectResult::error(item.package_identifier.clone(), error_str.clone())
                })
                .collect());
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let is_cask = self.is_cask();
        let results = items
            .iter()
            .map(|item| {
                let installed_version =
                    Self::parse_installed_version(&json, &item.package_identifier, is_cask)
                        .map(|v| Version::new(&v));
                BatchDetectResult::new(item.package_identifier.clone(), installed_version, None)
            })
            .collect();

        tracing::debug!(
            count = items.len(),
            "Homebrew batch version detection complete"
        );
        Ok(results)
    }

    /// Fetch available releases for multiple packages using a single `brew info` call.
    ///
    /// Runs:
    /// ```text
    /// brew info --json=v2 pkg1 pkg2 pkg3
    /// ```
    ///
    /// Parses the returned JSON once and resolves the latest version and homepage for
    /// each package individually. If the command fails, all items receive the same error.
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

        let mut args = vec!["info".to_string(), "--json=v2".to_string()];
        for item in items {
            args.push(item.package_identifier.clone());
        }

        tracing::debug!(count = items.len(), "batch fetching Homebrew releases");

        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", args))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            let error_str = format!("brew info exited with code {}", cmd_output.exit_code);
            return Ok(items
                .iter()
                .map(|item| {
                    BatchFetchResult::error(item.package_identifier.clone(), error_str.clone())
                })
                .collect());
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::PluginInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let is_cask = self.is_cask();
        let results = items
            .iter()
            .map(|item| {
                let Some(version_str) =
                    Self::parse_latest_version(&json, &item.package_identifier, is_cask)
                else {
                    return BatchFetchResult::empty(item.package_identifier.clone());
                };

                let homepage = if is_cask {
                    Self::find_cask_homepage(&json, &item.package_identifier)
                } else {
                    Self::find_formula_homepage(&json, &item.package_identifier)
                };

                BatchFetchResult::found(
                    item.package_identifier.clone(),
                    vec![{
                        let mut r = UpstreamRelease::new(
                            Version::new(&version_str),
                            version_str,
                            false,
                            "",
                        );
                        r.release_url = homepage;
                        r
                    }],
                )
            })
            .collect();

        tracing::debug!(count = items.len(), "Homebrew batch fetch complete");
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{CommandOutput, LocalCommandExecutor};

    // ── Sample `brew info --json=v2` output for a formula ───────────────

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn sample_formula_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [{
                "name": "wget",
                "full_name": "wget",
                "versions": {
                    "stable": "1.24.5",
                    "head": null
                },
                "installed": [{
                    "version": "1.24.4",
                    "installed_as_dependency": false
                }],
                "homepage": "https://www.gnu.org/software/wget/"
            }],
            "casks": []
        })
    }

    fn sample_formula_json_not_installed() -> serde_json::Value {
        serde_json::json!({
            "formulae": [{
                "name": "wget",
                "full_name": "wget",
                "versions": {
                    "stable": "1.24.5",
                    "head": null
                },
                "installed": [],
                "homepage": "https://www.gnu.org/software/wget/"
            }],
            "casks": []
        })
    }

    fn sample_cask_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "firefox",
                "name": ["Mozilla Firefox"],
                "version": "133.0",
                "installed": "132.0",
                "homepage": "https://www.mozilla.org/firefox/"
            }]
        })
    }

    fn sample_cask_json_not_installed() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "firefox",
                "name": ["Mozilla Firefox"],
                "version": "133.0",
                "installed": null,
                "homepage": "https://www.mozilla.org/firefox/"
            }]
        })
    }

    fn sample_installed_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [
                {
                    "name": "wget",
                    "full_name": "wget",
                    "versions": { "stable": "1.24.5" },
                    "installed": [{ "version": "1.24.4" }]
                },
                {
                    "name": "jq",
                    "full_name": "jq",
                    "versions": { "stable": "1.7.1" },
                    "installed": [{ "version": "1.7.1" }]
                }
            ],
            "casks": [
                {
                    "token": "firefox",
                    "name": ["Mozilla Firefox"],
                    "version": "133.0",
                    "installed": "132.0"
                }
            ]
        })
    }

    fn sample_cask_latest_version_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "google-chrome",
                "name": ["Google Chrome"],
                "version": "latest",
                "installed": "latest",
                "homepage": "https://www.google.com/chrome/"
            }]
        })
    }

    // ── parse_installed_version ─────────────────────────────────────────

    #[test]
    fn parse_installed_version_formula() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_installed_version(&json, "wget", false);
        assert_eq!(version, Some("1.24.4".to_string()));
    }

    #[test]
    fn parse_installed_version_formula_not_installed() {
        let json = sample_formula_json_not_installed();
        let version = HomebrewPlugin::parse_installed_version(&json, "wget", false);
        assert_eq!(version, None);
    }

    #[test]
    fn parse_installed_version_cask() {
        let json = sample_cask_json();
        let version = HomebrewPlugin::parse_installed_version(&json, "firefox", true);
        assert_eq!(version, Some("132.0".to_string()));
    }

    #[test]
    fn parse_installed_version_cask_not_installed() {
        let json = sample_cask_json_not_installed();
        let version = HomebrewPlugin::parse_installed_version(&json, "firefox", true);
        assert_eq!(version, None);
    }

    #[test]
    fn parse_installed_version_unknown_package() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_installed_version(&json, "nonexistent", false);
        assert_eq!(version, None);
    }

    // ── parse_latest_version ────────────────────────────────────────────

    #[test]
    fn parse_latest_version_formula() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "wget", false);
        assert_eq!(version, Some("1.24.5".to_string()));
    }

    #[test]
    fn parse_latest_version_cask() {
        let json = sample_cask_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "firefox", true);
        assert_eq!(version, Some("133.0".to_string()));
    }

    #[test]
    fn parse_latest_version_cask_with_latest_marker() {
        let json = sample_cask_latest_version_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "google-chrome", true);
        // "latest" is filtered out — not a useful version string
        assert_eq!(version, None);
    }

    #[test]
    fn parse_latest_version_unknown_package() {
        let json = sample_formula_json();
        let version = HomebrewPlugin::parse_latest_version(&json, "nonexistent", false);
        assert_eq!(version, None);
    }

    // ── parse_installed_formulae / parse_installed_casks ────────────────

    #[test]
    fn parse_installed_formulae_without_targets() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_formulae(&json, false);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].package_identifier, "wget");
        assert_eq!(packages[0].name, "wget");
        assert_eq!(packages[0].installed_version, "1.24.4");
        assert!(packages[0].targets.is_empty());
        assert_eq!(packages[1].package_identifier, "jq");
        assert_eq!(packages[1].installed_version, "1.7.1");
    }

    #[test]
    fn parse_installed_formulae_with_targets() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_formulae(&json, true);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].targets.len(), 1);
        assert_eq!(
            packages[0].targets[0].plugin_type,
            PluginType::PackageManagerHomebrew
        );
        assert_eq!(
            packages[0].targets[0].plugin_config,
            serde_json::json!({"package_type": "formula"})
        );
        assert_eq!(
            packages[0].targets[0].plugin_config_name,
            "Homebrew (Formulae)"
        );
        assert_eq!(packages[0].targets[0].roles.len(), 3);
    }

    #[test]
    fn parse_installed_casks_without_targets() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "firefox");
        assert_eq!(packages[0].name, "Mozilla Firefox");
        assert_eq!(packages[0].installed_version, "132.0");
        assert!(packages[0].targets.is_empty());
    }

    #[test]
    fn parse_installed_casks_with_targets() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_casks(&json, true);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].targets.len(), 1);
        assert_eq!(
            packages[0].targets[0].plugin_type,
            PluginType::PackageManagerHomebrew
        );
        assert_eq!(
            packages[0].targets[0].plugin_config,
            serde_json::json!({"package_type": "cask"})
        );
        assert_eq!(
            packages[0].targets[0].plugin_config_name,
            "Homebrew (Casks)"
        );
        assert_eq!(packages[0].targets[0].roles.len(), 3);
    }

    #[test]
    fn parse_installed_casks_skips_not_installed() {
        let json = sample_cask_json_not_installed();
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        assert!(packages.is_empty());
    }

    #[test]
    fn parse_installed_casks_skips_auto_updates_true() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [
                {
                    "token": "google-chrome",
                    "name": ["Google Chrome"],
                    "version": "130.0",
                    "installed": "129.0",
                    "auto_updates": true
                },
                {
                    "token": "firefox",
                    "name": ["Mozilla Firefox"],
                    "version": "133.0",
                    "installed": "132.0",
                    "auto_updates": false
                }
            ]
        });
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        // google-chrome (auto_updates=true) is excluded; firefox (auto_updates=false) is included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "firefox");
    }

    #[test]
    fn parse_installed_casks_includes_cask_without_auto_updates_field() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "iterm2",
                "name": ["iTerm2"],
                "version": "3.5.0",
                "installed": "3.4.23"
            }]
        });
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        // No auto_updates field → defaults to false → included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "iterm2");
    }

    #[test]
    fn parse_installed_casks_skips_latest_version() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [
                {
                    "token": "some-cask",
                    "name": ["Some Cask"],
                    "version": "latest",
                    "installed": "latest"
                },
                {
                    "token": "iterm2",
                    "name": ["iTerm2"],
                    "version": "3.5.0",
                    "installed": "3.4.23"
                }
            ]
        });
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        // "latest" cask excluded; iterm2 included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "iterm2");
    }

    #[test]
    fn parse_installed_formulae_skips_latest_version() {
        let json = serde_json::json!({
            "formulae": [
                {
                    "name": "some-formula",
                    "full_name": "some-formula",
                    "versions": { "stable": "latest" },
                    "installed": [{ "version": "latest" }]
                },
                {
                    "name": "wget",
                    "full_name": "wget",
                    "versions": { "stable": "1.24.5" },
                    "installed": [{ "version": "1.24.4" }]
                }
            ],
            "casks": []
        });
        let packages = HomebrewPlugin::parse_installed_formulae(&json, false);
        // "latest" formula excluded; wget included.
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "wget");
    }

    #[test]
    fn parse_installed_packages_empty() {
        let json = serde_json::json!({"formulae": [], "casks": []});
        let packages = HomebrewPlugin::parse_installed_formulae(&json, false);
        assert!(packages.is_empty());
    }

    // ── Mock executor ────────────────────────────────────────────────────

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

    // ── Plugin trait ──────────────────────────────────────────────────

    #[tokio::test]
    async fn homebrew_plugin_capabilities() {
        let plugin = HomebrewPlugin::new(HomebrewConfig::default(), test_executor())
            .await
            .expect("create");
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(plugin.has_capability(PluginCapability::RefreshPackageIndex));
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
        assert_eq!(plugin.capabilities().len(), 3);
    }

    #[tokio::test]
    async fn is_cask_returns_false_for_none() {
        let plugin = HomebrewPlugin::new(HomebrewConfig { package_type: None }, test_executor())
            .await
            .expect("create");
        assert!(!plugin.is_cask());
    }

    #[tokio::test]
    async fn is_cask_returns_true_for_cask() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Cask),
            },
            test_executor(),
        )
        .await
        .expect("create");
        assert!(plugin.is_cask());
    }

    #[tokio::test]
    async fn is_cask_returns_false_for_formula() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Formula),
            },
            test_executor(),
        )
        .await
        .expect("create");
        assert!(!plugin.is_cask());
    }

    #[tokio::test]
    async fn homebrew_plugin_detect_installed_empty_identifier_fails() {
        let plugin = HomebrewPlugin::new(HomebrewConfig::default(), test_executor())
            .await
            .expect("create");
        let result = plugin.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn homebrew_plugin_fetch_releases_empty_identifier_fails() {
        let plugin = HomebrewPlugin::new(HomebrewConfig::default(), test_executor())
            .await
            .expect("create");
        let result = plugin.fetch_releases("").await;
        assert!(result.is_err());
    }

    // ── detect_host_compatibility ────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_which_exits_zero() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig::default(),
            FixedExitCodeExecutor::with_exit_code(0),
        )
        .await
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_which_exits_nonzero() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig::default(),
            FixedExitCodeExecutor::with_exit_code(1),
        )
        .await
        .expect("create");
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert_eq!(msg, "brew not found");
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    // ── find_formula_homepage / find_cask_homepage ───────────────────────

    #[test]
    fn find_formula_homepage_returns_correct_url() {
        let json = sample_formula_json();
        let homepage = HomebrewPlugin::find_formula_homepage(&json, "wget");
        assert_eq!(homepage, "https://www.gnu.org/software/wget/");
    }

    #[test]
    fn find_formula_homepage_unknown_package_returns_empty() {
        let json = sample_formula_json();
        let homepage = HomebrewPlugin::find_formula_homepage(&json, "nonexistent");
        assert!(homepage.is_empty());
    }

    #[test]
    fn find_cask_homepage_returns_correct_url() {
        let json = sample_cask_json();
        let homepage = HomebrewPlugin::find_cask_homepage(&json, "firefox");
        assert_eq!(homepage, "https://www.mozilla.org/firefox/");
    }

    #[test]
    fn find_cask_homepage_unknown_package_returns_empty() {
        let json = sample_cask_json();
        let homepage = HomebrewPlugin::find_cask_homepage(&json, "nonexistent");
        assert!(homepage.is_empty());
    }

    // ── batch_detect_installed_version ───────────────────────────────────

    /// Mock executor that returns a specific JSON payload for brew info.
    struct BrewInfoExecutor {
        json: String,
    }

    impl BrewInfoExecutor {
        fn with_json(json: serde_json::Value) -> Arc<dyn CommandExecutor> {
            Arc::new(Self {
                json: json.to_string(),
            })
        }
    }

    #[async_trait]
    impl CommandExecutor for BrewInfoExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.json.clone(),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<CommandOutput> {
            Ok(CommandOutput {
                output: self.json.clone(),
                exit_code: 0,
            })
        }
    }

    fn multi_formula_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [
                {
                    "name": "wget",
                    "full_name": "wget",
                    "versions": { "stable": "1.24.5" },
                    "installed": [{ "version": "1.24.4" }],
                    "homepage": "https://www.gnu.org/software/wget/"
                },
                {
                    "name": "jq",
                    "full_name": "jq",
                    "versions": { "stable": "1.7.1" },
                    "installed": [{ "version": "1.7.1" }],
                    "homepage": "https://jqlang.github.io/jq/"
                },
                {
                    "name": "curl",
                    "full_name": "curl",
                    "versions": { "stable": "8.5.0" },
                    "installed": [],
                    "homepage": "https://curl.se/"
                }
            ],
            "casks": []
        })
    }

    fn multi_cask_json() -> serde_json::Value {
        serde_json::json!({
            "formulae": [],
            "casks": [
                {
                    "token": "firefox",
                    "name": ["Mozilla Firefox"],
                    "version": "133.0",
                    "installed": "132.0",
                    "homepage": "https://www.mozilla.org/firefox/"
                },
                {
                    "token": "google-chrome",
                    "name": ["Google Chrome"],
                    "version": "120.0",
                    "installed": null,
                    "homepage": "https://www.google.com/chrome/"
                }
            ]
        })
    }

    #[tokio::test]
    async fn batch_detect_installed_version_formulae() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Formula),
            },
            BrewInfoExecutor::with_json(multi_formula_json()),
        )
        .await
        .expect("create");

        let items = vec![
            BatchDetectItem::new("wget".to_string()),
            BatchDetectItem::new("jq".to_string()),
            BatchDetectItem::new("curl".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 3);

        let wget = results
            .iter()
            .find(|r| r.package_identifier == "wget")
            .unwrap();
        assert_eq!(wget.installed_version, Some(Version::new("1.24.4")));
        assert!(wget.error.is_none());

        let jq = results
            .iter()
            .find(|r| r.package_identifier == "jq")
            .unwrap();
        assert_eq!(jq.installed_version, Some(Version::new("1.7.1")));

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert!(
            curl.installed_version.is_none(),
            "curl has empty installed array"
        );
        assert!(curl.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_casks() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Cask),
            },
            BrewInfoExecutor::with_json(multi_cask_json()),
        )
        .await
        .expect("create");

        let items = vec![
            BatchDetectItem::new("firefox".to_string()),
            BatchDetectItem::new("google-chrome".to_string()),
        ];
        let results = plugin
            .batch_detect_installed_version(&items)
            .await
            .expect("ok");

        assert_eq!(results.len(), 2);

        let firefox = results
            .iter()
            .find(|r| r.package_identifier == "firefox")
            .unwrap();
        assert_eq!(firefox.installed_version, Some(Version::new("132.0")));

        let chrome = results
            .iter()
            .find(|r| r.package_identifier == "google-chrome")
            .unwrap();
        assert!(
            chrome.installed_version.is_none(),
            "chrome is not installed (installed: null)"
        );
        assert!(chrome.error.is_none());
    }

    #[tokio::test]
    async fn batch_detect_installed_version_empty_returns_empty() {
        let plugin = HomebrewPlugin::new(HomebrewConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin
            .batch_detect_installed_version(&[])
            .await
            .expect("ok");
        assert!(results.is_empty());
    }

    // ── batch_fetch_releases ─────────────────────────────────────────────

    #[tokio::test]
    async fn batch_fetch_releases_formulae() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Formula),
            },
            BrewInfoExecutor::with_json(multi_formula_json()),
        )
        .await
        .expect("create");

        let items = vec![
            BatchFetchItem::new("wget".to_string()),
            BatchFetchItem::new("jq".to_string()),
            BatchFetchItem::new("curl".to_string()),
        ];
        let results = plugin.batch_fetch_releases(&items).await.expect("ok");

        assert_eq!(results.len(), 3);

        let wget = results
            .iter()
            .find(|r| r.package_identifier == "wget")
            .unwrap();
        assert_eq!(wget.releases.len(), 1);
        assert_eq!(wget.releases[0].tag, "1.24.5");
        assert_eq!(
            wget.releases[0].release_url,
            "https://www.gnu.org/software/wget/"
        );
        assert!(wget.error.is_none());

        let jq = results
            .iter()
            .find(|r| r.package_identifier == "jq")
            .unwrap();
        assert_eq!(jq.releases.len(), 1);
        assert_eq!(jq.releases[0].release_url, "https://jqlang.github.io/jq/");

        let curl = results
            .iter()
            .find(|r| r.package_identifier == "curl")
            .unwrap();
        assert_eq!(curl.releases.len(), 1, "curl has a latest stable version");
        assert_eq!(curl.releases[0].tag, "8.5.0");
    }

    #[tokio::test]
    async fn batch_fetch_releases_casks() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Cask),
            },
            BrewInfoExecutor::with_json(multi_cask_json()),
        )
        .await
        .expect("create");

        let items = vec![
            BatchFetchItem::new("firefox".to_string()),
            BatchFetchItem::new("google-chrome".to_string()),
        ];
        let results = plugin.batch_fetch_releases(&items).await.expect("ok");

        assert_eq!(results.len(), 2);

        let firefox = results
            .iter()
            .find(|r| r.package_identifier == "firefox")
            .unwrap();
        assert_eq!(firefox.releases.len(), 1);
        assert_eq!(firefox.releases[0].tag, "133.0");
        assert_eq!(
            firefox.releases[0].release_url,
            "https://www.mozilla.org/firefox/"
        );

        let chrome = results
            .iter()
            .find(|r| r.package_identifier == "google-chrome")
            .unwrap();
        assert_eq!(chrome.releases.len(), 1);
        assert_eq!(
            chrome.releases[0].release_url,
            "https://www.google.com/chrome/"
        );
    }

    #[tokio::test]
    async fn batch_fetch_releases_empty_returns_empty() {
        let plugin = HomebrewPlugin::new(HomebrewConfig::default(), test_executor())
            .await
            .expect("create");
        let results = plugin.batch_fetch_releases(&[]).await.expect("ok");
        assert!(results.is_empty());
    }
}
