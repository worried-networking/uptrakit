// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::discovery_target::DiscoveryTarget;
use serde::{Deserialize, Serialize};
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
