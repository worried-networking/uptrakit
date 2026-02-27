use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
    sea_query::{Expr, OnConflict},
};
use time::OffsetDateTime;
use uptrakit_internal_wire::Capability;
use uptrakit_shared_db::entity::prelude::{RevocationReason, ServiceCertificate, ServiceHost};
use uptrakit_shared_db::entity::{service, service_certificate, service_host};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::services::{ListServicesQuery, ServiceResponse};
use uuid::Uuid;

use crate::auth::token;
use crate::service_profile::{ServiceProfile, parse_capabilities};
use crate::tenant_db::TenantDb;

/// Errors returned by service mutation queries.
#[derive(Debug)]
pub enum ServiceQueryError {
    /// No active service found with the given ID.
    NotFound,
    /// The service must be in `Pending` status for this operation.
    NotPending,
    /// The service must be in `Approved` status for this operation.
    NotApproved,
    /// The service does not support merge (requires SoftwareDiscovery capability).
    NotMergeable,
    /// The target service is still connected (checked before calling the query).
    TargetConnected,
    /// The source service ID was not found.
    SourceNotFound,
    /// A database error occurred.
    Db(sea_orm::DbErr),
}

// --- Private helpers ---

fn model_to_response(m: service::Model) -> ServiceResponse {
    let caps = parse_capabilities(&m.capabilities);
    let profile = ServiceProfile::from_capabilities(&caps);
    let has_ssh = caps.contains(&Capability::SshRemote);
    let cap_strings: Vec<String> = caps.iter().map(|c| c.as_str().to_string()).collect();
    ServiceResponse {
        id: m.id,
        capabilities: cap_strings,
        service_label: profile.service_label(has_ssh).to_string(),
        hostname: m.hostname,
        friendly_name: m.friendly_name,
        ip_address: m.ip_address,
        status: m.status,
        client_version: m.client_version,
        last_seen_at: m.last_seen_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
        ping_interval_seconds: m.ping_interval_seconds.map(|v| v as u32),
    }
}

// --- Public query functions ---

pub async fn list_services(
    tenant_db: &TenantDb,
    query: &ListServicesQuery,
) -> Result<PaginatedResponse<ServiceResponse>, sea_orm::DbErr> {
    let pagination = query.pagination().resolve();

    let mut q = tenant_db
        .find::<service::Entity>()
        .filter(service::Column::DeactivatedAt.is_null());

    if let Some(ref cap_filter) = query.capability {
        // JSON text column: use LIKE to match capability string within the JSON array
        q = q.filter(service::Column::Capabilities.contains(cap_filter));
    }
    if let Some(ref status_filter) = query.status {
        q = q.filter(service::Column::Status.eq(*status_filter));
    }

    let base_query = q.order_by_desc(service::Column::CreatedAt);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let services = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    let items: Vec<ServiceResponse> = services.into_iter().map(model_to_response).collect();
    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if not found or deactivated.
pub async fn get_active_service(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<ServiceResponse>, sea_orm::DbErr> {
    let svc = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?;
    Ok(svc.map(model_to_response))
}

/// Update `ping_interval_seconds`. `None` = clear the override.
/// Returns `None` if not found.
pub async fn update_service_settings(
    tenant_db: &TenantDb,
    id: Uuid,
    ping_interval_seconds: Option<u32>,
) -> Result<Option<ServiceResponse>, sea_orm::DbErr> {
    let Some(svc) = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };

    let mut active: service::ActiveModel = svc.into();
    match ping_interval_seconds {
        Some(0) => active.ping_interval_seconds = Set(None),
        Some(v) => active.ping_interval_seconds = Set(Some(v as i32)),
        None => {}
    }
    active.updated_at = Set(OffsetDateTime::now_utc());

    let updated = active.update(tenant_db.db()).await?;
    Ok(Some(model_to_response(updated)))
}

/// Approve a pending service.
pub async fn approve_service(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<ServiceResponse, ServiceQueryError> {
    let svc = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .map_err(ServiceQueryError::Db)?;

    let svc = svc.ok_or(ServiceQueryError::NotFound)?;

    if svc.status != service::ServiceStatus::Pending {
        return Err(ServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.status = Set(service::ServiceStatus::Approved);
    active.updated_at = Set(now);

    let updated = active
        .update(tenant_db.db())
        .await
        .map_err(ServiceQueryError::Db)?;
    Ok(model_to_response(updated))
}

/// Reject a pending service.
pub async fn reject_service(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<ServiceResponse, ServiceQueryError> {
    let svc = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .map_err(ServiceQueryError::Db)?;

    let svc = svc.ok_or(ServiceQueryError::NotFound)?;

    if svc.status != service::ServiceStatus::Pending {
        return Err(ServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.status = Set(service::ServiceStatus::Rejected);
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    let updated = active
        .update(tenant_db.db())
        .await
        .map_err(ServiceQueryError::Db)?;
    Ok(model_to_response(updated))
}

/// Soft-delete a service, revoke its certificates, and bump the revocation counter.
///
/// All three mutations run inside a single database transaction. If any step
/// fails the entire operation is rolled back, preventing a partially-deactivated
/// state where certificates are not revoked and the CRL is not updated.
///
/// Returns `true` if the service was deactivated, `false` if not found.
pub async fn deactivate_service(
    tenant_db: &TenantDb,
    id: Uuid,
    default_tenant_id: Uuid,
) -> Result<bool, ServiceQueryError> {
    let txn = tenant_db
        .db()
        .begin()
        .await
        .map_err(ServiceQueryError::Db)?;

    let Some(svc) = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .map_err(ServiceQueryError::Db)?
    else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(&txn).await.map_err(ServiceQueryError::Db)?;

    ServiceCertificate::update_many()
        .col_expr(
            service_certificate::Column::RevokedAt,
            Expr::value(Some(now)),
        )
        .col_expr(
            service_certificate::Column::RevocationReason,
            Expr::value(Some(RevocationReason::ServiceDeactivated)),
        )
        .filter(service_certificate::Column::ServiceId.eq(id))
        .filter(service_certificate::Column::RevokedAt.is_null())
        .exec(&txn)
        .await
        .map_err(ServiceQueryError::Db)?;

    crate::settings_store::bump_revocation_version(&txn, default_tenant_id)
        .await
        .map_err(|e| ServiceQueryError::Db(sea_orm::DbErr::Custom(e.to_string())))?;

    txn.commit().await.map_err(ServiceQueryError::Db)?;

    Ok(true)
}

/// Merge a pending (source) agent into an existing approved (target) agent.
///
/// The caller must verify that the target is not currently connected before
/// calling this function (pass `target_connected = true` to abort cleanly).
pub async fn merge_service(
    tenant_db: &TenantDb,
    target_uuid: Uuid,
    source_uuid: Uuid,
    target_connected: bool,
    default_tenant_id: Uuid,
) -> Result<ServiceResponse, ServiceQueryError> {
    if target_connected {
        return Err(ServiceQueryError::TargetConnected);
    }

    let txn = tenant_db
        .db()
        .begin()
        .await
        .map_err(ServiceQueryError::Db)?;

    // Find target service (must be approved, not deactivated, mergeable).
    let target = tenant_db
        .find_by_id::<service::Entity, _>(target_uuid)
        .lock_exclusive()
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .map_err(ServiceQueryError::Db)?
        .ok_or(ServiceQueryError::NotFound)?;

    let target_caps = parse_capabilities(&target.capabilities);
    if !target_caps.contains(&Capability::SoftwareDiscovery) {
        return Err(ServiceQueryError::NotMergeable);
    }
    if target.status != service::ServiceStatus::Approved {
        return Err(ServiceQueryError::NotApproved);
    }

    // Find source service (must be pending, not deactivated, mergeable).
    let source = tenant_db
        .find_by_id::<service::Entity, _>(source_uuid)
        .lock_exclusive()
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .map_err(ServiceQueryError::Db)?
        .ok_or(ServiceQueryError::SourceNotFound)?;

    let source_caps = parse_capabilities(&source.capabilities);
    if !source_caps.contains(&Capability::SoftwareDiscovery) {
        return Err(ServiceQueryError::NotMergeable);
    }
    if source.status != service::ServiceStatus::Pending {
        return Err(ServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();

    let source_secret_hash = source.enrollment_secret_hash.clone();
    let source_hostname = source.hostname.clone();
    let source_friendly_name = source.friendly_name.clone();
    let source_ip_address = source.ip_address.clone();

    // Deactivate source — invalidate its hash to free the unique constraint.
    let invalidated_hash = token::hash_token(&token::generate_uuid().to_string());
    let mut source_active: service::ActiveModel = source.into();
    source_active.enrollment_secret_hash = Set(invalidated_hash);
    source_active.deactivated_at = Set(Some(now));
    source_active.updated_at = Set(now);
    source_active
        .update(&txn)
        .await
        .map_err(ServiceQueryError::Db)?;

    // Revoke all non-revoked certificates for both services.
    for (svc_uuid, label) in [(source_uuid, "source"), (target_uuid, "target")] {
        if let Err(e) = ServiceCertificate::update_many()
            .col_expr(
                service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                service_certificate::Column::RevocationReason,
                Expr::value(Some(RevocationReason::ServiceMerged)),
            )
            .filter(service_certificate::Column::ServiceId.eq(svc_uuid))
            .filter(service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
        {
            tracing::error!("Failed to revoke {label} service certificates: {}", e);
            return Err(ServiceQueryError::Db(e));
        }
    }

    if let Err(e) = crate::settings_store::bump_revocation_version(&txn, default_tenant_id).await {
        tracing::warn!(error = ?e, "failed to bump revocation version counter during merge");
    }

    // Update target: copy source identity fields.
    let mut target_active: service::ActiveModel = target.into();
    target_active.enrollment_secret_hash = Set(source_secret_hash);
    target_active.hostname = Set(source_hostname);
    target_active.friendly_name = Set(source_friendly_name);
    target_active.ip_address = Set(source_ip_address);
    target_active.updated_at = Set(now);

    let updated_target = target_active
        .update(&txn)
        .await
        .map_err(ServiceQueryError::Db)?;

    // Copy source service's host links to target (INSERT ON CONFLICT DO NOTHING).
    let source_links = ServiceHost::find()
        .filter(service_host::Column::ServiceId.eq(source_uuid))
        .all(&txn)
        .await
        .map_err(ServiceQueryError::Db)?;

    for link in source_links {
        let new_link = service_host::ActiveModel {
            service_id: Set(target_uuid),
            host_id: Set(link.host_id),
            linked_at: Set(now),
        };
        if let Err(e) = ServiceHost::insert(new_link)
            .on_conflict(
                OnConflict::columns([
                    service_host::Column::ServiceId,
                    service_host::Column::HostId,
                ])
                .do_nothing()
                .to_owned(),
            )
            .exec(&txn)
            .await
        {
            tracing::error!("Failed to copy host link during merge: {}", e);
            return Err(ServiceQueryError::Db(e));
        }
    }

    txn.commit().await.map_err(ServiceQueryError::Db)?;

    Ok(model_to_response(updated_target))
}
