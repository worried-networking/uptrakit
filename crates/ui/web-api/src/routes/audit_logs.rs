//! HTTP handlers for audit log endpoints.
//!
//! Two endpoints are provided:
//!
//! - `GET /api/v1/audit-logs` — tenant-scoped entries, requires [`Permission::ViewAuditLogs`].
//! - `GET /api/v1/system-audit-logs` — system-level entries, requires
//!   [`Permission::ViewSystemAuditLogs`].

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::api_error::ApiError;
use crate::app_state::DbState;
use crate::middleware::permission::{CanViewAuditLogs, CanViewSystemAuditLogs};
use crate::queries::audit_logs as audit_log_queries;
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
        ("actor_type" = Option<String>, Query, description = "Filter by actor type: user, api_token, oidc, service, system"),
        ("action_type" = Option<String>, Query, description = "Filter by semantic action type"),
        ("outcome" = Option<String>, Query, description = "Filter by action outcome"),
        ("target_type" = Option<String>, Query, description = "Filter by semantic target type"),
        ("target_id" = Option<String>, Query, description = "Filter by semantic target id"),
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
#[tracing::instrument(skip_all)]
pub async fn list_audit_logs(
    tenant_db: TenantDb,
    CanViewAuditLogs(_user): CanViewAuditLogs,
    Query(params): Query<AuditLogListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = audit_log_queries::list_tenant_audit_logs(&tenant_db, &params).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// List system-level audit log entries
#[utoipa::path(
    get,
    path = "/api/v1/system-audit-logs",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("actor_type" = Option<String>, Query, description = "Filter by actor type: user, api_token, oidc, service, system"),
        ("action_type" = Option<String>, Query, description = "Filter by semantic action type"),
        ("outcome" = Option<String>, Query, description = "Filter by action outcome"),
        ("target_type" = Option<String>, Query, description = "Filter by semantic target type"),
        ("target_id" = Option<String>, Query, description = "Filter by semantic target id"),
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
#[tracing::instrument(skip_all)]
pub async fn list_system_audit_logs(
    State(db): State<DbState>,
    CanViewSystemAuditLogs(_user): CanViewSystemAuditLogs,
    Query(params): Query<AuditLogListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = audit_log_queries::list_system_audit_logs(db.db(), &params).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}
