use serde::{Deserialize, Serialize};

use crate::discovery_target::DiscoveryTarget;
use crate::tracking_system::TrackingSystem;

/// A piece of software discovered on the local system by a plugin.
///
/// `installed_version` is required — plugins that cannot determine a version
/// must omit the item from results entirely.
///
/// This type is the canonical shared definition used in both the agent/plugin
/// layer and the wire protocol. The `uptrakit-plugin-core` crate re-exports it.
///
/// # Discovery targets
///
/// The `targets` field drives plugin-config creation and role assignment on the
/// controller. When non-empty, the controller processes each target generically
/// (find-or-create plugin config, create role assignments). When empty, the
/// controller falls back to the `plugin_config_id` on the enclosing
/// `DiscoveryPluginResult`.
///
/// The `extra` field is purely informational metadata (e.g. Docker's container
/// names) — the controller never interprets it for config synthesis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DiscoveredSoftware {
    /// Plugin-specific identifier for this software (e.g., package name, app slug).
    pub package_identifier: String,
    /// Human-readable display name.
    pub name: String,
    /// Currently installed version (required; plugins omit items with unknown versions).
    pub installed_version: String,
    /// Target plugin configurations for managing this item.
    ///
    /// Empty = use the discovering plugin's own config for all roles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<DiscoveryTarget>,
    /// Optional informational metadata (not used for config synthesis).
    ///
    /// Example: Docker's `{"containers": ["web-server"]}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
    /// Which tracking system this item belongs to.
    ///
    /// Every discovery plugin must explicitly declare the target system.
    /// Defaults to [`TrackingSystem::Targeted`] for backward compatibility
    /// with existing discovery results.
    #[serde(default)]
    pub tracking_system: TrackingSystem,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PluginRole, PluginType};

    #[test]
    fn serialization_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "wget".to_string(),
            name: "Wget".to_string(),
            installed_version: "1.21.3".to_string(),
            targets: vec![],
            extra: Some(serde_json::json!({"package_type": "formula"})),
            tracking_system: TrackingSystem::Targeted,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn optional_extra_omitted() {
        let sw = DiscoveredSoftware {
            package_identifier: "curl".to_string(),
            name: "cURL".to_string(),
            installed_version: "8.4.0".to_string(),
            targets: vec![],
            extra: None,
            tracking_system: TrackingSystem::Targeted,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(!json.contains("extra"));
        assert!(!json.contains("targets"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn empty_targets_omitted() {
        let sw = DiscoveredSoftware {
            package_identifier: "git".to_string(),
            name: "Git".to_string(),
            installed_version: "2.43.0".to_string(),
            targets: vec![],
            extra: None,
            tracking_system: TrackingSystem::Targeted,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(!json.contains("targets"));
    }

    #[test]
    fn targets_present_when_non_empty() {
        let sw = DiscoveredSoftware {
            package_identifier: "booklore".to_string(),
            name: "BookLore".to_string(),
            installed_version: "1.18.5".to_string(),
            targets: vec![DiscoveryTarget {
                plugin_type: PluginType::ReleasesGithub,
                plugin_config: serde_json::json!({"tag_strip_prefix": "v"}),
                plugin_config_name: "GitHub Releases".to_string(),
                roles: vec![PluginRole::FetchReleases],
                package_identifier: Some("BookLore/BookLore".to_string()),
                config_override: None,
                execution_site: None,
            }],
            extra: None,
            tracking_system: TrackingSystem::Targeted,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(json.contains("targets"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn backward_compatible_deserialize_without_targets() {
        let json = r#"{
            "package_identifier": "wget",
            "name": "Wget",
            "installed_version": "1.21.3"
        }"#;
        let sw: DiscoveredSoftware = serde_json::from_str(json).expect("deserialize");
        assert!(sw.targets.is_empty());
        assert!(sw.extra.is_none());
        assert_eq!(sw.tracking_system, TrackingSystem::Targeted);
    }

    #[test]
    fn tracking_system_host_managed_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "nginx".to_string(),
            name: "nginx".to_string(),
            installed_version: "1.24.0".to_string(),
            targets: vec![],
            extra: None,
            tracking_system: TrackingSystem::HostManaged,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(json.contains("host_managed"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.tracking_system, TrackingSystem::HostManaged);
    }
}
