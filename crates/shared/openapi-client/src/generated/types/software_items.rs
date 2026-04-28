// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use crate::generated::shared_types::{PluginRole, PluginTypeId};
use crate::generated::types::pagination::PaginationParams;
use crate::generated::types::plugin_configs::CreatePluginConfigRequest;
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;
fn default_execution_site() -> String {
    "auto".to_string()
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "serde_json::Value", into = "serde_json::Value")]
pub struct JsonObjectMap(serde_json::Map<String, serde_json::Value>);
impl TryFrom<serde_json::Value> for JsonObjectMap {
    type Error = ValidationError;
    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::Object(map) => Ok(Self(map)),
            _ => Err(ValidationError {
                field: "config_override",
                message: "must be a JSON object".to_string(),
            }),
        }
    }
}
impl JsonObjectMap {
    pub fn is_object(&self) -> bool {
        true
    }
    pub fn as_object(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.0
    }
}
impl From<JsonObjectMap> for serde_json::Value {
    fn from(value: JsonObjectMap) -> Self {
        serde_json::Value::Object(value.0)
    }
}
#[derive(Debug, Clone, Default, PartialEq)]
pub enum IconUrlPatch {
    #[default]
    Keep,
    Set(String),
    Clear,
}
impl IconUrlPatch {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
    pub fn from_json(value: Option<&serde_json::Value>) -> Result<Self, ValidationError> {
        match value {
            None => Ok(Self::Keep),
            Some(serde_json::Value::Null) => Ok(Self::Clear),
            Some(serde_json::Value::String(url)) => Ok(Self::Set(url.clone())),
            Some(_) => Err(ValidationError {
                field: "icon_url",
                message: "icon_url must be null, a string, or omitted".to_string(),
            }),
        }
    }
}
impl Serialize for IconUrlPatch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Keep | Self::Clear => serializer.serialize_none(),
            Self::Set(url) => url.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for IconUrlPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match Option::<String>::deserialize(deserializer)? {
            Some(url) => Self::Set(url),
            None => Self::Clear,
        })
    }
}
#[derive(Clone, Debug, Default, PartialEq)]
pub enum JsonObjectMapPatch {
    #[default]
    Keep,
    Set(JsonObjectMap),
    Clear,
}
impl JsonObjectMapPatch {
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
    pub fn as_set(&self) -> Option<&JsonObjectMap> {
        match self {
            Self::Set(value) => Some(value),
            Self::Keep | Self::Clear => None,
        }
    }
    pub fn into_option(self) -> Option<JsonObjectMap> {
        match self {
            Self::Set(value) => Some(value),
            Self::Keep | Self::Clear => None,
        }
    }
    pub fn resolve(self, current: Option<JsonObjectMap>) -> Option<JsonObjectMap> {
        match self {
            Self::Keep => current,
            Self::Set(value) => Some(value),
            Self::Clear => None,
        }
    }
}
impl Serialize for JsonObjectMapPatch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            Self::Keep | Self::Clear => serializer.serialize_none(),
            Self::Set(value) => value.serialize(serializer),
        }
    }
}
impl<'de> Deserialize<'de> for JsonObjectMapPatch {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(match Option::<JsonObjectMap>::deserialize(deserializer)? {
            Some(value) => Self::Set(value),
            None => Self::Clear,
        })
    }
}
fn validate_https_icon_url(url: &str) -> Result<(), ValidationError> {
    if url.len() > 2048 {
        return Err(ValidationError {
            field: "icon_url",
            message: "icon_url must not exceed 2048 characters".to_string(),
        });
    }
    if !url.starts_with("https://") {
        return Err(ValidationError {
            field: "icon_url",
            message: "icon_url must start with https://".to_string(),
        });
    }
    Ok(())
}
/// Create a new software item (catalog entry only — no plugin coupling).
#[derive(Serialize, Deserialize)]
pub struct CreateSoftwareItemRequest {
    /// Display name (e.g. "1Password").
    pub name: String,
    /// Whether this item is featured (shown prominently). Defaults to true for manual creation.
    #[serde(default = "crate::generated::types::default_featured")]
    pub featured: bool,
    /// Optional HTTPS URL to an icon/logo image for this software item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}
/// Partial update for a software item. Only `name` and `featured` are updatable.
#[derive(Serialize, Deserialize)]
pub struct UpdateSoftwareItemRequest {
    pub name: Option<String>,
    pub featured: Option<bool>,
    /// Set, clear, or keep the icon URL.
    ///
    /// - Absent JSON key: keep existing value.
    /// - `null`: clear the icon URL.
    /// - String: set a new HTTPS URL.
    #[serde(default, skip_serializing_if = "IconUrlPatch::is_keep")]
    pub icon_url: IconUrlPatch,
}
/// Per-host plugin assignment used when assigning hosts to a software item.
///
/// Each host assignment contains a list of role-specific plugin assignments.
/// At minimum, a `detect_version` role should be provided for version tracking.
#[derive(Serialize, Deserialize)]
pub struct HostSoftwareAssignment {
    pub host_id: Uuid,
    /// Role-specific plugin assignments for this host-software pair.
    pub plugins: Vec<HostPluginRoleAssignment>,
}
/// A plugin assignment for a specific role on a host-software pair.
#[derive(Serialize, Deserialize)]
pub struct HostPluginRoleAssignment {
    /// The role this plugin serves (e.g. `detect_version`, `fetch_releases`, `execute_update`).
    pub role: PluginRole,
    /// Ordinal for hook roles; must be `0` for non-hook roles. Defaults to `0`.
    #[serde(default)]
    pub ordinal: i32,
    /// UUID of an existing plugin config to use.
    pub plugin_config_id: Option<Uuid>,
    /// Inline plugin config to create (mutually exclusive with `plugin_config_id`).
    pub plugin_config: Option<CreatePluginConfigRequest>,
    /// Plugin-specific package identifier.
    pub package_identifier: String,
    /// Plugin-specific overrides merged onto the base config at resolution time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_override: Option<JsonObjectMap>,
    /// Controls where this plugin's operation is executed.
    /// - `"auto"`: system decides based on plugin capabilities (default)
    /// - `"agent"`: always run on the agent
    /// - `"controller"`: always run on the controller (only valid for `fetch_releases`)
    #[serde(default = "default_execution_site")]
    pub execution_site: String,
}
/// Assign one or more hosts to a software item, each with its own plugin info.
#[derive(Serialize, Deserialize)]
pub struct AssignHostsRequest {
    pub host_assignments: Vec<HostSoftwareAssignment>,
}
/// Update a single role assignment for an existing host–software-item pair.
#[derive(Serialize, Deserialize)]
pub struct UpdateHostAssignmentRequest {
    /// The role to update (e.g. `detect_version`, `fetch_releases`, `execute_update`).
    pub role: PluginRole,
    /// Ordinal for this assignment. For hook roles (pre/post_update_hook), multiple
    /// assignments with different ordinals are allowed. For non-hook roles, this
    /// must be `0`. Defaults to `0`.
    #[serde(default)]
    pub ordinal: i32,
    /// UUID of an existing plugin config to use.
    pub plugin_config_id: Option<Uuid>,
    /// Inline plugin config to create and link (mutually exclusive with `plugin_config_id` and `plugin_type`).
    pub plugin_config: Option<CreatePluginConfigRequest>,
    /// Plugin type for a truly inline assignment with no shared config row
    /// (mutually exclusive with `plugin_config_id` and `plugin_config`).
    /// The full plugin config is supplied via `config_override`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_type: Option<PluginTypeId>,
    pub package_identifier: Option<String>,
    /// Omit to keep, send `null` to clear, or send an object to set the override.
    #[serde(default, skip_serializing_if = "JsonObjectMapPatch::is_keep")]
    pub config_override: JsonObjectMapPatch,
    /// Controls where this plugin's operation is executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_site: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct SoftwareItemResponse {
    pub id: Uuid,
    pub name: String,
    /// Distinct plugin type identifiers from all active host assignments (for display in lists).
    pub plugins: Vec<String>,
    pub featured: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_checked_at: Option<OffsetDateTime>,
    pub host_count: u64,
    /// Installed version on the specific host. Present only when the `host_id`
    /// query filter is used; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    /// Plugin-provided display version for the installed version. Present only
    /// when the `host_id` query filter is used and the plugin provides one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_display_version: Option<String>,
    /// Latest known version derived as the maximum across all hosts'
    /// `latest_version` values. `None` when no host has a known latest version yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Intentionally left dynamic: payload shape is plugin-defined at the REST boundary.
    /// Present only when the `host_id` query filter is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_release_metadata: Option<serde_json::Value>,
    /// `true` when at least one assigned host has an `installed_version` that differs
    /// from its per-host `latest_version` (and both values are known). Uses string
    /// equality — no semver parsing — because version formats are plugin-specific.
    pub update_available: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// Optional HTTPS URL to an icon/logo image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}
#[derive(Serialize, Deserialize)]
pub struct SoftwareItemDetailResponse {
    pub id: Uuid,
    pub name: String,
    /// Distinct plugin type identifiers from all active host assignments.
    pub plugins: Vec<String>,
    pub featured: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_checked_at: Option<OffsetDateTime>,
    pub host_count: u64,
    /// Latest known version derived as the maximum across all hosts' `latest_version` values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// `true` when any assigned host has a known `installed_version` that differs from
    /// its per-host `latest_version`.
    pub update_available: bool,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    /// Optional HTTPS URL to an icon/logo image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub hosts: Vec<SoftwareItemHostSummary>,
}
#[derive(Serialize, Deserialize)]
pub struct SoftwareItemHostSummary {
    /// Primary key of the `host_software_items` row — unique per link even when the same
    /// host appears multiple times (e.g. two Docker containers from the same image).
    pub id: Uuid,
    pub host_id: Uuid,
    pub hostname: String,
    pub friendly_name: String,
    /// Disambiguates multiple links between the same host and software item
    /// (e.g. different Docker container names sharing the same image).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    /// Role-specific plugin assignments for this host-software pair.
    pub plugins: Vec<HostPluginRoleSummary>,
    pub installed_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub installed_version_detected_at: Option<OffsetDateTime>,
    /// Plugin-provided display version for the installed version (e.g. Docker image publish date).
    /// `None` when the installed version is self-explanatory (semver, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_display_version: Option<String>,
    /// Per-host latest known version (from the `fetch_releases` role plugin).
    /// `None` when no upstream version has been resolved yet for this host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Intentionally left dynamic: payload shape is plugin-defined at the REST boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_release_metadata: Option<serde_json::Value>,
    /// `true` when `installed_version` and `latest_version` are both `Some` and differ.
    pub update_available: bool,
    /// ID of the currently active (queued / pending / in_progress) update for this host,
    /// if any. `None` when no update is running. Used by the UI to show a contextual
    /// status badge and open the live terminal instead of the update confirmation dialog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_update_history_id: Option<Uuid>,
    /// Classification of the available update (security, bugfix, feature, unknown).
    pub update_category: String,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    pub linked_at: OffsetDateTime,
}
/// Summary of a plugin role assignment on a host-software pair (read-only).
///
/// When the assignment was created via autodiscovery (package managers),
/// `plugin_config_id` and `plugin_config_name` are `None` — the plugin type
/// is read directly from the HSIP row's `plugin_type` column.
#[derive(Serialize, Deserialize)]
pub struct HostPluginRoleSummary {
    pub role: PluginRole,
    /// Ordinal (0-based) for hook roles; always 0 for non-hook roles.
    #[serde(default)]
    pub ordinal: i32,
    /// `None` for autodiscovered package-manager assignments (no stored config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_config_id: Option<Uuid>,
    /// `None` when `plugin_config_id` is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_config_name: Option<String>,
    pub plugin_type: String,
    pub package_identifier: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_override: Option<JsonObjectMap>,
    pub execution_site: String,
}
/// Status returned when triggering an update.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerUpdateStatus {
    /// Agent connected, update sent.
    Pending,
    /// Agent offline, will deliver on reconnect.
    Queued,
    /// Update failed on the controller before any agent execution started.
    Failed,
}
impl std::fmt::Display for TriggerUpdateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("pending"),
            Self::Queued => f.write_str("queued"),
            Self::Failed => f.write_str("failed"),
        }
    }
}
/// Release asset information for triggering an update.
#[derive(Serialize, Deserialize)]
pub struct ReleaseAssetInfoRequest {
    pub name: String,
    pub download_url: String,
    pub size: Option<u64>,
}
/// Release information for triggering an update.
#[derive(Serialize, Deserialize)]
pub struct ReleaseInfoRequest {
    pub tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ReleaseAssetInfoRequest>,
}
/// Request body for triggering a software update.
#[derive(Serialize, Deserialize)]
pub struct TriggerUpdateRequest {
    /// Target version to update to.
    pub to_version: String,
    /// Optional release information (for plugins that need it).
    pub release_info: Option<ReleaseInfoRequest>,
    /// When true, the agent allocates a PTY and keeps stdin open for forwarding.
    #[serde(default)]
    pub interactive: bool,
}
/// Response when triggering a software update.
#[derive(Serialize, Deserialize)]
pub struct TriggerUpdateResponse {
    pub update_history_id: Uuid,
    pub status: TriggerUpdateStatus,
}
/// Response when triggering a version check for a software item.
#[derive(Debug, Serialize, Deserialize)]
pub struct TriggerVersionCheckResponse {
    /// Number of agents that were sent version-check messages.
    pub agents_notified: u32,
    /// Number of controller-side `fetch_releases` checks that ran synchronously.
    ///
    /// Non-zero when at least one `fetch_releases` plugin has
    /// `ControllerSideFetchReleases` capability (e.g. GitHub, Docker) and ran
    /// directly on the controller rather than being delegated to an agent.
    #[serde(default)]
    pub controller_checks_run: u32,
    /// Human-readable status message.
    pub message: String,
}
/// Query parameters for listing software items, extending pagination with an optional
/// featured filter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListSoftwareItemsParams {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
    /// Filter by featured status. Omit to return all items.
    pub featured: Option<bool>,
    /// Filter by host — only return software items assigned to this host.
    pub host_id: Option<Uuid>,
    /// Filter by update availability.
    ///
    /// - `true`: only items where at least one active host has an update available
    ///   (`installed_version != latest_version`, both non-null).
    /// - `false`: only items where no active host has an update available.
    /// - Omit: no filter.
    pub updatable: Option<bool>,
    /// Filter by plugin type — only return items that have at least one host
    /// assignment using this plugin type (e.g. `"releases_docker"`).
    /// Omit to return items for any plugin type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_type: Option<String>,
    /// Free-text search query applied against item name and related metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
}
impl ListSoftwareItemsParams {
    /// Convert the pagination fields to a [`PaginationParams`] for resolution.
    pub fn pagination(&self) -> PaginationParams {
        PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}
/// Compact summary of a software item used by merge preview responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSoftwareItemSummary {
    pub id: Uuid,
    pub name: String,
    pub host_count: u64,
    pub plugins: Vec<String>,
}
/// Compact summary of a host-software link affected by a merge preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSoftwareItemLinkSummary {
    pub id: Uuid,
    pub host_id: Uuid,
    pub hostname: String,
    pub friendly_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
}
/// Request payload for previewing a manual merge of software items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSoftwareItemsPreviewRequest {
    pub candidate_ids: Vec<Uuid>,
    pub survivor_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_item_id: Option<Uuid>,
}
/// Response payload for previewing a manual merge of software items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSoftwareItemsPreviewResponse {
    pub candidates: Vec<MergeSoftwareItemSummary>,
    pub survivor: MergeSoftwareItemSummary,
    pub losers: Vec<MergeSoftwareItemSummary>,
    pub moved_links: Vec<MergeSoftwareItemLinkSummary>,
    pub skipped_duplicate_links: Vec<MergeSoftwareItemLinkSummary>,
    pub candidate_count: u64,
    pub loser_count: u64,
    pub moved_link_count: u64,
    pub skipped_duplicate_link_count: u64,
}
/// Request payload for executing a manual merge of software items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSoftwareItemsExecuteRequest {
    pub candidate_ids: Vec<Uuid>,
    pub survivor_id: Uuid,
}
/// Response payload for executing a manual merge of software items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeSoftwareItemsExecuteResponse {
    pub survivor_id: Uuid,
    pub deleted_ids: Vec<Uuid>,
    pub moved_link_ids: Vec<Uuid>,
    pub skipped_duplicate_link_ids: Vec<Uuid>,
}
impl Validate for CreateSoftwareItemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError {
                field: "name",
                message: "name must not be empty".to_string(),
            });
        }
        if let Some(url) = &self.icon_url {
            validate_https_icon_url(url)?;
        }
        Ok(())
    }
}
impl Validate for UpdateSoftwareItemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let IconUrlPatch::Set(url) = &self.icon_url {
            validate_https_icon_url(url)?;
        }
        Ok(())
    }
}
