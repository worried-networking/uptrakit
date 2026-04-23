use std::sync::Arc;

use uptrakit_plugin_infrastructure_core::command::CommandExecutor;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, HostRequirements, HostRuntime, PluginConfigValidationError,
    PluginFamily, Result, declare_plugin,
};

use crate::config::{HomebrewConfig, HomebrewPackageType};

/// Validate a Homebrew package identifier.
///
/// Rejects empty values, leading/trailing whitespace, embedded whitespace, path-traversal
/// segments (`..`, `.`), empty path segments (`//`), and any characters outside the
/// allowed set `[A-Za-z0-9\-_.@+/]`.
pub fn validate_identifier(value: &str) -> std::result::Result<(), PluginConfigValidationError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must not be empty".to_string(),
        ));
    }
    if value != trimmed {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must not include leading or trailing whitespace".to_string(),
        ));
    }
    if value.chars().any(char::is_whitespace) {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must not contain whitespace".to_string(),
        ));
    }
    if value.len() > 200 {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier is too long".to_string(),
        ));
    }
    for ch in value.chars() {
        let valid = ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '@' | '+' | '/');
        if !valid {
            return Err(PluginConfigValidationError::InvalidIdentifier(format!(
                "package_identifier contains invalid character: {ch}"
            )));
        }
    }
    if value.split('/').any(|segment| segment.is_empty()) {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier contains an empty segment".to_string(),
        ));
    }
    if value
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier contains invalid segment".to_string(),
        ));
    }
    Ok(())
}

/// Validate that a Homebrew package identifier is non-empty.
///
/// Homebrew accepts any non-empty, non-whitespace-only string as a package
/// identifier.  This is a looser check than the formula name rules because
/// Homebrew enforces those constraints internally.
pub fn validate_identifier_nonempty(
    value: &str,
) -> std::result::Result<(), PluginConfigValidationError> {
    if value.trim().is_empty() {
        Err(PluginConfigValidationError::InvalidIdentifier(
            "package_identifier must not be empty".to_string(),
        ))
    } else {
        Ok(())
    }
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
    /// Create a new Homebrew plugin with the given configuration.
    pub fn new(
        config: HomebrewConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
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
        uptrakit_plugin_infrastructure_core::require_package_identifier(
            package_identifier,
            validate_identifier_nonempty,
        )
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

// ── Plugin descriptor ─────────────────────────────────────────────────────

declare_plugin!(HomebrewPlugin, HomebrewConfig, "package_manager_homebrew", {
    display_name: "Homebrew",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    type_settings: true,
    roles: [Discoverer, VersionDetector, ReleaseFetcher, PackageIndexer, UpdateExecutor],
});

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PluginMeta, StandardHostRuntime,
    };

    use crate::config::{HomebrewConfig, HomebrewPackageType};

    /// Helper to create a `HomebrewPlugin` for testing.
    fn test_plugin(config: HomebrewConfig) -> HomebrewPlugin {
        let executor = Arc::new(uptrakit_plugin_infrastructure_core::LocalCommandExecutor)
            as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        let runtime = Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>;
        HomebrewPlugin::new(config, runtime).unwrap()
    }

    // ── Plugin descriptor ───────────────────────────────────────────────

    #[test]
    fn plugin_type_id() {
        let plugin = test_plugin(HomebrewConfig::default());
        assert_eq!(plugin.plugin_type_id().as_str(), "package_manager_homebrew");
    }

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DiscoverLocalSoftware)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::DetectHostCompatibility)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::RefreshPackageIndex)
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
    }

    #[test]
    fn descriptor_has_expected_roles() {
        assert!(DESCRIPTOR.roles.discoverer.is_some());
        assert!(DESCRIPTOR.roles.version_detector.is_some());
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.package_indexer.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.lifecycle_hook.is_none());
    }

    #[test]
    fn descriptor_has_type_settings() {
        assert!(DESCRIPTOR.type_settings.is_some());
    }

    // ── is_cask ─────────────────────────────────────────────────────────

    #[test]
    fn is_cask_returns_false_for_both() {
        let plugin = test_plugin(HomebrewConfig {
            package_type: HomebrewPackageType::Both,
        });
        assert!(!plugin.is_cask());
    }

    #[test]
    fn is_cask_returns_true_for_cask() {
        let plugin = test_plugin(HomebrewConfig {
            package_type: HomebrewPackageType::Cask,
        });
        assert!(plugin.is_cask());
    }

    #[test]
    fn is_cask_returns_false_for_formula() {
        let plugin = test_plugin(HomebrewConfig {
            package_type: HomebrewPackageType::Formula,
        });
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
