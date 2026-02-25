use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_core::mpsc;
use uptrakit_plugin_core::{
    DiscoveredSoftware, OutputStreamType, Plugin, PluginCapability, PluginError,
    PluginType, ReleaseInfo, Result, UpdateOutputLine, UpstreamRelease, Version,
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

/// Provider for Homebrew (macOS/Linux package manager).
///
/// Supports both formulae and casks. The `package_identifier` in `SoftwareItem`
/// is the Homebrew formula/cask name (e.g., `wget`, `firefox`).
pub struct HomebrewPlugin {
    config: HomebrewConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl HomebrewPlugin {
    /// Create a new Homebrew provider with the given configuration.
    pub fn new(config: HomebrewConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
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
    /// When `tag_extra` is true, annotates each item with
    /// `extra = {"package_type": "formula"}` for auto-discovery grouping.
    fn parse_installed_formulae(
        json: &serde_json::Value,
        tag_extra: bool,
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

                let extra = if tag_extra {
                    Some(serde_json::json!({"package_type": "formula"}))
                } else {
                    None
                };

                result.push(DiscoveredSoftware {
                    package_identifier: name.to_string(),
                    name: full_name.to_string(),
                    installed_version,
                    extra,
                });
            }
        }
        result
    }

    /// Parse installed casks from `brew info --installed --json=v2` output.
    ///
    /// Emits items only for packages with a known installed version.
    /// When `tag_extra` is true, annotates each item with
    /// `extra = {"package_type": "cask"}` for auto-discovery grouping.
    fn parse_installed_casks(json: &serde_json::Value, tag_extra: bool) -> Vec<DiscoveredSoftware> {
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

                let extra = if tag_extra {
                    Some(serde_json::json!({"package_type": "cask"}))
                } else {
                    None
                };

                result.push(DiscoveredSoftware {
                    package_identifier: token.to_string(),
                    name: name.to_string(),
                    installed_version,
                    extra,
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
}

#[async_trait]
impl Plugin for HomebrewPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Homebrew
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        &[
            PluginCapability::DiscoverLocalSoftware,
            PluginCapability::RefreshPackageIndex,
        ]
    }

    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Homebrew package index");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", ["update".to_string()]))
            .await
            .map_err(|e| {
                report!(PluginError::ProviderInternal(format!(
                    "brew update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("Homebrew package index refreshed");
        Ok(())
    }

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
                report!(PluginError::ProviderInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let packages = match &self.config.package_type {
            None => {
                // Discover-all mode: return both formulae and casks, each tagged
                // with extra metadata so the controller can route them to the
                // correct auto-created provider configs.
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
                report!(PluginError::ProviderInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let version = Self::parse_installed_version(&json, package_identifier, self.is_cask())
            .map(|v| Version::new(&v));
        tracing::debug!(version = ?version, "Homebrew version detection result");
        Ok(version)
    }

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
                report!(PluginError::ProviderInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(PluginError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(PluginError::ProviderInternal(format!(
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

        let releases = vec![UpstreamRelease {
            version: Version::new(&version_str),
            tag: version_str,
            is_prerelease: false,
            release_url: homepage.to_string(),
            release_notes: None,
            published_at: None,
            assets: vec![],
        }];
        tracing::debug!(count = releases.len(), "Homebrew releases fetched");
        Ok(releases)
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_core::LocalCommandExecutor;

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
    fn parse_installed_formulae_without_extra_tag() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_formulae(&json, false);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].package_identifier, "wget");
        assert_eq!(packages[0].name, "wget");
        assert_eq!(packages[0].installed_version, "1.24.4");
        assert!(packages[0].extra.is_none());
        assert_eq!(packages[1].package_identifier, "jq");
        assert_eq!(packages[1].installed_version, "1.7.1");
    }

    #[test]
    fn parse_installed_formulae_with_extra_tag() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_formulae(&json, true);
        assert_eq!(packages.len(), 2);
        assert_eq!(
            packages[0].extra,
            Some(serde_json::json!({"package_type": "formula"}))
        );
    }

    #[test]
    fn parse_installed_casks_without_extra_tag() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "firefox");
        assert_eq!(packages[0].name, "Mozilla Firefox");
        assert_eq!(packages[0].installed_version, "132.0");
        assert!(packages[0].extra.is_none());
    }

    #[test]
    fn parse_installed_casks_with_extra_tag() {
        let json = sample_installed_json();
        let packages = HomebrewPlugin::parse_installed_casks(&json, true);
        assert_eq!(packages.len(), 1);
        assert_eq!(
            packages[0].extra,
            Some(serde_json::json!({"package_type": "cask"}))
        );
    }

    #[test]
    fn parse_installed_casks_skips_not_installed() {
        let json = sample_cask_json_not_installed();
        let packages = HomebrewPlugin::parse_installed_casks(&json, false);
        assert!(packages.is_empty());
    }

    #[test]
    fn parse_installed_packages_empty() {
        let json = serde_json::json!({"formulae": [], "casks": []});
        let packages = HomebrewPlugin::parse_installed_formulae(&json, false);
        assert!(packages.is_empty());
    }

    // ── Provider trait ──────────────────────────────────────────────────

    #[test]
    fn homebrew_provider_capabilities() {
        let provider =
            HomebrewPlugin::new(HomebrewConfig::default(), test_executor()).expect("create");
        assert!(provider.has_capability(PluginCapability::DiscoverLocalSoftware));
        assert!(provider.has_capability(PluginCapability::RefreshPackageIndex));
        assert_eq!(provider.capabilities().len(), 2);
    }

    #[test]
    fn is_cask_returns_false_for_none() {
        let provider =
            HomebrewPlugin::new(HomebrewConfig { package_type: None }, test_executor())
                .expect("create");
        assert!(!provider.is_cask());
    }

    #[test]
    fn is_cask_returns_true_for_cask() {
        let provider = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Cask),
            },
            test_executor(),
        )
        .expect("create");
        assert!(provider.is_cask());
    }

    #[test]
    fn is_cask_returns_false_for_formula() {
        let provider = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: Some(HomebrewPackageType::Formula),
            },
            test_executor(),
        )
        .expect("create");
        assert!(!provider.is_cask());
    }

    #[tokio::test]
    async fn homebrew_provider_detect_installed_empty_identifier_fails() {
        let provider =
            HomebrewPlugin::new(HomebrewConfig::default(), test_executor()).expect("create");
        let result = provider.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn homebrew_provider_fetch_releases_empty_identifier_fails() {
        let provider =
            HomebrewPlugin::new(HomebrewConfig::default(), test_executor()).expect("create");
        let result = provider.fetch_releases("").await;
        assert!(result.is_err());
    }
}
