use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait, Set, TransactionTrait,
    sea_query::{Expr, OnConflict},
};
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_internal_wire::Capability;
use uptrakit_shared_db::entity::prelude::{RevocationReason, ServiceCertificate, ServiceHost};
use uptrakit_shared_db::entity::{service, service_certificate, service_host};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::services::{ListServicesQuery, ServiceResponse};
use uuid::Uuid;

use crate::queries::embedded_runtime_states::load_fresh_yielded_to;
use crate::tenant_db::TenantDb;
use crate::token_utils;
use uptrakit_internal_wire::service_profile::{ServiceProfile, parse_capabilities};

/// Errors returned by service mutation queries.
#[derive(Debug, Error)]
pub enum ServiceQueryError {
    /// No active service found with the given ID.
    #[error("service not found")]
    NotFound,
    /// The service must be in `Pending` status for this operation.
    #[error("service is not in pending status")]
    NotPending,
    /// The service must be in `Approved` status for this operation.
    #[error("service is not in approved status")]
    NotApproved,
    /// The service does not support merge (requires SoftwareDiscovery capability).
    #[error("service does not support merge")]
    NotMergeable,
    /// The target service is still connected (checked before calling the query).
    #[error("target service is still connected")]
    TargetConnected,
    /// The source service ID was not found.
    #[error("source service not found")]
    SourceNotFound,
    /// Embedded services cannot be deactivated through the API.
    #[error("embedded services cannot be deactivated")]
    EmbeddedService,
    /// A database error occurred.
    #[error("database error: {0}")]
    Db(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<ServiceQueryError>>;
impl_report_conversion!(sea_orm::DbErr => ServiceQueryError::Db);

// --- Private helpers ---

fn model_to_response(m: service::Model, yielded_to: Option<Vec<Uuid>>) -> ServiceResponse {
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
        is_embedded: m.is_embedded,
        ip_address: m.ip_address,
        status: m.status,
        client_version: m.client_version,
        last_seen_at: m.last_seen_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
        ping_interval_seconds: m.ping_interval_seconds.map(|v| v as u32),
        cert_lifetime_hours: m.cert_lifetime_hours.map(|v| v as u32),
        yielded_to,
    }
}

async fn build_service_response(
    tenant_db: &TenantDb,
    model: service::Model,
) -> Result<ServiceResponse> {
    let yielded = load_fresh_yielded_to(tenant_db.db(), &[model.id])
        .await
        .context_to()?;
    let yielded_to = if model.is_embedded {
        yielded.get(&model.id).cloned()
    } else {
        None
    };
    Ok(model_to_response(model, yielded_to))
}

async fn build_service_responses(
    tenant_db: &TenantDb,
    models: Vec<service::Model>,
) -> Result<Vec<ServiceResponse>> {
    let service_ids: Vec<Uuid> = models
        .iter()
        .filter(|model| model.is_embedded)
        .map(|model| model.id)
        .collect();
    let yielded = load_fresh_yielded_to(tenant_db.db(), &service_ids)
        .await
        .context_to()?;

    Ok(models
        .into_iter()
        .map(|model| {
            let yielded_to = if model.is_embedded {
                yielded.get(&model.id).cloned()
            } else {
                None
            };
            model_to_response(model, yielded_to)
        })
        .collect())
}

// --- Public query functions ---

#[tracing::instrument(skip_all)]
pub async fn list_services(
    tenant_db: &TenantDb,
    query: &ListServicesQuery,
) -> Result<PaginatedResponse<ServiceResponse>> {
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

    let total = base_query
        .clone()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let services = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    let items = build_service_responses(tenant_db, services).await?;
    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if not found or deactivated.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn get_active_service(tenant_db: &TenantDb, id: Uuid) -> Result<Option<ServiceResponse>> {
    let svc = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?;
    match svc {
        Some(model) => Ok(Some(build_service_response(tenant_db, model).await?)),
        None => Ok(None),
    }
}

/// Update configurable service settings.
///
/// For both `ping_interval_seconds` and `cert_lifetime_hours`:
/// - `None` — keep the current value (field not touched)
/// - `Some(0)` — clear the override (set column to `NULL`)
/// - `Some(v)` — set column to `v`
///
/// Returns `None` if the service is not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn update_service_settings(
    tenant_db: &TenantDb,
    id: Uuid,
    ping_interval_seconds: Option<u32>,
    cert_lifetime_hours: Option<u32>,
) -> Result<Option<ServiceResponse>> {
    let Some(svc) = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?
    else {
        return Ok(None);
    };

    let mut active: service::ActiveModel = svc.into();
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

    let updated = active.update(tenant_db.db()).await.context_to()?;
    Ok(Some(build_service_response(tenant_db, updated).await?))
}

/// Approve a pending service.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn approve_service(tenant_db: &TenantDb, id: Uuid) -> Result<ServiceResponse> {
    let svc = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?;

    let svc = svc.ok_or_else(|| report!(ServiceQueryError::NotFound))?;

    if svc.status != service::ServiceStatus::Pending {
        bail!(ServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.status = Set(service::ServiceStatus::Approved);
    active.updated_at = Set(now);

    let updated = active.update(tenant_db.db()).await.context_to()?;
    build_service_response(tenant_db, updated).await
}

/// Reject a pending service.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn reject_service(tenant_db: &TenantDb, id: Uuid) -> Result<ServiceResponse> {
    let svc = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
        .context_to()?;

    let svc = svc.ok_or_else(|| report!(ServiceQueryError::NotFound))?;

    if svc.status != service::ServiceStatus::Pending {
        bail!(ServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.status = Set(service::ServiceStatus::Rejected);
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);

    let updated = active.update(tenant_db.db()).await.context_to()?;
    build_service_response(tenant_db, updated).await
}

/// Soft-delete a service, revoke its certificates, and bump the revocation counter.
///
/// All three mutations run inside a single database transaction. If any step
/// fails the entire operation is rolled back, preventing a partially-deactivated
/// state where certificates are not revoked and the CRL is not updated.
///
/// Returns `true` if the service was deactivated, `false` if not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn deactivate_service(
    tenant_db: &TenantDb,
    id: Uuid,
    default_tenant_id: Uuid,
) -> Result<bool> {
    let txn = tenant_db.db().begin().await.context_to()?;

    let Some(svc) = tenant_db
        .find_by_id::<service::Entity, _>(id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .context_to()?
    else {
        return Ok(false);
    };

    if svc.is_embedded {
        bail!(ServiceQueryError::EmbeddedService);
    }

    let now = OffsetDateTime::now_utc();
    let mut active: service::ActiveModel = svc.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(&txn).await.context_to()?;

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
        .context_to()?;

    crate::settings_version::bump_revocation_version(&txn, default_tenant_id)
        .await
        .map_err(|e| report!(ServiceQueryError::Db(e)))?;

    txn.commit().await.context_to()?;

    Ok(true)
}

/// Merge a pending (source) agent into an existing approved (target) agent.
///
/// The caller must verify that the target is not currently connected before
/// calling this function (pass `target_connected = true` to abort cleanly).
#[tracing::instrument(skip_all)]
pub async fn merge_service(
    tenant_db: &TenantDb,
    target_uuid: Uuid,
    source_uuid: Uuid,
    target_connected: bool,
    default_tenant_id: Uuid,
) -> Result<ServiceResponse> {
    if target_connected {
        bail!(ServiceQueryError::TargetConnected);
    }

    let txn = tenant_db.db().begin().await.context_to()?;

    // Find target service (must be approved, not deactivated, mergeable).
    let target = tenant_db
        .find_by_id::<service::Entity, _>(target_uuid)
        .lock_exclusive()
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .context_to()?
        .ok_or_else(|| report!(ServiceQueryError::NotFound))?;

    let target_caps = parse_capabilities(&target.capabilities);
    if !target_caps.contains(&Capability::SoftwareDiscovery) {
        bail!(ServiceQueryError::NotMergeable);
    }
    if target.status != service::ServiceStatus::Approved {
        bail!(ServiceQueryError::NotApproved);
    }

    // Find source service (must be pending, not deactivated, mergeable).
    let source = tenant_db
        .find_by_id::<service::Entity, _>(source_uuid)
        .lock_exclusive()
        .filter(service::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .context_to()?
        .ok_or_else(|| report!(ServiceQueryError::SourceNotFound))?;

    let source_caps = parse_capabilities(&source.capabilities);
    if !source_caps.contains(&Capability::SoftwareDiscovery) {
        bail!(ServiceQueryError::NotMergeable);
    }
    if source.status != service::ServiceStatus::Pending {
        bail!(ServiceQueryError::NotPending);
    }

    let now = OffsetDateTime::now_utc();

    let source_secret_hash = source.enrollment_secret_hash.clone();
    let source_hostname = source.hostname.clone();
    let source_friendly_name = source.friendly_name.clone();
    let source_ip_address = source.ip_address.clone();

    // Deactivate source — invalidate its hash to free the unique constraint.
    let invalidated_hash = token_utils::hash_token(&token_utils::generate_uuid().to_string());
    let mut source_active: service::ActiveModel = source.into();
    source_active.enrollment_secret_hash = Set(invalidated_hash);
    source_active.deactivated_at = Set(Some(now));
    source_active.updated_at = Set(now);
    source_active.update(&txn).await.context_to()?;

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
            bail!(ServiceQueryError::Db(e));
        }
    }

    if let Err(e) = crate::settings_version::bump_revocation_version(&txn, default_tenant_id).await
    {
        tracing::warn!(error = ?e, "failed to bump revocation version counter during merge");
    }

    // Update target: copy source identity fields.
    let mut target_active: service::ActiveModel = target.into();
    target_active.enrollment_secret_hash = Set(source_secret_hash);
    target_active.hostname = Set(source_hostname);
    target_active.friendly_name = Set(source_friendly_name);
    target_active.ip_address = Set(source_ip_address);
    target_active.updated_at = Set(now);

    let updated_target = target_active.update(&txn).await.context_to()?;

    // Copy source service's host links to target (tenant-scoped via join on service).
    // Run within the transaction so the join sees the same snapshot as the
    // other DML in this function.
    let source_links = tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::ServiceId.eq(source_uuid))
        .all(&txn)
        .await
        .context_to()?;

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
            bail!(ServiceQueryError::Db(e));
        }
    }

    txn.commit().await.context_to()?;

    build_service_response(tenant_db, updated_target).await
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Approve multiple pending services in one query.
///
/// Returns `(succeeded_ids, failed)` where `failed` contains `(id, reason)` pairs.
/// Services that are not found or not in `Pending` status are reported as failed.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_approve_services(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let services = tenant_db
        .find::<service::Entity>()
        .filter(service::Column::Id.is_in(ids.iter().copied()))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, service::Model> =
        services.into_iter().map(|s| (s.id, s)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    // Report missing IDs.
    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, svc) in &found {
        if svc.status != service::ServiceStatus::Pending {
            failed.push((*id, "service is not in pending status".to_string()));
            continue;
        }
        let mut active: service::ActiveModel = svc.clone().into();
        active.status = Set(service::ServiceStatus::Approved);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Reject multiple pending services in one query.
///
/// Returns `(succeeded_ids, failed)` where `failed` contains `(id, reason)` pairs.
/// Services that are not found or not in `Pending` status are reported as failed.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_reject_services(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let services = tenant_db
        .find::<service::Entity>()
        .filter(service::Column::Id.is_in(ids.iter().copied()))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, service::Model> =
        services.into_iter().map(|s| (s.id, s)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, svc) in &found {
        if svc.status != service::ServiceStatus::Pending {
            failed.push((*id, "service is not in pending status".to_string()));
            continue;
        }
        let mut active: service::ActiveModel = svc.clone().into();
        active.status = Set(service::ServiceStatus::Rejected);
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Deactivate multiple services (soft-delete with certificate revocation).
///
/// Returns `(succeeded_ids, failed)` where `failed` contains `(id, reason)` pairs.
/// Services that are not found are reported as failed.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_deactivate_services(
    tenant_db: &TenantDb,
    ids: &[Uuid],
    default_tenant_id: Uuid,
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let services = tenant_db
        .find::<service::Entity>()
        .filter(service::Column::Id.is_in(ids.iter().copied()))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, service::Model> =
        services.into_iter().map(|s| (s.id, s)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    // Deactivate each service in its own transaction so that individual
    // failures don't block the rest of the batch.
    for (id, svc) in &found {
        if svc.is_embedded {
            failed.push((*id, "embedded services cannot be deactivated".to_string()));
            continue;
        }

        let txn = tenant_db.db().begin().await.context_to()?;

        let mut active: service::ActiveModel = svc.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await.context_to()?;

        ServiceCertificate::update_many()
            .col_expr(
                service_certificate::Column::RevokedAt,
                Expr::value(Some(now)),
            )
            .col_expr(
                service_certificate::Column::RevocationReason,
                Expr::value(Some(RevocationReason::ServiceDeactivated)),
            )
            .filter(service_certificate::Column::ServiceId.eq(*id))
            .filter(service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        crate::settings_version::bump_revocation_version(&txn, default_tenant_id)
            .await
            .map_err(|e| report!(ServiceQueryError::Db(e)))?;

        txn.commit().await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}
