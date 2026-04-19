use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// A single tenant-scoped semantic audit log entry, returned by
/// `GET /api/v1/audit-logs`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuditLogResponse {
    /// Unique identifier of this audit log entry.
    pub id: Uuid,
    /// Actor type: `"user"`, `"api_token"`, `"oidc"`, `"service"`, `"system"`.
    pub actor_type: String,
    /// Optional actor identifier.
    pub actor_id: Option<Uuid>,
    /// Optional human-readable actor label.
    pub actor_display: Option<String>,
    /// Canonical semantic action identifier.
    pub action_type: String,
    /// Optional semantic target type.
    pub target_type: Option<String>,
    /// Optional semantic target id.
    pub target_id: Option<String>,
    /// Optional human-readable target label.
    pub target_display: Option<String>,
    /// Action outcome.
    pub outcome: String,
    /// Optional curated structured metadata payload.
    pub details_json: Option<Value>,
    /// Optional request correlation id.
    pub request_id: Option<String>,
    /// Timestamp when the action occurred (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub occurred_at: OffsetDateTime,
}

/// A single system-level semantic audit log entry, returned by
/// `GET /api/v1/system-audit-logs`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemAuditLogResponse {
    /// Unique identifier of this audit log entry.
    pub id: Uuid,
    /// Actor type: `"user"`, `"api_token"`, `"oidc"`, `"service"`, `"system"`.
    pub actor_type: String,
    /// Optional actor identifier.
    pub actor_id: Option<Uuid>,
    /// Optional human-readable actor label.
    pub actor_display: Option<String>,
    /// Canonical semantic action identifier.
    pub action_type: String,
    /// Optional semantic target type.
    pub target_type: Option<String>,
    /// Optional semantic target id.
    pub target_id: Option<String>,
    /// Optional human-readable target label.
    pub target_display: Option<String>,
    /// Action outcome.
    pub outcome: String,
    /// Optional curated structured metadata payload.
    pub details_json: Option<Value>,
    /// Optional request correlation id.
    pub request_id: Option<String>,
    /// Timestamp when the action occurred (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub occurred_at: OffsetDateTime,
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters for listing audit log entries (tenant-scoped or system).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct AuditLogListParams {
    /// Page number (1-based). Defaults to 1.
    pub page: Option<u64>,
    /// Number of entries per page. Defaults to 25, maximum 200.
    pub per_page: Option<u64>,
    /// Filter by actor type: `"user"`, `"api_token"`, `"oidc"`, `"service"`, `"system"`.
    pub actor_type: Option<String>,
    /// Filter by semantic action type.
    pub action_type: Option<String>,
    /// Filter by action outcome.
    pub outcome: Option<String>,
    /// Filter by semantic target type.
    pub target_type: Option<String>,
    /// Filter by semantic target id.
    pub target_id: Option<String>,
    /// Lower bound timestamp (inclusive), RFC 3339 format.
    pub from: Option<String>,
    /// Upper bound timestamp (inclusive), RFC 3339 format.
    pub to: Option<String>,
    /// Filter entries by a specific actor UUID.
    pub actor_id: Option<Uuid>,
}
