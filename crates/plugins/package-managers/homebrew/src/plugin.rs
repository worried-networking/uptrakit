use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{PluginCapability, PluginError, Result};

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
    pub(crate) config: HomebrewConfig,
    pub(crate) executor: Arc<dyn CommandExecutor>,
}

impl HomebrewPlugin {
    /// Compile-time capabilities for the Homebrew plugin.
    pub const CAPABILITIES: &'static [PluginCapability] = &[
        PluginCapability::DiscoverLocalSoftware,
        PluginCapability::RefreshPackageIndex,
        PluginCapability::DetectHostCompatibility,
        PluginCapability::VersionDetection,
        PluginCapability::ReleaseFetching,
        PluginCapability::UpdateExecution,
        PluginCapability::ConfigTest,
    ];

    /// Create a new Homebrew plugin with the given configuration.
    pub async fn new(config: HomebrewConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        config
            .validate()
            .map_err(|e| rootcause::report!(PluginError::Configuration(e.to_string())))?;
        Ok(Self { config, executor })
    }

    /// Returns `true` if this instance is configured to track casks only.
    ///
    /// Returns `false` for `Both` (discover all) and `Formula` configs, so
    /// version-check operations default to formula behaviour when not explicitly
    /// set to `Cask`.
    pub(crate) fn is_cask(&self) -> bool {
        matches!(self.config.package_type, HomebrewPackageType::Cask)
    }

    pub(crate) fn require_package_identifier(&self, package_identifier: &str) -> Result<()> {
        if package_identifier.trim().is_empty() {
            rootcause::bail!(PluginError::Configuration(
                "package_identifier must not be empty".to_string()
            ));
        }
        Ok(())
    }

    /// Find the homepage URL of a formula by name in `brew info --json=v2` output.
    pub(crate) fn find_formula_homepage(json: &serde_json::Value, pkg_id: &str) -> String {
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
    pub(crate) fn find_cask_homepage(json: &serde_json::Value, pkg_id: &str) -> String {
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

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    HomebrewPlugin,
    HomebrewConfig,
    "package_manager_homebrew",
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
    use uptrakit_plugin_infrastructure_core::{LocalCommandExecutor, PluginBase, PluginCapability};

    use crate::config::{HomebrewConfig, HomebrewPackageType};

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
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
        assert!(plugin.has_capability(PluginCapability::VersionDetection));
        assert!(plugin.has_capability(PluginCapability::ReleaseFetching));
        assert!(plugin.has_capability(PluginCapability::UpdateExecution));
        assert_eq!(plugin.capabilities().len(), 7);
    }

    #[tokio::test]
    async fn is_cask_returns_false_for_both() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: HomebrewPackageType::Both,
            },
            test_executor(),
        )
        .await
        .expect("create");
        assert!(!plugin.is_cask());
    }

    #[tokio::test]
    async fn is_cask_returns_true_for_cask() {
        let plugin = HomebrewPlugin::new(
            HomebrewConfig {
                package_type: HomebrewPackageType::Cask,
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
                package_type: HomebrewPackageType::Formula,
            },
            test_executor(),
        )
        .await
        .expect("create");
        assert!(!plugin.is_cask());
    }

    // ── find_formula_homepage / find_cask_homepage ───────────────────────

    #[test]
    fn find_formula_homepage_returns_correct_url() {
        let json = serde_json::json!({
            "formulae": [{
                "name": "wget",
                "full_name": "wget",
                "versions": { "stable": "1.24.5", "head": null },
                "installed": [{ "version": "1.24.4", "installed_as_dependency": false }],
                "homepage": "https://www.gnu.org/software/wget/"
            }],
            "casks": []
        });
        let homepage = HomebrewPlugin::find_formula_homepage(&json, "wget");
        assert_eq!(homepage, "https://www.gnu.org/software/wget/");
    }

    #[test]
    fn find_formula_homepage_unknown_package_returns_empty() {
        let json = serde_json::json!({
            "formulae": [{
                "name": "wget",
                "full_name": "wget",
                "versions": { "stable": "1.24.5", "head": null },
                "installed": [{ "version": "1.24.4", "installed_as_dependency": false }],
                "homepage": "https://www.gnu.org/software/wget/"
            }],
            "casks": []
        });
        let homepage = HomebrewPlugin::find_formula_homepage(&json, "nonexistent");
        assert!(homepage.is_empty());
    }

    #[test]
    fn find_cask_homepage_returns_correct_url() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "firefox",
                "name": ["Mozilla Firefox"],
                "version": "133.0",
                "installed": "132.0",
                "homepage": "https://www.mozilla.org/firefox/"
            }]
        });
        let homepage = HomebrewPlugin::find_cask_homepage(&json, "firefox");
        assert_eq!(homepage, "https://www.mozilla.org/firefox/");
    }

    #[test]
    fn find_cask_homepage_unknown_package_returns_empty() {
        let json = serde_json::json!({
            "formulae": [],
            "casks": [{
                "token": "firefox",
                "name": ["Mozilla Firefox"],
                "version": "133.0",
                "installed": "132.0",
                "homepage": "https://www.mozilla.org/firefox/"
            }]
        });
        let homepage = HomebrewPlugin::find_cask_homepage(&json, "nonexistent");
        assert!(homepage.is_empty());
    }
}
