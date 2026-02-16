use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_provider_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_provider_core::{
    DiscoveredSoftware, Provider, ProviderCapability, ProviderError, ProviderType, ReleaseInfo,
    Result, UpdateOutputLine, UpdateOutputStream, UpstreamRelease, Version,
};

use crate::config::{HomebrewConfig, HomebrewPackageType};

/// Provider for Homebrew (macOS/Linux package manager).
///
/// Supports both formulae and casks. The `package_identifier` in `SoftwareItem`
/// is the Homebrew formula/cask name (e.g., `wget`, `firefox`).
pub struct HomebrewProvider {
    config: HomebrewConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl HomebrewProvider {
    /// Create a new Homebrew provider with the given configuration.
    pub fn new(config: HomebrewConfig, executor: Arc<dyn CommandExecutor>) -> Self {
        Self { config, executor }
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

    /// Parse all installed formulae/casks from `brew info --installed --json=v2` output.
    fn parse_installed_packages(
        json: &serde_json::Value,
        is_cask: bool,
    ) -> Vec<DiscoveredSoftware> {
        let mut result = Vec::new();

        if is_cask {
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
                    let installed_version = cask
                        .get("installed")
                        .and_then(|v| v.as_str())
                        .map(Version::new);

                    result.push(DiscoveredSoftware {
                        package_identifier: token.to_string(),
                        name: name.to_string(),
                        installed_version,
                        extra: None,
                    });
                }
            }
        } else if let Some(formulae) = json.get("formulae").and_then(|f| f.as_array()) {
            for formula in formulae {
                let Some(name) = formula.get("name").and_then(|n| n.as_str()) else {
                    continue;
                };
                let full_name = formula
                    .get("full_name")
                    .and_then(|n| n.as_str())
                    .unwrap_or(name);
                let installed_version = formula
                    .get("installed")
                    .and_then(|arr| arr.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|obj| obj.get("version"))
                    .and_then(|v| v.as_str())
                    .map(Version::new);

                result.push(DiscoveredSoftware {
                    package_identifier: name.to_string(),
                    name: full_name.to_string(),
                    installed_version,
                    extra: None,
                });
            }
        }

        result
    }

    fn is_cask(&self) -> bool {
        self.config.package_type == HomebrewPackageType::Cask
    }

    fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        if package_identifier.trim().is_empty() {
            bail!(ProviderError::Configuration(
                "package_identifier must not be empty".to_string()
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for HomebrewProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::Homebrew
    }

    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[
            ProviderCapability::DiscoverLocalSoftware,
            ProviderCapability::RefreshPackageIndex,
        ]
    }

    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Homebrew package index");
        let cmd_output = self
            .executor
            .execute_quiet(&CommandSpec::exec("brew", ["update".to_string()]))
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "brew update failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(cmd_output.exit_code));
        }

        tracing::info!("Homebrew package index refreshed");
        Ok(())
    }

    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::debug!(
            is_cask = self.is_cask(),
            "discovering installed Homebrew packages"
        );
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
                report!(ProviderError::ProviderInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        Ok(Self::parse_installed_packages(&json, self.is_cask()))
    }

    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        self.require_package_identifier(package_identifier)?;
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
                report!(ProviderError::ProviderInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        Ok(
            Self::parse_installed_version(&json, package_identifier, self.is_cask())
                .map(|v| Version::new(&v)),
        )
    }

    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;
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
                report!(ProviderError::ProviderInternal(format!(
                    "brew info failed: {e}"
                )))
            })?;

        if cmd_output.exit_code != 0 {
            bail!(ProviderError::CommandFailed(cmd_output.exit_code));
        }

        let json: serde_json::Value = serde_json::from_str(&cmd_output.output).map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
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

        Ok(vec![UpstreamRelease {
            version: Version::new(&version_str),
            tag: version_str,
            is_prerelease: false,
            release_url: homepage.to_string(),
            release_notes: None,
            published_at: None,
            assets: vec![],
        }])
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

        let action = if self.is_cask() {
            format!("brew upgrade --cask {pkg}")
        } else {
            format!("brew upgrade {pkg}")
        };

        send_output(
            output_tx,
            &format!("Running: {action}"),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!("Running: {action}\n"));

        let args: Vec<String> = if self.is_cask() {
            vec!["upgrade".to_string(), "--cask".to_string(), pkg.to_string()]
        } else {
            vec!["upgrade".to_string(), pkg.to_string()]
        };

        send_output(
            output_tx,
            &format!("Running: brew {}", args.join(" ")),
            UpdateOutputStream::Stdout,
        )
        .await;
        output.push_str(&format!("Running: brew {}\n", args.join(" ")));

        let cmd_output = self
            .executor
            .execute(&CommandSpec::exec("brew", args), output_tx)
            .await
            .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output.output);

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_provider_core::LocalCommandExecutor;

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
        let version = HomebrewProvider::parse_installed_version(&json, "wget", false);
        assert_eq!(version, Some("1.24.4".to_string()));
    }

    #[test]
    fn parse_installed_version_formula_not_installed() {
        let json = sample_formula_json_not_installed();
        let version = HomebrewProvider::parse_installed_version(&json, "wget", false);
        assert_eq!(version, None);
    }

    #[test]
    fn parse_installed_version_cask() {
        let json = sample_cask_json();
        let version = HomebrewProvider::parse_installed_version(&json, "firefox", true);
        assert_eq!(version, Some("132.0".to_string()));
    }

    #[test]
    fn parse_installed_version_cask_not_installed() {
        let json = sample_cask_json_not_installed();
        let version = HomebrewProvider::parse_installed_version(&json, "firefox", true);
        assert_eq!(version, None);
    }

    #[test]
    fn parse_installed_version_unknown_package() {
        let json = sample_formula_json();
        let version = HomebrewProvider::parse_installed_version(&json, "nonexistent", false);
        assert_eq!(version, None);
    }

    // ── parse_latest_version ────────────────────────────────────────────

    #[test]
    fn parse_latest_version_formula() {
        let json = sample_formula_json();
        let version = HomebrewProvider::parse_latest_version(&json, "wget", false);
        assert_eq!(version, Some("1.24.5".to_string()));
    }

    #[test]
    fn parse_latest_version_cask() {
        let json = sample_cask_json();
        let version = HomebrewProvider::parse_latest_version(&json, "firefox", true);
        assert_eq!(version, Some("133.0".to_string()));
    }

    #[test]
    fn parse_latest_version_cask_with_latest_marker() {
        let json = sample_cask_latest_version_json();
        let version = HomebrewProvider::parse_latest_version(&json, "google-chrome", true);
        // "latest" is filtered out — not a useful version string
        assert_eq!(version, None);
    }

    #[test]
    fn parse_latest_version_unknown_package() {
        let json = sample_formula_json();
        let version = HomebrewProvider::parse_latest_version(&json, "nonexistent", false);
        assert_eq!(version, None);
    }

    // ── parse_installed_packages ────────────────────────────────────────

    #[test]
    fn parse_installed_packages_formulae() {
        let json = sample_installed_json();
        let packages = HomebrewProvider::parse_installed_packages(&json, false);
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0].package_identifier, "wget");
        assert_eq!(packages[0].name, "wget");
        assert_eq!(packages[0].installed_version, Some(Version::new("1.24.4")));
        assert_eq!(packages[1].package_identifier, "jq");
        assert_eq!(packages[1].installed_version, Some(Version::new("1.7.1")));
    }

    #[test]
    fn parse_installed_packages_casks() {
        let json = sample_installed_json();
        let packages = HomebrewProvider::parse_installed_packages(&json, true);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].package_identifier, "firefox");
        assert_eq!(packages[0].name, "Mozilla Firefox");
        assert_eq!(packages[0].installed_version, Some(Version::new("132.0")));
    }

    #[test]
    fn parse_installed_packages_empty() {
        let json = serde_json::json!({"formulae": [], "casks": []});
        let packages = HomebrewProvider::parse_installed_packages(&json, false);
        assert!(packages.is_empty());
    }

    // ── Provider trait ──────────────────────────────────────────────────

    #[test]
    fn homebrew_provider_capabilities() {
        let provider = HomebrewProvider::new(HomebrewConfig::default(), test_executor());
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        assert!(provider.has_capability(ProviderCapability::RefreshPackageIndex));
        assert_eq!(provider.capabilities().len(), 2);
    }

    #[tokio::test]
    async fn homebrew_provider_detect_installed_empty_identifier_fails() {
        let provider = HomebrewProvider::new(HomebrewConfig::default(), test_executor());
        let result = provider.detect_installed_version("").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn homebrew_provider_fetch_releases_empty_identifier_fails() {
        let provider = HomebrewProvider::new(HomebrewConfig::default(), test_executor());
        let result = provider.fetch_releases("").await;
        assert!(result.is_err());
    }
}
