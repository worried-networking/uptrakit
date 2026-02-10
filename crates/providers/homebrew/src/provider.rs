use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_provider_core::command::{run_command_exec, send_output};
use uptrakit_provider_core::{
    DiscoveredSoftware, Provider, ProviderCapability, ProviderError, Result, UpdateContext,
    UpdateOutputLine, UpdateOutputStream, UpstreamRelease, Version,
};

use crate::config::{HomebrewConfig, HomebrewPackageType};

/// Provider for Homebrew (macOS/Linux package manager).
///
/// Supports both formulae and casks. The `package_identifier` in `SoftwareItem`
/// is the Homebrew formula/cask name (e.g., `wget`, `firefox`).
pub struct HomebrewProvider {
    config: HomebrewConfig,
}

impl HomebrewProvider {
    /// Create a new Homebrew provider with the given configuration.
    pub fn new(config: HomebrewConfig) -> Self {
        Self { config }
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
}

#[async_trait]
impl Provider for HomebrewProvider {
    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[
            ProviderCapability::DiscoverLocalSoftware,
            ProviderCapability::RefreshPackageIndex,
        ]
    }

    async fn refresh_package_index(&self) -> Result<()> {
        tracing::info!("refreshing Homebrew package index");
        let (tx, _rx) = mpsc::channel(1);
        let (_output, exit_code) = run_command_exec("brew", &["update".to_string()], None, &tx)
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "brew update failed: {e}"
                )))
            })?;

        if exit_code != 0 {
            return Err(report!(ProviderError::CommandFailed(exit_code)));
        }

        tracing::info!("Homebrew package index refreshed");
        Ok(())
    }

    async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
        tracing::debug!(
            is_cask = self.is_cask(),
            "discovering installed Homebrew packages"
        );
        let (tx, _rx) = mpsc::channel(1);
        let (output, exit_code) = run_command_exec(
            "brew",
            &[
                "info".to_string(),
                "--installed".to_string(),
                "--json=v2".to_string(),
            ],
            None,
            &tx,
        )
        .await
        .map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "brew info failed: {e}"
            )))
        })?;

        if exit_code != 0 {
            return Err(report!(ProviderError::CommandFailed(exit_code)));
        }

        let json: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        Ok(Self::parse_installed_packages(&json, self.is_cask()))
    }

    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        // detect_installed_version requires a package_identifier which is not
        // available at the provider level. The agent calls this via
        // check_version() which provides the config including the package
        // identifier. For Homebrew, the package_identifier is passed via the
        // provider config at runtime. This default returns None.
        Ok(None)
    }

    async fn fetch_releases(&self) -> Result<Vec<UpstreamRelease>> {
        // fetch_releases also requires a package_identifier. For Homebrew,
        // the agent calls this after creating a per-item provider. This
        // default returns an empty vec.
        Ok(vec![])
    }

    async fn execute_update(
        &self,
        ctx: &UpdateContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        let pkg = &ctx.package_identifier;
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

        let (cmd_output, _exit_code) = run_command_exec("brew", &args, None, output_tx)
            .await
            .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output);

        Ok(output)
    }
}

/// A package-specific Homebrew provider that knows the package identifier.
///
/// Created by the agent for per-item version checks and release fetches.
pub struct HomebrewPackageProvider {
    config: HomebrewConfig,
    package_identifier: String,
}

impl HomebrewPackageProvider {
    /// Create a new package-specific Homebrew provider.
    pub fn new(config: HomebrewConfig, package_identifier: String) -> Self {
        Self {
            config,
            package_identifier,
        }
    }

    fn is_cask(&self) -> bool {
        self.config.package_type == HomebrewPackageType::Cask
    }
}

#[async_trait]
impl Provider for HomebrewPackageProvider {
    fn capabilities(&self) -> &'static [ProviderCapability] {
        &[
            ProviderCapability::DiscoverLocalSoftware,
            ProviderCapability::RefreshPackageIndex,
        ]
    }

    async fn refresh_package_index(&self) -> Result<()> {
        let (tx, _rx) = mpsc::channel(1);
        let (_output, exit_code) = run_command_exec("brew", &["update".to_string()], None, &tx)
            .await
            .map_err(|e| {
                report!(ProviderError::ProviderInternal(format!(
                    "brew update failed: {e}"
                )))
            })?;

        if exit_code != 0 {
            return Err(report!(ProviderError::CommandFailed(exit_code)));
        }
        Ok(())
    }

    async fn detect_installed_version(&self) -> Result<Option<Version>> {
        let pkg = &self.package_identifier;
        let (tx, _rx) = mpsc::channel(1);
        let (output, exit_code) = run_command_exec(
            "brew",
            &["info".to_string(), "--json=v2".to_string(), pkg.to_string()],
            None,
            &tx,
        )
        .await
        .map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "brew info failed: {e}"
            )))
        })?;

        if exit_code != 0 {
            return Err(report!(ProviderError::CommandFailed(exit_code)));
        }

        let json: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        Ok(
            HomebrewProvider::parse_installed_version(&json, pkg, self.is_cask())
                .map(|v| Version::new(&v)),
        )
    }

    async fn fetch_releases(&self) -> Result<Vec<UpstreamRelease>> {
        let pkg = &self.package_identifier;
        let (tx, _rx) = mpsc::channel(1);
        let (output, exit_code) = run_command_exec(
            "brew",
            &["info".to_string(), "--json=v2".to_string(), pkg.to_string()],
            None,
            &tx,
        )
        .await
        .map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "brew info failed: {e}"
            )))
        })?;

        if exit_code != 0 {
            return Err(report!(ProviderError::CommandFailed(exit_code)));
        }

        let json: serde_json::Value = serde_json::from_str(&output).map_err(|e| {
            report!(ProviderError::ProviderInternal(format!(
                "failed to parse brew info JSON: {e}"
            )))
        })?;

        let Some(version_str) = HomebrewProvider::parse_latest_version(&json, pkg, self.is_cask())
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
        ctx: &UpdateContext,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> Result<String> {
        let pkg = &ctx.package_identifier;
        let mut output = String::new();

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

        let (cmd_output, _exit_code) = run_command_exec("brew", &args, None, output_tx)
            .await
            .map_err(|e| report!(ProviderError::InstallFailed(e.to_string())))?;
        output.push_str(&cmd_output);

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Sample `brew info --json=v2` output for a formula ───────────────

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
        let provider = HomebrewProvider::new(HomebrewConfig::default());
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        assert!(provider.has_capability(ProviderCapability::RefreshPackageIndex));
        assert_eq!(provider.capabilities().len(), 2);
    }

    #[test]
    fn homebrew_package_provider_capabilities() {
        let provider = HomebrewPackageProvider::new(HomebrewConfig::default(), "wget".to_string());
        assert!(provider.has_capability(ProviderCapability::DiscoverLocalSoftware));
        assert!(provider.has_capability(ProviderCapability::RefreshPackageIndex));
    }

    #[tokio::test]
    async fn homebrew_provider_detect_installed_returns_none() {
        let provider = HomebrewProvider::new(HomebrewConfig::default());
        let result = provider.detect_installed_version().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn homebrew_provider_fetch_releases_returns_empty() {
        let provider = HomebrewProvider::new(HomebrewConfig::default());
        let result = provider.fetch_releases().await.unwrap();
        assert!(result.is_empty());
    }
}
