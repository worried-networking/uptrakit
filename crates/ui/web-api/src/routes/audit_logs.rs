//! HTTP handlers for audit log endpoints.
//!
//! Two endpoints are provided:
//!
//! - `GET /api/v1/audit-logs` — tenant-scoped entries, requires [`Permission::ViewAuditLogs`].
//! - `GET /api/v1/system-audit-logs` — system-level entries, requires
//!   [`Permission::ViewSystemAuditLogs`].

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::{CanViewAuditLogs, CanViewSystemAuditLogs};
use crate::queries::audit_logs::{self as audit_log_queries, AuditLogQueryError};
use crate::tenant_db::TenantDb;

pub use uptrakit_web_api_types::audit_logs::{
    AuditLogListParams, AuditLogResponse, SystemAuditLogResponse,
};
pub use uptrakit_web_api_types::pagination::PaginatedResponse;

// ---------------------------------------------------------------------------
// Endpoints
// ---------------------------------------------------------------------------

/// List tenant-scoped audit log entries
#[utoipa::path(
    get,
    path = "/api/v1/audit-logs",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("actor_type" = Option<String>, Query, description = "Filter by actor type: user, api_token, oidc"),
        ("method" = Option<String>, Query, description = "Filter by HTTP method: GET, POST, PUT, DELETE, PATCH"),
        ("status" = Option<u16>, Query, description = "Filter by exact HTTP status code"),
        ("from" = Option<String>, Query, description = "Lower bound timestamp (inclusive), RFC 3339"),
        ("to" = Option<String>, Query, description = "Upper bound timestamp (inclusive), RFC 3339"),
        ("actor_id" = Option<uuid::Uuid>, Query, description = "Filter by actor UUID")
    ),
    responses(
        (status = 200, description = "Paginated list of tenant audit log entries", body = PaginatedResponse<AuditLogResponse>),
        (status = 400, description = "Invalid filter parameter"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Audit Logs",
    extensions(("x-required-permission" = json!("view_audit_logs"))),
    security(("bearer_token" = []))
)]
pub async fn list_audit_logs(
    tenant_db: TenantDb,
    CanViewAuditLogs(_user): CanViewAuditLogs,
    Query(params): Query<AuditLogListParams>,
) -> Response {
    match audit_log_queries::list_tenant_audit_logs(&tenant_db, &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(ref e) if matches!(e.current_context(), AuditLogQueryError::InvalidFilter(_)) => {
            error_response(StatusCode::BAD_REQUEST, format!("{}", e.current_context()))
        }
        Err(e) => {
            tracing::error!("Failed to list audit logs: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// List system-level audit log entries
#[utoipa::path(
    get,
    path = "/api/v1/system-audit-logs",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("actor_type" = Option<String>, Query, description = "Filter by actor type: user, api_token, oidc"),
        ("method" = Option<String>, Query, description = "Filter by HTTP method: GET, POST, PUT, DELETE, PATCH"),
        ("status" = Option<u16>, Query, description = "Filter by exact HTTP status code"),
        ("from" = Option<String>, Query, description = "Lower bound timestamp (inclusive), RFC 3339"),
        ("to" = Option<String>, Query, description = "Upper bound timestamp (inclusive), RFC 3339"),
        ("actor_id" = Option<uuid::Uuid>, Query, description = "Filter by actor UUID")
    ),
    responses(
        (status = 200, description = "Paginated list of system audit log entries", body = PaginatedResponse<SystemAuditLogResponse>),
        (status = 400, description = "Invalid filter parameter"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Audit Logs",
    extensions(("x-required-permission" = json!("view_system_audit_logs"))),
    security(("bearer_token" = []))
)]
pub async fn list_system_audit_logs(
    State(state): State<Arc<AppState>>,
    CanViewSystemAuditLogs(_user): CanViewSystemAuditLogs,
    Query(params): Query<AuditLogListParams>,
) -> Response {
    match audit_log_queries::list_system_audit_logs(state.db(), &params).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(ref e) if matches!(e.current_context(), AuditLogQueryError::InvalidFilter(_)) => {
            error_response(StatusCode::BAD_REQUEST, format!("{}", e.current_context()))
        }
        Err(e) => {
            tracing::error!("Failed to list system audit logs: {}", e);
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
