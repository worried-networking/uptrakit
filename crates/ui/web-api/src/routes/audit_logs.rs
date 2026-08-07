//! HTTP handlers for audit log endpoints.
//!
//! Two endpoints are provided:
//!
//! - `GET /api/v1/audit-logs` — tenant-scoped entries, gated by [`CanReadAudit`]
//!   (`audit:read`).
//! - `GET /api/v1/system-audit-logs` — system-level entries, gated by
//!   [`CanReadSystemAudit`] (`system.audit:read`).

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::api_error::ApiError;
use crate::app_state::DbState;
use crate::middleware::action::{CanReadAudit, CanReadSystemAudit};
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
    params(AuditLogListParams),
    responses(
        (status = 200, description = "Paginated list of tenant audit log entries", body = PaginatedResponse<AuditLogResponse>),
        (status = 400, description = "Invalid filter parameter"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Audit Logs",
    security(("oauth2" = ["audit:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_audit_logs(
    tenant_db: TenantDb,
    CanReadAudit(_user): CanReadAudit,
    Query(params): Query<AuditLogListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = audit_log_queries::list_tenant_audit_logs(&tenant_db, &params).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// List system-level audit log entries
#[utoipa::path(
    get,
    path = "/api/v1/system-audit-logs",
    params(AuditLogListParams),
    responses(
        (status = 200, description = "Paginated list of system audit log entries", body = PaginatedResponse<SystemAuditLogResponse>),
        (status = 400, description = "Invalid filter parameter"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Audit Logs",
    security(("oauth2" = ["system.audit:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_system_audit_logs(
    State(db): State<DbState>,
    CanReadSystemAudit(_user): CanReadSystemAudit,
    Query(params): Query<AuditLogListParams>,
) -> Result<impl IntoResponse, ApiError> {
    let resp = audit_log_queries::list_system_audit_logs(db.db(), &params).await?;
    Ok((StatusCode::OK, Json(resp)).into_response())
}
