use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// A single tenant-scoped audit log entry, returned by
/// `GET /api/v1/audit-logs`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuditLogResponse {
    /// Unique identifier of this audit log entry.
    pub id: Uuid,
    /// Identifier of the actor who made the request.
    pub actor_id: Uuid,
    /// Type of actor: `"user"`, `"api_token"`, or `"oidc"`.
    pub actor_type: String,
    /// Authentication method used: `"password"`, `"oidc"`, or `"api_token"`.
    pub auth_method: String,
    /// HTTP method of the request (e.g. `"GET"`, `"POST"`).
    pub http_method: String,
    /// Full request path.
    pub http_path: String,
    /// Matched router pattern, if available (e.g. `/api/v1/hosts/:id`).
    pub route_pattern: Option<String>,
    /// HTTP response status code.
    pub http_status: u16,
    /// Client IP address, if available.
    pub client_ip: Option<String>,
    /// `User-Agent` header value, if present.
    pub user_agent: Option<String>,
    /// Request duration in milliseconds.
    pub duration_ms: u64,
    /// Timestamp when the request was processed (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub occurred_at: OffsetDateTime,
}

/// A single system-level audit log entry, returned by
/// `GET /api/v1/system-audit-logs`.
///
/// Contains the same fields as [`AuditLogResponse`] but represents
/// infrastructure-scoped operations (global settings changes, CA rotation,
/// etc.) that are not associated with any tenant.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemAuditLogResponse {
    /// Unique identifier of this audit log entry.
    pub id: Uuid,
    /// Identifier of the actor who made the request.
    pub actor_id: Uuid,
    /// Type of actor: `"user"`, `"api_token"`, or `"oidc"`.
    pub actor_type: String,
    /// Authentication method used: `"password"`, `"oidc"`, or `"api_token"`.
    pub auth_method: String,
    /// HTTP method of the request (e.g. `"GET"`, `"POST"`).
    pub http_method: String,
    /// Full request path.
    pub http_path: String,
    /// Matched router pattern, if available.
    pub route_pattern: Option<String>,
    /// HTTP response status code.
    pub http_status: u16,
    /// Client IP address, if available.
    pub client_ip: Option<String>,
    /// `User-Agent` header value, if present.
    pub user_agent: Option<String>,
    /// Request duration in milliseconds.
    pub duration_ms: u64,
    /// Timestamp when the request was processed (RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(feature = "openapi", schema(value_type = String, format = DateTime))]
    pub occurred_at: OffsetDateTime,
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Query parameters for listing audit log entries (tenant-scoped or system).
///
/// All filters are optional; omitting a filter returns all entries. When
/// multiple filters are provided they are combined with AND.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct AuditLogListParams {
    /// Page number (1-based). Defaults to 1.
    pub page: Option<u64>,
    /// Number of entries per page. Defaults to 25, maximum 200.
    pub per_page: Option<u64>,
    /// Filter by actor type: `"user"`, `"api_token"`, or `"oidc"`.
    pub actor_type: Option<String>,
    /// Filter by HTTP method: `"GET"`, `"POST"`, `"PUT"`, `"DELETE"`, `"PATCH"`.
    pub method: Option<String>,
    /// Filter by exact HTTP status code (e.g. `200`, `403`, `500`).
    pub status: Option<u16>,
    /// Lower bound timestamp (inclusive), RFC 3339 format.
    pub from: Option<String>,
    /// Upper bound timestamp (inclusive), RFC 3339 format.
    pub to: Option<String>,
    /// Filter entries by a specific actor UUID.
    pub actor_id: Option<Uuid>,
}
