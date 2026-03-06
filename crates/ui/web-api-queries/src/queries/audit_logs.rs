use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use thiserror::Error;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_db::entity::{audit_log, system_audit_log};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::audit_logs::{
    AuditLogListParams, AuditLogResponse, SystemAuditLogResponse,
};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

use crate::tenant_db::TenantDb;

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors returned by audit log queries.
#[derive(Debug, Error)]
pub enum AuditLogQueryError {
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),
    /// A filter parameter could not be parsed.
    #[error("invalid filter: {0}")]
    InvalidFilter(String),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<AuditLogQueryError>>;
impl_report_conversion!(sea_orm::DbErr => AuditLogQueryError::Database);

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

fn resolve_pagination(
    params: &AuditLogListParams,
) -> uptrakit_web_api_types::pagination::ResolvedPagination {
    PaginationParams {
        page: params.page,
        per_page: params.per_page,
    }
    .resolve()
}

fn audit_log_to_response(m: audit_log::Model) -> AuditLogResponse {
    AuditLogResponse {
        id: m.id,
        actor_id: m.actor_id,
        actor_type: m.actor_type,
        auth_method: m.auth_method,
        http_method: m.http_method,
        http_path: m.http_path,
        route_pattern: m.route_pattern,
        http_status: m.http_status as u16,
        client_ip: m.client_ip,
        user_agent: m.user_agent,
        duration_ms: m.duration_ms as u64,
        occurred_at: m.occurred_at,
    }
}

fn system_audit_log_to_response(m: system_audit_log::Model) -> SystemAuditLogResponse {
    SystemAuditLogResponse {
        id: m.id,
        actor_id: m.actor_id,
        actor_type: m.actor_type,
        auth_method: m.auth_method,
        http_method: m.http_method,
        http_path: m.http_path,
        route_pattern: m.route_pattern,
        http_status: m.http_status as u16,
        client_ip: m.client_ip,
        user_agent: m.user_agent,
        duration_ms: m.duration_ms as u64,
        occurred_at: m.occurred_at,
    }
}

// ---------------------------------------------------------------------------
// Public query functions
// ---------------------------------------------------------------------------

/// List tenant-scoped audit log entries with optional filters and pagination.
#[tracing::instrument(skip_all)]
pub async fn list_tenant_audit_logs(
    tenant_db: &TenantDb,
    params: &AuditLogListParams,
) -> Result<PaginatedResponse<AuditLogResponse>> {
    let pagination = resolve_pagination(params);

    let mut q = tenant_db.find::<audit_log::Entity>();

    if let Some(ref actor_type) = params.actor_type {
        q = q.filter(audit_log::Column::ActorType.eq(actor_type));
    }
    if let Some(ref method) = params.method {
        q = q.filter(audit_log::Column::HttpMethod.eq(method));
    }
    if let Some(status) = params.status {
        q = q.filter(audit_log::Column::HttpStatus.eq(status as i32));
    }
    if let Some(ref from) = params.from {
        let from_dt = time::OffsetDateTime::parse(from, &Rfc3339).map_err(|_| {
            report!(AuditLogQueryError::InvalidFilter(format!(
                "invalid 'from' timestamp: {from}"
            )))
        })?;
        q = q.filter(audit_log::Column::OccurredAt.gte(from_dt));
    }
    if let Some(ref to) = params.to {
        let to_dt = time::OffsetDateTime::parse(to, &Rfc3339).map_err(|_| {
            report!(AuditLogQueryError::InvalidFilter(format!(
                "invalid 'to' timestamp: {to}"
            )))
        })?;
        q = q.filter(audit_log::Column::OccurredAt.lte(to_dt));
    }
    if let Some(actor_id) = params.actor_id {
        q = q.filter(audit_log::Column::ActorId.eq(actor_id));
    }

    let base_query = q.order_by_desc(audit_log::Column::OccurredAt);

    let total = base_query
        .clone()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let items: Vec<AuditLogResponse> = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?
        .into_iter()
        .map(audit_log_to_response)
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// List system-level audit log entries with optional filters and pagination.
#[tracing::instrument(skip_all)]
pub async fn list_system_audit_logs(
    db: &DatabaseConnection,
    params: &AuditLogListParams,
) -> Result<PaginatedResponse<SystemAuditLogResponse>> {
    let pagination = resolve_pagination(params);

    let mut q = system_audit_log::Entity::find();

    if let Some(ref actor_type) = params.actor_type {
        q = q.filter(system_audit_log::Column::ActorType.eq(actor_type));
    }
    if let Some(ref method) = params.method {
        q = q.filter(system_audit_log::Column::HttpMethod.eq(method));
    }
    if let Some(status) = params.status {
        q = q.filter(system_audit_log::Column::HttpStatus.eq(status as i32));
    }
    if let Some(ref from) = params.from {
        let from_dt = time::OffsetDateTime::parse(from, &Rfc3339).map_err(|_| {
            report!(AuditLogQueryError::InvalidFilter(format!(
                "invalid 'from' timestamp: {from}"
            )))
        })?;
        q = q.filter(system_audit_log::Column::OccurredAt.gte(from_dt));
    }
    if let Some(ref to) = params.to {
        let to_dt = time::OffsetDateTime::parse(to, &Rfc3339).map_err(|_| {
            report!(AuditLogQueryError::InvalidFilter(format!(
                "invalid 'to' timestamp: {to}"
            )))
        })?;
        q = q.filter(system_audit_log::Column::OccurredAt.lte(to_dt));
    }
    if let Some(actor_id) = params.actor_id {
        q = q.filter(system_audit_log::Column::ActorId.eq(actor_id));
    }

    let base_query = q.order_by_desc(system_audit_log::Column::OccurredAt);

    let total = base_query.clone().count(db).await.context_to()?;

    let items: Vec<SystemAuditLogResponse> = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(system_audit_log_to_response)
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}
