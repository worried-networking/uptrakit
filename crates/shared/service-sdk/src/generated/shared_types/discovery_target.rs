// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::plugin_role::PluginRole;
use crate::generated::shared_types::plugin_type_id::PluginTypeId;
use serde::{Deserialize, Serialize};
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
/// # use uptrakit_service_sdk::generated::shared_types::{DiscoveryTarget, PluginTypeId, PluginRole, plugin_ids};
/// let target = DiscoveryTarget {
///     plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
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
    /// `releases_github` or `package_manager_apt` for tracking.
    pub plugin_type: PluginTypeId,
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
