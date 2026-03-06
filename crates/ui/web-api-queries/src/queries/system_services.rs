use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, Set, TransactionTrait, sea_query::Expr,
};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_internal_wire::service_profile::parse_capabilities;
use uptrakit_shared_db::entity::system_service::{self, SystemServiceStatus};
use uptrakit_shared_db::entity::system_service_certificate::{self, SystemRevocationReason};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_shared_types::ServiceStatus;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::system_services::{ListSystemServicesQuery, SystemServiceResponse};
use uuid::Uuid;

/// Errors returned by system service mutation queries.
#[derive(Debug, Error)]
pub enum SystemServiceQueryError {
    /// No active system service found with the given ID.
    #[error("system service not found")]
    NotFound,
    /// The system service must be in `Pending` status for this operation.
    #[error("system service is not in pending status")]
    NotPending,
    /// The system service must be in `Approved` status for this operation.
    #[error("system service is not in approved status")]
    NotApproved,
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<SystemServiceQueryError>>;
impl_report_conversion!(sea_orm::DbErr => SystemServiceQueryError::Db);

// --- Private helpers ---

fn db_status_to_service_status(s: SystemServiceStatus) -> ServiceStatus {
    match s {
        SystemServiceStatus::Pending => ServiceStatus::Pending,
        SystemServiceStatus::Approved => ServiceStatus::Approved,
        SystemServiceStatus::Rejected => ServiceStatus::Rejected,
        SystemServiceStatus::Deactivated => ServiceStatus::Deactivated,
    }
}

fn service_status_to_db_status(s: ServiceStatus) -> SystemServiceStatus {
    match s {
        ServiceStatus::Pending => SystemServiceStatus::Pending,
        ServiceStatus::Approved => SystemServiceStatus::Approved,
        ServiceStatus::Rejected => SystemServiceStatus::Rejected,
        ServiceStatus::Deactivated => SystemServiceStatus::Deactivated,
        _ => {
            tracing::warn!(
                "unrecognised ServiceStatus variant in system_service conversion, defaulting to Pending"
            );
            SystemServiceStatus::Pending
        }
    }
}

fn model_to_response(m: system_service::Model) -> SystemServiceResponse {
    let caps = parse_capabilities(&m.capabilities);
    let cap_strings: Vec<String> = caps.iter().map(|c| c.as_str().to_string()).collect();
    SystemServiceResponse {
        id: m.id,
        capabilities: cap_strings,
        hostname: m.hostname,
        friendly_name: m.friendly_name,
        ip_address: m.ip_address,
        status: db_status_to_service_status(m.status),
        client_version: m.client_version,
        last_seen_at: m.last_seen_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
        ping_interval_seconds: m.ping_interval_seconds.map(|v| v as u32),
        cert_lifetime_hours: m.cert_lifetime_hours.map(|v| v as u32),
    }
}

// --- Public query functions ---

/// List system services with optional filters and pagination.
#[tracing::instrument(skip_all)]
pub async fn list_system_services(
    db: &DatabaseConnection,
    query: &ListSystemServicesQuery,
) -> Result<PaginatedResponse<SystemServiceResponse>> {
    let pagination = query.pagination().resolve();

    let mut q =
        system_service::Entity::find().filter(system_service::Column::DeactivatedAt.is_null());

    if let Some(ref cap_filter) = query.capability {
        q = q.filter(system_service::Column::Capabilities.contains(cap_filter));
    }
    if let Some(ref status_filter) = query.status {
        q = q
            .filter(system_service::Column::Status.eq(service_status_to_db_status(*status_filter)));
    }

    let base_query = q.order_by_desc(system_service::Column::CreatedAt);

    let total = base_query.clone().count(db).await.context_to()?;

    let services = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(db)
        .await
        .context_to()?;

    let items: Vec<SystemServiceResponse> = services.into_iter().map(model_to_response).collect();
    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if not found or deactivated.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn get_active_system_service(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<Option<SystemServiceResponse>> {
    let svc = system_service::Entity::find_by_id(id)
        .filter(system_service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?;
    Ok(svc.map(model_to_response))
}

/// Update configurable system service settings.
///
/// For both `ping_interval_seconds` and `cert_lifetime_hours`:
/// - `None` — keep the current value (field not touched)
/// - `Some(0)` — clear the override (set column to `NULL`)
/// - `Some(v)` — set column to `v`
///
/// Returns `None` if the service is not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn update_system_service_settings(
    db: &DatabaseConnection,
    id: Uuid,
    ping_interval_seconds: Option<u32>,
    cert_lifetime_hours: Option<u32>,
) -> Result<Option<SystemServiceResponse>> {
    let Some(svc) = system_service::Entity::find_by_id(id)
        .filter(system_service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
    else {
        return Ok(None);
    };

    let mut active: system_service::ActiveModel = svc.into();
    match ping_interval_seconds {
        Some(0) => active.ping_interval_seconds = Set(None),
        Some(v) => active.ping_interval_seconds = Set(Some(v as i32)),
        None => {}
    }
    match cert_lifetime_hours {
        Some(0) => active.cert_lifetime_hours = Set(None),
        Some(v) => active.cert_lifetime_hours = Set(Some(v as i32)),
        None => {}
    }
    active.updated_at = Set(OffsetDateTime::now_utc());

    let updated = active.update(db).await.context_to()?;
    Ok(Some(model_to_response(updated)))
}

/// Approve a pending system service.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn approve_system_service(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<SystemServiceResponse> {
    let svc = system_service::Entity::find_by_id(id)
        .filter(system_service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?;

    let svc = svc.ok_or_else(|| report!(SystemServiceQueryError::NotFound))?;

    if svc.status != SystemServiceStatus::Pending {
        bail!(SystemServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: system_service::ActiveModel = svc.into();
    active.status = Set(SystemServiceStatus::Approved);
    active.updated_at = Set(now);

    let updated = active.update(db).await.context_to()?;
    Ok(model_to_response(updated))
}

/// Reject a pending system service.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn reject_system_service(
    db: &DatabaseConnection,
    id: Uuid,
) -> Result<SystemServiceResponse> {
    let svc = system_service::Entity::find_by_id(id)
        .filter(system_service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?;

    let svc = svc.ok_or_else(|| report!(SystemServiceQueryError::NotFound))?;

    if svc.status != SystemServiceStatus::Pending {
        bail!(SystemServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: system_service::ActiveModel = svc.into();
    active.status = Set(SystemServiceStatus::Rejected);
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    let updated = active.update(db).await.context_to()?;
    Ok(model_to_response(updated))
}

/// Soft-delete a system service and revoke all its non-revoked certificates.
///
/// Both mutations run inside a single database transaction to prevent a
/// partially-deactivated state where the service is deactivated but its
/// certificates are not revoked.
///
/// Returns `true` if the system service was deactivated, `false` if not found.
///
/// Note: unlike tenant services, this function does **not** call
/// `bump_revocation_version` because that function is tenant-scoped. The
/// calling route handler is responsible for triggering `revocation_notify`
/// and requesting a CRL renewal.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn deactivate_system_service(db: &DatabaseConnection, id: Uuid) -> Result<bool> {
    let txn = db.begin().await.context_to()?;

    let Some(svc) = system_service::Entity::find_by_id(id)
        .filter(system_service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .context_to()?
    else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut active: system_service::ActiveModel = svc.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(&txn).await.context_to()?;

    system_service_certificate::Entity::update_many()
        .col_expr(
            system_service_certificate::Column::RevokedAt,
            Expr::value(Some(now)),
        )
        .col_expr(
            system_service_certificate::Column::RevocationReason,
            Expr::value(Some(SystemRevocationReason::ServiceDeactivated)),
        )
        .filter(system_service_certificate::Column::SystemServiceId.eq(id))
        .filter(system_service_certificate::Column::RevokedAt.is_null())
        .exec(&txn)
        .await
        .context_to()?;

    txn.commit().await.context_to()?;

    Ok(true)
}
