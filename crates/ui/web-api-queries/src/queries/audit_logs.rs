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
        actor_type: m.actor_type,
        actor_id: m.actor_id,
        actor_display: m.actor_display,
        action_type: m.action_type,
        target_type: m.target_type,
        target_id: m.target_id,
        target_display: m.target_display,
        outcome: m.outcome,
        details_json: m.details_json,
        request_id: m.request_id,
        occurred_at: m.occurred_at,
    }
}

fn system_audit_log_to_response(m: system_audit_log::Model) -> SystemAuditLogResponse {
    SystemAuditLogResponse {
        id: m.id,
        actor_type: m.actor_type,
        actor_id: m.actor_id,
        actor_display: m.actor_display,
        action_type: m.action_type,
        target_type: m.target_type,
        target_id: m.target_id,
        target_display: m.target_display,
        outcome: m.outcome,
        details_json: m.details_json,
        request_id: m.request_id,
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
    if let Some(ref action_type) = params.action_type {
        q = q.filter(audit_log::Column::ActionType.eq(action_type));
    }
    if let Some(ref outcome) = params.outcome {
        q = q.filter(audit_log::Column::Outcome.eq(outcome));
    }
    if let Some(ref target_type) = params.target_type {
        q = q.filter(audit_log::Column::TargetType.eq(target_type));
    }
    if let Some(ref target_id) = params.target_id {
        q = q.filter(audit_log::Column::TargetId.eq(target_id));
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
    if let Some(ref action_type) = params.action_type {
        q = q.filter(system_audit_log::Column::ActionType.eq(action_type));
    }
    if let Some(ref outcome) = params.outcome {
        q = q.filter(system_audit_log::Column::Outcome.eq(outcome));
    }
    if let Some(ref target_type) = params.target_type {
        q = q.filter(system_audit_log::Column::TargetType.eq(target_type));
    }
    if let Some(ref target_id) = params.target_id {
        q = q.filter(system_audit_log::Column::TargetId.eq(target_id));
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    #[test]
    fn system_audit_log_to_response_maps_semantic_fields() {
        let occurred_at = time::OffsetDateTime::now_utc();
        let model = system_audit_log::Model {
            id: Uuid::now_v7(),
            actor_id: None,
            actor_type: "system".to_string(),
            actor_display: Some("scheduler".to_string()),
            action_type: "system.scheduler.audit_log_cleanup".to_string(),
            target_type: Some("audit_log".to_string()),
            target_id: Some("cleanup".to_string()),
            target_display: Some("Audit Log Cleanup".to_string()),
            outcome: "success".to_string(),
            details_json: Some(json!({ "deleted_rows": 42 })),
            request_id: Some("req-123".to_string()),
            occurred_at,
        };

        let response = system_audit_log_to_response(model);
        assert_eq!(response.actor_type, "system");
        assert_eq!(response.action_type, "system.scheduler.audit_log_cleanup");
        assert_eq!(response.target_type.as_deref(), Some("audit_log"));
        assert_eq!(response.target_id.as_deref(), Some("cleanup"));
        assert_eq!(
            response.target_display.as_deref(),
            Some("Audit Log Cleanup")
        );
        assert_eq!(response.outcome, "success");
        assert_eq!(response.details_json, Some(json!({ "deleted_rows": 42 })));
        assert_eq!(response.request_id.as_deref(), Some("req-123"));
        assert_eq!(response.occurred_at, occurred_at);
    }
}
