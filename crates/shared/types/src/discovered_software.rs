use serde::{Deserialize, Serialize};

use crate::discovery_target::DiscoveryTarget;

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
///
/// # Per-row qualifier
///
/// The `qualifier` field selects which `host_software_item` row to create or
/// reuse. `None` = unqualified (default behaviour, one row per software item per
/// host). Docker uses the container name here so that each container gets its
/// own tracking row even when multiple containers run the same image.
///
/// # Plugin package identifier
///
/// `plugin_package_identifier`, when set, overrides `package_identifier` as the
/// value stored in `host_software_item_plugin.package_identifier` for plugin
/// operations. `None` = use `package_identifier` (existing behaviour).
///
/// # Pinning
///
/// When `featured` is `true`, the controller marks the software item as
/// featured on first creation so it gets individual MQTT entities and
/// prominent visibility. Default `false` — item starts unfeatured
/// (bulk/aggregate view only). The controller only applies `featured: true`
/// when **creating** a new `software_items` row. Subsequent discoveries do
/// not override a user's manual feature/unfeature choice.
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
    /// Row discriminator within `host_software_items`.
    ///
    /// `None` = unqualified (default). Docker sets this to the container name
    /// so that each container produces its own `host_software_item` row even
    /// when multiple containers run the same image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    /// Override for the `package_identifier` stored in
    /// `host_software_item_plugin.package_identifier`.
    ///
    /// `None` = use `package_identifier` (existing behaviour).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_package_identifier: Option<String>,
    /// When `true`, the controller marks the software item as featured on
    /// first creation. Default `false` — item starts unfeatured.
    #[serde(default)]
    pub featured: bool,
    /// Plugin-provided display version for the installed version (e.g. Docker image publish date).
    /// `None` when the plugin cannot determine a display version during discovery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_display_version: Option<String>,
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
            qualifier: None,
            plugin_package_identifier: None,
            featured: false,
            installed_display_version: None,
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
            qualifier: None,
            plugin_package_identifier: None,
            featured: false,
            installed_display_version: None,
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
            qualifier: None,
            plugin_package_identifier: None,
            featured: false,
            installed_display_version: None,
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
            qualifier: None,
            plugin_package_identifier: None,
            featured: true,
            installed_display_version: None,
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
        assert!(sw.qualifier.is_none());
        assert!(sw.plugin_package_identifier.is_none());
        assert!(!sw.featured);
    }

    #[test]
    fn qualifier_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "nginx".to_string(),
            name: "nginx".to_string(),
            installed_version: "1.24.0".to_string(),
            targets: vec![],
            extra: None,
            qualifier: Some("web-container".to_string()),
            plugin_package_identifier: None,
            featured: false,
            installed_display_version: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(json.contains("qualifier"));
        assert!(json.contains("web-container"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.qualifier, Some("web-container".to_string()));
    }

    #[test]
    fn qualifier_none_omitted() {
        let sw = DiscoveredSoftware {
            package_identifier: "nginx".to_string(),
            name: "nginx".to_string(),
            installed_version: "1.24.0".to_string(),
            targets: vec![],
            extra: None,
            qualifier: None,
            plugin_package_identifier: None,
            featured: false,
            installed_display_version: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(!json.contains("qualifier"));
        assert!(!json.contains("plugin_package_identifier"));
    }

    #[test]
    fn plugin_package_identifier_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "sha256:abc123".to_string(),
            name: "my-app".to_string(),
            installed_version: "2.0.0".to_string(),
            targets: vec![],
            extra: None,
            qualifier: Some("app-container".to_string()),
            plugin_package_identifier: Some("my-app:2.0.0".to_string()),
            featured: false,
            installed_display_version: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(json.contains("plugin_package_identifier"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(
            deserialized.plugin_package_identifier,
            Some("my-app:2.0.0".to_string())
        );
        assert_eq!(deserialized.qualifier, Some("app-container".to_string()));
    }

    #[test]
    fn featured_true_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "myapp".to_string(),
            name: "My App".to_string(),
            installed_version: "1.0.0".to_string(),
            targets: vec![],
            extra: None,
            qualifier: None,
            plugin_package_identifier: None,
            featured: true,
            installed_display_version: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(json.contains("\"featured\":true"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert!(deserialized.featured);
    }

    #[test]
    fn featured_defaults_to_false() {
        let json = r#"{
            "package_identifier": "wget",
            "name": "Wget",
            "installed_version": "1.21.3"
        }"#;
        let sw: DiscoveredSoftware = serde_json::from_str(json).expect("deserialize");
        assert!(!sw.featured);
    }
}
