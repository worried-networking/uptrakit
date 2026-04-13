use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_shared_types::{PluginRole, PluginTypeId};
use uuid::Uuid;

use crate::pagination::PaginationParams;
use crate::plugin_configs::CreatePluginConfigRequest;
use crate::validation::{Validate, ValidationError};

fn default_execution_site() -> String {
    "auto".to_string()
}

/// Create a new software item (catalog entry only — no plugin coupling).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateSoftwareItemRequest {
    /// Display name (e.g. "1Password").
    pub name: String,
    /// Whether this item is featured (shown prominently). Defaults to true for manual creation.
    #[serde(default = "crate::default_featured")]
    pub featured: bool,
    /// Optional HTTPS URL to an icon/logo image for this software item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

/// Partial update for a software item. Only `name` and `featured` are updatable.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSoftwareItemRequest {
    pub name: Option<String>,
    pub featured: Option<bool>,
    /// Set, clear, or keep the icon URL.
    ///
    /// - Absent / `None` JSON key: keep existing value.
    /// - `null`: clear the icon URL.
    /// - String: set a new HTTPS URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<serde_json::Value>,
}

/// Per-host plugin assignment used when assigning hosts to a software item.
///
/// Each host assignment contains a list of role-specific plugin assignments.
/// At minimum, a `detect_version` role should be provided for version tracking.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostSoftwareAssignment {
    pub host_id: Uuid,
    /// Role-specific plugin assignments for this host-software pair.
    pub plugins: Vec<HostPluginRoleAssignment>,
}

/// A plugin assignment for a specific role on a host-software pair.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    pub config_override: Option<serde_json::Value>,
    /// Controls where this plugin's operation is executed.
    /// - `"auto"`: system decides based on plugin capabilities (default)
    /// - `"agent"`: always run on the agent
    /// - `"controller"`: always run on the controller (only valid for `fetch_releases`)
    #[serde(default = "default_execution_site")]
    pub execution_site: String,
}

/// Assign one or more hosts to a software item, each with its own plugin info.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AssignHostsRequest {
    pub host_assignments: Vec<HostSoftwareAssignment>,
}

/// Update a single role assignment for an existing host–software-item pair.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    /// Send `null` to clear the override, an object to set it.
    pub config_override: Option<serde_json::Value>,
    /// Controls where this plugin's operation is executed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_site: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemResponse {
    pub id: Uuid,
    pub name: String,
    /// Distinct plugin type identifiers from all active host assignments (for display in lists).
    pub plugins: Vec<String>,
    pub featured: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
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
    /// Rich release metadata (notes, date, assets) from the latest fetch. Present
    /// only when the `host_id` query filter is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_release_metadata: Option<serde_json::Value>,
    /// `true` when at least one assigned host has an `installed_version` that differs
    /// from its per-host `latest_version` (and both values are known). Uses string
    /// equality — no semver parsing — because version formats are plugin-specific.
    pub update_available: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    /// Optional HTTPS URL to an icon/logo image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SoftwareItemDetailResponse {
    pub id: Uuid,
    pub name: String,
    /// Distinct plugin type identifiers from all active host assignments.
    pub plugins: Vec<String>,
    pub featured: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_checked_at: Option<OffsetDateTime>,
    pub host_count: u64,
    /// Latest known version derived as the maximum across all hosts' `latest_version` values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// `true` when any assigned host has a known `installed_version` that differs from
    /// its per-host `latest_version`.
    pub update_available: bool,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub updated_at: OffsetDateTime,
    /// Optional HTTPS URL to an icon/logo image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    pub hosts: Vec<SoftwareItemHostSummary>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub installed_version_detected_at: Option<OffsetDateTime>,
    /// Plugin-provided display version for the installed version (e.g. Docker image publish date).
    /// `None` when the installed version is self-explanatory (semver, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_display_version: Option<String>,
    /// Per-host latest known version (from the `fetch_releases` role plugin).
    /// `None` when no upstream version has been resolved yet for this host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Rich release metadata (notes, date, assets) from the latest fetch.
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
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, format = DateTime))]
    pub last_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub linked_at: OffsetDateTime,
}

/// Summary of a plugin role assignment on a host-software pair (read-only).
///
/// When the assignment was created via autodiscovery (package managers),
/// `plugin_config_id` and `plugin_config_name` are `None` — the plugin type
/// is read directly from the HSIP row's `plugin_type` column.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
    pub config_override: Option<serde_json::Value>,
    pub execution_site: String,
}

/// Status returned when triggering an update.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TriggerUpdateStatus {
    /// Agent connected, update sent.
    Pending,
    /// Agent offline, will deliver on reconnect.
    Queued,
}

impl std::fmt::Display for TriggerUpdateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("pending"),
            Self::Queued => f.write_str("queued"),
        }
    }
}

/// Release asset information for triggering an update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleaseAssetInfoRequest {
    pub name: String,
    pub download_url: String,
    pub size: Option<u64>,
}

/// Release information for triggering an update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReleaseInfoRequest {
    pub tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ReleaseAssetInfoRequest>,
}

/// Request body for triggering a software update.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerUpdateResponse {
    pub update_history_id: Uuid,
    pub status: TriggerUpdateStatus,
}

/// Response when triggering a version check for a software item.
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemSummary {
    pub id: Uuid,
    pub name: String,
    pub host_count: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<String>,
}

/// Compact summary of a host-software link affected by a merge preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsPreviewRequest {
    pub candidate_ids: Vec<Uuid>,
    pub survivor_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_item_id: Option<Uuid>,
}

/// Response payload for previewing a manual merge of software items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsExecuteRequest {
    pub candidate_ids: Vec<Uuid>,
    pub survivor_id: Uuid,
}

/// Response payload for executing a manual merge of software items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MergeSoftwareItemsExecuteResponse {
    pub survivor_id: Uuid,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted_ids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moved_link_ids: Vec<Uuid>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
        }
        Ok(())
    }
}

impl Validate for UpdateSoftwareItemRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(serde_json::Value::String(url)) = &self.icon_url {
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
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::plugin_ids;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    fn valid_create_request() -> CreateSoftwareItemRequest {
        CreateSoftwareItemRequest {
            name: "1Password".to_string(),
            featured: true,
            icon_url: None,
        }
    }

    // ── CreateSoftwareItemRequest serialization ──────────────────────

    #[test]
    fn create_software_item_request_round_trip() {
        let req = valid_create_request();
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: CreateSoftwareItemRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.name, "1Password");
        assert!(deserialized.featured);
    }

    #[test]
    fn create_software_item_request_default_featured_from_json() {
        let json = serde_json::json!({ "name": "Test" });
        let req: CreateSoftwareItemRequest =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert!(req.featured, "featured should default to true");
    }

    // ── CreateSoftwareItemRequest validation ─────────────────────────

    #[test]
    fn validate_valid_request_passes() {
        let req = valid_create_request();
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_empty_name_fails() {
        let req = CreateSoftwareItemRequest {
            name: "".to_string(),
            featured: true,
            icon_url: None,
        };
        let err = req
            .validate()
            .expect_err("empty name should fail validation");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn validate_whitespace_only_name_fails() {
        let req = CreateSoftwareItemRequest {
            name: "   ".to_string(),
            featured: true,
            icon_url: None,
        };
        let err = req
            .validate()
            .expect_err("whitespace-only name should fail validation");
        assert_eq!(err.field, "name");
    }

    #[test]
    fn create_software_item_icon_url_https_passes() {
        let req = CreateSoftwareItemRequest {
            name: "App".to_string(),
            featured: true,
            icon_url: Some("https://example.com/icon.png".to_string()),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_software_item_icon_url_http_rejected() {
        let req = CreateSoftwareItemRequest {
            name: "App".to_string(),
            featured: true,
            icon_url: Some("http://example.com/icon.png".to_string()),
        };
        let err = req.validate().expect_err("http URL should fail validation");
        assert_eq!(err.field, "icon_url");
    }

    #[test]
    fn create_software_item_icon_url_none_passes() {
        let req = CreateSoftwareItemRequest {
            name: "App".to_string(),
            featured: true,
            icon_url: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_software_item_icon_url_https_passes() {
        let req = UpdateSoftwareItemRequest {
            name: None,
            featured: None,
            icon_url: Some(serde_json::Value::String(
                "https://example.com/icon.png".to_string(),
            )),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_software_item_icon_url_null_clears() {
        let req = UpdateSoftwareItemRequest {
            name: None,
            featured: None,
            icon_url: Some(serde_json::Value::Null),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn update_software_item_icon_url_http_rejected() {
        let req = UpdateSoftwareItemRequest {
            name: None,
            featured: None,
            icon_url: Some(serde_json::Value::String(
                "http://example.com/icon.png".to_string(),
            )),
        };
        let err = req.validate().expect_err("http URL should fail validation");
        assert_eq!(err.field, "icon_url");
    }

    // ── AssignHostsRequest round-trip ──────────────────────────────

    #[test]
    fn assign_hosts_request_round_trip() {
        let req = AssignHostsRequest {
            host_assignments: vec![
                HostSoftwareAssignment {
                    host_id: sample_uuid(),
                    plugins: vec![
                        HostPluginRoleAssignment {
                            role: PluginRole::DetectVersion,
                            ordinal: 0,
                            plugin_config_id: Some(sample_uuid()),
                            plugin_config: None,
                            package_identifier: "1password".to_string(),
                            config_override: None,
                            execution_site: "auto".to_string(),
                        },
                        HostPluginRoleAssignment {
                            role: PluginRole::FetchReleases,
                            ordinal: 0,
                            plugin_config_id: Some(sample_uuid()),
                            plugin_config: None,
                            package_identifier: "1password".to_string(),
                            config_override: None,
                            execution_site: "auto".to_string(),
                        },
                    ],
                },
                HostSoftwareAssignment {
                    host_id: Uuid::nil(),
                    plugins: vec![HostPluginRoleAssignment {
                        role: PluginRole::ExecuteUpdate,
                        ordinal: 0,
                        plugin_config_id: None,
                        plugin_config: Some(crate::plugin_configs::CreatePluginConfigRequest {
                            name: "Homebrew Casks".to_string(),
                            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                            config: serde_json::json!({"package_type": "cask"}),
                            enabled: true,
                        }),
                        package_identifier: "1password-cli".to_string(),
                        config_override: None,
                        execution_site: "agent".to_string(),
                    }],
                },
            ],
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: AssignHostsRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.host_assignments.len(), 2);
        assert_eq!(deserialized.host_assignments[0].host_id, sample_uuid());
        assert_eq!(deserialized.host_assignments[0].plugins.len(), 2);
        assert_eq!(
            deserialized.host_assignments[0].plugins[0].package_identifier,
            "1password"
        );
        assert_eq!(deserialized.host_assignments[1].plugins.len(), 1);
        assert!(
            deserialized.host_assignments[1].plugins[0]
                .plugin_config
                .is_some()
        );
    }

    #[test]
    fn host_plugin_role_assignment_defaults_execution_site() {
        let json = serde_json::json!({
            "role": "detect_version",
            "plugin_config_id": sample_uuid(),
            "package_identifier": "nginx"
        });
        let assignment: HostPluginRoleAssignment =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert_eq!(assignment.execution_site, "auto");
        assert_eq!(assignment.role, PluginRole::DetectVersion);
    }

    #[test]
    fn update_host_assignment_request_round_trip() {
        let req = UpdateHostAssignmentRequest {
            role: PluginRole::FetchReleases,
            ordinal: 0,
            plugin_config_id: Some(sample_uuid()),
            plugin_config: None,
            plugin_type: None,
            package_identifier: Some("nginx".to_string()),
            config_override: None,
            execution_site: Some("controller".to_string()),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateHostAssignmentRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.role, PluginRole::FetchReleases);
        assert_eq!(deserialized.execution_site.as_deref(), Some("controller"));
    }

    // ── SoftwareItemResponse ─────────────────────────────────────────

    #[test]
    fn software_item_response_round_trip() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "1Password".to_string(),
            plugins: vec![
                "package_manager_homebrew".to_string(),
                "releases_github".to_string(),
            ],
            featured: true,
            last_checked_at: Some(datetime!(2025-06-01 12:00:00 UTC)),
            host_count: 5,
            installed_version: Some("8.9.0".to_string()),
            installed_display_version: None,
            latest_version: Some("8.10.0".to_string()),
            latest_release_metadata: None,
            update_available: true,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
            icon_url: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.name, "1Password");
        assert_eq!(deserialized.host_count, 5);
        assert_eq!(deserialized.plugins.len(), 2);
        assert!(deserialized.featured);
        assert_eq!(deserialized.installed_version.as_deref(), Some("8.9.0"));
        assert_eq!(deserialized.latest_version.as_deref(), Some("8.10.0"));
        assert!(deserialized.update_available);
    }

    #[test]
    fn software_item_response_update_available_false_when_no_latest() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "MyApp".to_string(),
            plugins: vec!["releases_github".to_string()],
            featured: true,
            last_checked_at: None,
            host_count: 1,
            installed_version: None,
            installed_display_version: None,
            latest_version: None,
            latest_release_metadata: None,
            update_available: false,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            icon_url: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.installed_version.is_none());
        assert!(deserialized.latest_version.is_none());
        assert!(!deserialized.update_available);
        // installed_version and latest_version are skipped when None
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert!(json_value.get("installed_version").is_none());
        assert!(json_value.get("latest_version").is_none());
    }

    #[test]
    fn software_item_response_empty_plugins() {
        use time::macros::datetime;
        let resp = SoftwareItemResponse {
            id: sample_uuid(),
            name: "Test".to_string(),
            plugins: vec![],
            featured: false,
            last_checked_at: None,
            host_count: 0,
            installed_version: None,
            installed_display_version: None,
            latest_version: None,
            latest_release_metadata: None,
            update_available: false,
            created_at: datetime!(2025-01-01 00:00:00 UTC),
            updated_at: datetime!(2025-01-01 00:00:00 UTC),
            icon_url: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SoftwareItemResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.plugins.is_empty());
        assert!(deserialized.last_checked_at.is_none());
        assert!(!deserialized.featured);
        assert!(!deserialized.update_available);
    }

    // ── TriggerUpdateRequest / TriggerUpdateResponse ─────────────────

    #[test]
    fn trigger_update_request_round_trip() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: None,
            interactive: false,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: TriggerUpdateRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.to_version, "2.0.0");
        assert!(deserialized.release_info.is_none());
    }

    #[test]
    fn trigger_update_request_with_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "3.0.0".to_string(),
            release_info: Some(ReleaseInfoRequest {
                tag: "v3.0.0".to_string(),
                release_url: "https://github.com/example/repo/releases/v3.0.0".to_string(),
                assets: vec![ReleaseAssetInfoRequest {
                    name: "binary.tar.gz".to_string(),
                    download_url: "https://example.com/binary.tar.gz".to_string(),
                    size: Some(1024),
                }],
            }),
            interactive: false,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: TriggerUpdateRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        let info = deserialized
            .release_info
            .expect("release_info should be present");
        assert_eq!(info.tag, "v3.0.0");
        assert_eq!(info.assets.len(), 1);
        assert_eq!(info.assets[0].size, Some(1024));
    }

    #[test]
    fn trigger_update_response_round_trip() {
        let resp = TriggerUpdateResponse {
            update_history_id: sample_uuid(),
            status: TriggerUpdateStatus::Pending,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerUpdateResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.update_history_id, sample_uuid());
        assert_eq!(deserialized.status, TriggerUpdateStatus::Pending);
    }

    #[test]
    fn trigger_update_response_queued_status() {
        let resp = TriggerUpdateResponse {
            update_history_id: sample_uuid(),
            status: TriggerUpdateStatus::Queued,
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("status").and_then(|v| v.as_str()),
            Some("queued")
        );
    }

    // ── TriggerVersionCheckResponse ──────────────────────────────────

    #[test]
    fn trigger_version_check_response_round_trip() {
        let resp = TriggerVersionCheckResponse {
            agents_notified: 3,
            controller_checks_run: 0,
            message: "Version check triggered for 3 agents".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerVersionCheckResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents_notified, 3);
        assert_eq!(deserialized.controller_checks_run, 0);
        assert_eq!(deserialized.message, "Version check triggered for 3 agents");
    }

    #[test]
    fn trigger_version_check_response_controller_only() {
        let resp = TriggerVersionCheckResponse {
            agents_notified: 0,
            controller_checks_run: 2,
            message: "Version check completed for 2 item(s) on the controller".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerVersionCheckResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents_notified, 0);
        assert_eq!(deserialized.controller_checks_run, 2);
    }

    #[test]
    fn trigger_version_check_response_controller_checks_run_defaults_to_zero() {
        // Old JSON without controller_checks_run should deserialize with default 0.
        let json = r#"{"agents_notified":1,"message":"ok"}"#;
        let deserialized: TriggerVersionCheckResponse =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents_notified, 1);
        assert_eq!(deserialized.controller_checks_run, 0);
    }

    #[test]
    fn trigger_version_check_response_zero_agents() {
        let resp = TriggerVersionCheckResponse {
            agents_notified: 0,
            controller_checks_run: 0,
            message: "No agents connected".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: TriggerVersionCheckResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents_notified, 0);
        assert_eq!(deserialized.controller_checks_run, 0);
    }

    // ── TriggerUpdateStatus Display ──────────────────────────────────

    #[test]
    fn trigger_update_status_display() {
        assert_eq!(TriggerUpdateStatus::Pending.to_string(), "pending");
        assert_eq!(TriggerUpdateStatus::Queued.to_string(), "queued");
    }

    // ── ListSoftwareItemsParams ──────────────────────────────────────

    #[test]
    fn list_software_items_params_featured_filter() {
        let json = serde_json::json!({ "featured": true });
        let params: ListSoftwareItemsParams =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert_eq!(params.featured, Some(true));
    }

    #[test]
    fn list_software_items_params_no_filter() {
        let params = ListSoftwareItemsParams::default();
        assert!(params.featured.is_none());
        assert!(params.page.is_none());
        assert!(params.per_page.is_none());
    }

    #[test]
    fn list_software_items_params_updatable_filter() {
        let json = serde_json::json!({ "updatable": true });
        let params: ListSoftwareItemsParams =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert_eq!(params.updatable, Some(true));
    }

    #[test]
    fn list_software_items_params_plugin_type_filter() {
        let json = serde_json::json!({ "plugin_type": "releases_docker" });
        let params: ListSoftwareItemsParams =
            serde_json::from_value(json).expect("deserialization should succeed");
        assert_eq!(params.plugin_type.as_deref(), Some("releases_docker"));
    }

    #[test]
    fn list_software_items_params_query_filter() {
        let params: ListSoftwareItemsParams =
            serde_json::from_str(r#"{"query":"node","plugin_type":"releases_docker"}"#)
                .expect("deserialize");
        assert_eq!(params.query.as_deref(), Some("node"));
        assert_eq!(params.plugin_type.as_deref(), Some("releases_docker"));
    }

    #[test]
    fn merge_preview_request_round_trip() {
        let req = MergeSoftwareItemsPreviewRequest {
            candidate_ids: vec![Uuid::nil(), Uuid::new_v4()],
            survivor_id: Uuid::nil(),
            seed_item_id: Some(Uuid::new_v4()),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        let parsed: MergeSoftwareItemsPreviewRequest =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.candidate_ids.len(), 2);
        assert_eq!(parsed.survivor_id, Uuid::nil());
    }

    #[test]
    fn merge_preview_response_round_trip() {
        let resp = MergeSoftwareItemsPreviewResponse {
            candidates: vec![MergeSoftwareItemSummary {
                id: Uuid::nil(),
                name: "Node.js".to_string(),
                host_count: 2,
                plugins: vec!["releases_github".to_string()],
            }],
            survivor: MergeSoftwareItemSummary {
                id: Uuid::new_v4(),
                name: "Node.js LTS".to_string(),
                host_count: 4,
                plugins: vec!["releases_github".to_string()],
            },
            losers: vec![MergeSoftwareItemSummary {
                id: Uuid::new_v4(),
                name: "Node".to_string(),
                host_count: 1,
                plugins: vec![],
            }],
            moved_links: vec![MergeSoftwareItemLinkSummary {
                id: Uuid::new_v4(),
                host_id: Uuid::new_v4(),
                hostname: "host-a".to_string(),
                friendly_name: "Host A".to_string(),
                qualifier: None,
            }],
            skipped_duplicate_links: vec![MergeSoftwareItemLinkSummary {
                id: Uuid::new_v4(),
                host_id: Uuid::new_v4(),
                hostname: "host-b".to_string(),
                friendly_name: "Host B".to_string(),
                qualifier: Some("docker".to_string()),
            }],
            candidate_count: 1,
            loser_count: 1,
            moved_link_count: 1,
            skipped_duplicate_link_count: 1,
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let parsed: MergeSoftwareItemsPreviewResponse =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.losers.len(), 1);
        assert_eq!(parsed.moved_links.len(), 1);
        assert_eq!(parsed.candidate_count, 1);
        assert_eq!(parsed.loser_count, 1);
        assert_eq!(parsed.moved_link_count, 1);
        assert_eq!(parsed.skipped_duplicate_link_count, 1);
    }

    #[test]
    fn merge_execute_response_round_trip() {
        let resp = MergeSoftwareItemsExecuteResponse {
            survivor_id: Uuid::nil(),
            deleted_ids: vec![Uuid::new_v4()],
            moved_link_ids: vec![Uuid::new_v4()],
            skipped_duplicate_link_ids: vec![Uuid::new_v4()],
        };
        let json = serde_json::to_string(&resp).expect("serialize");
        let parsed: MergeSoftwareItemsExecuteResponse =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.deleted_ids.len(), 1);
    }
}
