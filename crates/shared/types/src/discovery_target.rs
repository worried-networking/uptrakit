use serde::{Deserialize, Serialize};

use crate::plugin_role::PluginRole;
use crate::plugin_types::PluginType;

/// A structured target that tells the autodiscovery controller exactly which
/// plugin config (and role assignments) to create for a discovered software item.
///
/// Plugins emit `DiscoveryTarget` values inside [`super::DiscoveredSoftware::targets`]
/// so that the web-API controller can process them generically — without any
/// plugin-specific synthesis logic.
///
/// # Examples
///
/// PHS plugin discovering a GitHub-managed app (fetch releases only; the
/// `owner/repo` is expressed as the `package_identifier` override):
///
/// ```
/// # use uptrakit_shared_types::{DiscoveryTarget, PluginType, PluginRole};
/// let target = DiscoveryTarget {
///     plugin_type: PluginType::ReleasesGithub,
///     plugin_config: serde_json::json!({
///         "tag_strip_prefix": "v",
///         "include_prereleases": false,
///     }),
///     plugin_config_name: "GitHub Releases".to_string(),
///     roles: vec![PluginRole::FetchReleases],
///     package_identifier: Some("BookLore/BookLore".to_string()),
///     config_override: None,
///     execution_site: None,
/// };
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DiscoveryTarget {
    /// Target plugin type (may differ from the discovering plugin).
    ///
    /// For example, the PHS plugin discovers software but targets
    /// `GithubReleases` or `Apt` for tracking.
    pub plugin_type: PluginType,

    /// Config JSON for find-or-create of the target plugin config.
    ///
    /// The controller will search for an existing active plugin config
    /// whose JSON matches this value, or create a new one.
    pub plugin_config: serde_json::Value,

    /// Display name for auto-created plugin config (e.g. "BookLore/BookLore").
    pub plugin_config_name: String,

    /// Which roles this target covers.
    ///
    /// Typically all three: `DetectVersion`, `FetchReleases`, `ExecuteUpdate`.
    pub roles: Vec<PluginRole>,

    /// Package identifier override (None = same as parent `DiscoveredSoftware`).
    ///
    /// Used when the target plugin needs a different identifier than the
    /// discovering plugin's slug (e.g. PHS slug → APT package name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_identifier: Option<String>,

    /// Per-assignment config override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_override: Option<serde_json::Value>,

    /// Execution site hint (`"auto"` | `"agent"` | `"controller"`; None = `"auto"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_site: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_roles() -> Vec<PluginRole> {
        vec![
            PluginRole::DetectVersion,
            PluginRole::FetchReleases,
            PluginRole::ExecuteUpdate,
        ]
    }

    #[test]
    fn serialization_roundtrip() {
        let target = DiscoveryTarget {
            plugin_type: PluginType::ReleasesGithub,
            plugin_config: serde_json::json!({"tag_strip_prefix": "v"}),
            plugin_config_name: "GitHub Releases".to_string(),
            roles: vec![PluginRole::FetchReleases],
            package_identifier: Some("acme/widget".to_string()),
            config_override: None,
            execution_site: None,
        };
        let json = serde_json::to_string(&target).expect("serialize");
        let deserialized: DiscoveryTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, target);
    }

    #[test]
    fn optional_fields_omitted_when_none() {
        let target = DiscoveryTarget {
            plugin_type: PluginType::PackageManagerApt,
            plugin_config: serde_json::json!({}),
            plugin_config_name: "APT".to_string(),
            roles: all_roles(),
            package_identifier: None,
            config_override: None,
            execution_site: None,
        };
        let json = serde_json::to_string(&target).expect("serialize");
        assert!(!json.contains("package_identifier"));
        assert!(!json.contains("config_override"));
        assert!(!json.contains("execution_site"));
    }

    #[test]
    fn optional_fields_present_when_set() {
        let target = DiscoveryTarget {
            plugin_type: PluginType::PackageManagerApt,
            plugin_config: serde_json::json!({}),
            plugin_config_name: "APT".to_string(),
            roles: vec![PluginRole::DetectVersion],
            package_identifier: Some("grafana".to_string()),
            config_override: Some(serde_json::json!({"priority": "high"})),
            execution_site: Some("agent".to_string()),
        };
        let json = serde_json::to_string(&target).expect("serialize");
        assert!(json.contains("package_identifier"));
        assert!(json.contains("config_override"));
        assert!(json.contains("execution_site"));

        let deserialized: DiscoveryTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, target);
    }

    #[test]
    fn cross_plugin_target() {
        // Represents a PHS-discovered GitHub-managed item: the GitHub plugin
        // covers FetchReleases only; `owner/repo` is the package_identifier override.
        let target = DiscoveryTarget {
            plugin_type: PluginType::ReleasesGithub,
            plugin_config: serde_json::json!({"tag_strip_prefix": "v"}),
            plugin_config_name: "GitHub Releases".to_string(),
            roles: vec![PluginRole::FetchReleases],
            package_identifier: Some("BookLore/BookLore".to_string()),
            config_override: None,
            execution_site: None,
        };
        let json = serde_json::to_string(&target).expect("serialize");
        let deserialized: DiscoveryTarget = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.plugin_type, PluginType::ReleasesGithub);
        assert_eq!(
            deserialized.package_identifier,
            Some("BookLore/BookLore".to_string())
        );
    }

    #[test]
    fn deserialize_with_missing_optional_fields() {
        let json = r#"{
            "plugin_type": "package_manager_apt",
            "plugin_config": {},
            "plugin_config_name": "APT",
            "roles": ["detect_version", "fetch_releases", "execute_update"]
        }"#;
        let target: DiscoveryTarget = serde_json::from_str(json).expect("deserialize");
        assert_eq!(target.plugin_type, PluginType::PackageManagerApt);
        assert_eq!(target.package_identifier, None);
        assert_eq!(target.config_override, None);
        assert_eq!(target.execution_site, None);
    }
}
