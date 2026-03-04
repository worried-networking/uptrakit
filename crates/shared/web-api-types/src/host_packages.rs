use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::software_items::TriggerUpdateStatus;
use crate::update_history::UpdateStatus;
use crate::validation::{Validate, ValidationError};

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

/// Response for a host package in list views.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostPackageResponse {
    pub id: Uuid,
    pub host_id: Uuid,
    pub plugin_config_id: Uuid,
    pub package_identifier: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub installed_version_detected_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub latest_version_fetched_at: Option<OffsetDateTime>,
    pub update_category: String,
    pub enabled: bool,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub last_checked_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub last_updated_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    /// Whether an update is available (installed_version != latest_version).
    pub has_update: bool,
}

/// Detailed response for a single host package, including update history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostPackageDetailResponse {
    pub package: HostPackageResponse,
    pub plugin_config_name: String,
    pub plugin_type: String,
    pub recent_updates: Vec<HostPackageUpdateHistoryResponse>,
}

/// Response for a host package update history entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostPackageUpdateHistoryResponse {
    pub id: Uuid,
    pub host_package_id: Uuid,
    pub from_version: Option<String>,
    pub to_version: Option<String>,
    pub status: UpdateStatus,
    pub output: String,
    pub actor_type: String,
    pub actor_id: String,
    pub update_category: String,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub started_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub completed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<Uuid>,
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/// Request body for updating a host package (enable/disable).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHostPackageRequest {
    /// Enable or disable this host package.
    pub enabled: bool,
}

/// Request body for triggering an update of a single host package.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerHostPackageUpdateRequest {
    /// Target version to update to.
    pub to_version: String,
}

impl Validate for TriggerHostPackageUpdateRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.to_version.trim().is_empty() {
            return Err(ValidationError {
                field: "to_version",
                message: "to_version must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

/// Response when triggering a single host package update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerHostPackageUpdateResponse {
    pub update_history_id: Uuid,
    pub status: TriggerUpdateStatus,
}

/// Request body for triggering a batch update of host packages.
///
/// If `package_ids` is empty/absent, all outdated packages matching the
/// optional `category` filter are included.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerBatchHostPackageUpdateRequest {
    /// Only include packages with this update category (e.g. `"security"`).
    /// `None` means all outdated packages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category_filter: Option<String>,
    /// Specific package IDs to update. `None` or empty means all outdated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_ids: Option<Vec<Uuid>>,
}

impl Validate for TriggerBatchHostPackageUpdateRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(ref cat) = self.category_filter {
            let valid = ["security", "bugfix", "feature", "unknown"];
            if !valid.contains(&cat.as_str()) {
                return Err(ValidationError {
                    field: "category_filter",
                    message: format!("must be one of: {}", valid.join(", ")),
                });
            }
        }
        Ok(())
    }
}

/// Response when triggering a batch host package update.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerBatchHostPackageUpdateResponse {
    /// Batch ID for tracking the operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub batch_id: Option<Uuid>,
    /// Number of update records created.
    pub total_created: usize,
    /// Per-package update details.
    pub updates: Vec<BatchHostPackageUpdateItem>,
    /// Packages that were skipped (already in-progress, disabled, etc.).
    pub skipped: Vec<BatchHostPackageSkippedItem>,
}

/// A single item within a batch host package update response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchHostPackageUpdateItem {
    pub update_history_id: Uuid,
    pub host_package_id: Uuid,
    pub package_identifier: String,
    pub name: String,
    pub to_version: String,
    pub trigger_status: TriggerUpdateStatus,
}

/// A package that was skipped during batch creation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BatchHostPackageSkippedItem {
    pub host_package_id: Uuid,
    pub package_identifier: String,
    pub name: String,
    pub reason: String,
}

// ---------------------------------------------------------------------------
// Ignore rules
// ---------------------------------------------------------------------------

/// Response for a host package ignore rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostPackageIgnoreResponse {
    pub id: Uuid,
    pub host_id: Uuid,
    pub plugin_config_id: Uuid,
    pub package_identifier: String,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
}

/// Request body for creating a host package ignore rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CreateHostPackageIgnoreRequest {
    pub plugin_config_id: Uuid,
    pub package_identifier: String,
}

impl Validate for CreateHostPackageIgnoreRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.package_identifier.trim().is_empty() {
            return Err(ValidationError {
                field: "package_identifier",
                message: "package_identifier must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters for listing host packages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListHostPackagesParams {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
    /// Filter by enabled status.
    pub enabled: Option<bool>,
    /// Filter to packages that have an available update.
    pub has_update: Option<bool>,
    /// Filter by update category.
    pub category: Option<String>,
    /// Search by package name or identifier.
    pub search: Option<String>,
}

impl ListHostPackagesParams {
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}

// ---------------------------------------------------------------------------
// Host update summary (embedded in HostResponse)
// ---------------------------------------------------------------------------

/// Aggregate update counts for a host, computed from host_packages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostUpdateSummary {
    /// Total host packages with available updates.
    pub available_updates_count: u32,
    /// Subset where update_category = 'security'.
    pub security_updates_count: u32,
}

// ---------------------------------------------------------------------------
// Trigger version check response
// ---------------------------------------------------------------------------

/// Response when triggering a version check for host packages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TriggerHostPackageVersionCheckResponse {
    /// Number of agents notified with version check assignments.
    pub agents_notified: u32,
    /// Human-readable status message.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    fn sample_uuid_2() -> Uuid {
        Uuid::parse_str("b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6")
            .expect("hard-coded UUID should be valid")
    }

    // ── HostPackageResponse ───────────────────────────────────────────

    #[test]
    fn host_package_response_round_trip() {
        let resp = HostPackageResponse {
            id: sample_uuid(),
            host_id: sample_uuid_2(),
            plugin_config_id: sample_uuid(),
            package_identifier: "nginx".to_string(),
            name: "nginx".to_string(),
            installed_version: Some("1.22.0-1".to_string()),
            installed_version_detected_at: Some(datetime!(2025-06-01 12:00:00 UTC)),
            latest_version: Some("1.24.0-2".to_string()),
            latest_version_fetched_at: Some(datetime!(2025-06-01 13:00:00 UTC)),
            update_category: "security".to_string(),
            enabled: true,
            last_checked_at: Some(datetime!(2025-06-01 13:00:00 UTC)),
            last_updated_at: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            has_update: true,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: HostPackageResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.package_identifier, "nginx");
        assert_eq!(deserialized.installed_version.as_deref(), Some("1.22.0-1"));
        assert!(deserialized.has_update);
        assert!(deserialized.enabled);
    }

    #[test]
    fn host_package_response_none_fields_omitted() {
        let resp = HostPackageResponse {
            id: sample_uuid(),
            host_id: sample_uuid_2(),
            plugin_config_id: sample_uuid(),
            package_identifier: "curl".to_string(),
            name: "curl".to_string(),
            installed_version: None,
            installed_version_detected_at: None,
            latest_version: None,
            latest_version_fetched_at: None,
            update_category: "unknown".to_string(),
            enabled: true,
            last_checked_at: None,
            last_updated_at: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            has_update: false,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        assert!(
            !json.contains(r#""installed_version":"#),
            "installed_version should be omitted when None"
        );
        assert!(
            !json.contains(r#""latest_version":"#),
            "latest_version should be omitted when None"
        );
    }

    // ── TriggerHostPackageUpdateRequest ───────────────────────────────

    #[test]
    fn trigger_host_package_update_request_validate_valid() {
        let req = TriggerHostPackageUpdateRequest {
            to_version: "1.24.0-2".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn trigger_host_package_update_request_validate_empty() {
        let req = TriggerHostPackageUpdateRequest {
            to_version: "  ".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "to_version");
    }

    // ── TriggerBatchHostPackageUpdateRequest ──────────────────────────

    #[test]
    fn trigger_batch_request_validate_valid_category() {
        let req = TriggerBatchHostPackageUpdateRequest {
            category_filter: Some("security".to_string()),
            package_ids: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn trigger_batch_request_validate_no_filter() {
        let req = TriggerBatchHostPackageUpdateRequest {
            category_filter: None,
            package_ids: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn trigger_batch_request_validate_invalid_category() {
        let req = TriggerBatchHostPackageUpdateRequest {
            category_filter: Some("invalid".to_string()),
            package_ids: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "category_filter");
    }

    // ── CreateHostPackageIgnoreRequest ────────────────────────────────

    #[test]
    fn create_ignore_request_validate_valid() {
        let req = CreateHostPackageIgnoreRequest {
            plugin_config_id: sample_uuid(),
            package_identifier: "nginx".to_string(),
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn create_ignore_request_validate_empty_identifier() {
        let req = CreateHostPackageIgnoreRequest {
            plugin_config_id: sample_uuid(),
            package_identifier: "  ".to_string(),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "package_identifier");
    }

    // ── HostUpdateSummary ────────────────────────────────────────────

    #[test]
    fn host_update_summary_default() {
        let summary = HostUpdateSummary::default();
        assert_eq!(summary.available_updates_count, 0);
        assert_eq!(summary.security_updates_count, 0);
    }

    #[test]
    fn host_update_summary_round_trip() {
        let summary = HostUpdateSummary {
            available_updates_count: 42,
            security_updates_count: 5,
        };
        let json = serde_json::to_string(&summary).expect("serialization should succeed");
        let deserialized: HostUpdateSummary =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.available_updates_count, 42);
        assert_eq!(deserialized.security_updates_count, 5);
    }

    // ── ListHostPackagesParams ───────────────────────────────────────

    #[test]
    fn list_host_packages_params_default() {
        let params = ListHostPackagesParams::default();
        assert!(params.page.is_none());
        assert!(params.per_page.is_none());
        assert!(params.enabled.is_none());
        assert!(params.has_update.is_none());
        assert!(params.category.is_none());
        assert!(params.search.is_none());
    }

    #[test]
    fn list_host_packages_params_round_trip() {
        let params = ListHostPackagesParams {
            page: Some(2),
            per_page: Some(50),
            enabled: Some(true),
            has_update: Some(true),
            category: Some("security".to_string()),
            search: Some("nginx".to_string()),
        };
        let json = serde_json::to_string(&params).expect("serialization should succeed");
        let deserialized: ListHostPackagesParams =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.page, Some(2));
        assert_eq!(deserialized.per_page, Some(50));
        assert_eq!(deserialized.enabled, Some(true));
        assert_eq!(deserialized.has_update, Some(true));
        assert_eq!(deserialized.category.as_deref(), Some("security"));
        assert_eq!(deserialized.search.as_deref(), Some("nginx"));
    }

    // ── Batch response ───────────────────────────────────────────────

    #[test]
    fn batch_response_serializes() {
        let resp = TriggerBatchHostPackageUpdateResponse {
            batch_id: Some(sample_uuid()),
            total_created: 3,
            updates: vec![],
            skipped: vec![],
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        assert!(json.contains("\"total_created\":3"));
    }

    #[test]
    fn batch_response_no_batch_id_skips() {
        let resp = TriggerBatchHostPackageUpdateResponse {
            batch_id: None,
            total_created: 0,
            updates: vec![],
            skipped: vec![],
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        assert!(!json.contains("batch_id"));
    }
}
