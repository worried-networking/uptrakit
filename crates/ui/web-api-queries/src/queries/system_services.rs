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

use crate::queries::embedded_runtime_states::load_fresh_yielded_to;

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
    /// Embedded system services cannot be deactivated through the API.
    #[error("embedded system services cannot be deactivated")]
    EmbeddedService,
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

fn model_to_response(
    m: system_service::Model,
    yielded_to: Option<Vec<Uuid>>,
) -> SystemServiceResponse {
    let caps = parse_capabilities(&m.capabilities);
    let cap_strings: Vec<String> = caps.iter().map(|c| c.as_str().to_string()).collect();
    SystemServiceResponse {
        id: m.id,
        capabilities: cap_strings,
        hostname: m.hostname,
        friendly_name: m.friendly_name,
        is_embedded: m.is_embedded,
        ip_address: m.ip_address,
        status: db_status_to_service_status(m.status),
        client_version: m.client_version,
        last_seen_at: m.last_seen_at,
        created_at: m.created_at,
        updated_at: m.updated_at,
        ping_interval_seconds: m.ping_interval_seconds.map(|v| v as u32),
        cert_lifetime_hours: m.cert_lifetime_hours.map(|v| v as u32),
        yielded_to,
    }
}

async fn build_system_service_response(
    db: &DatabaseConnection,
    model: system_service::Model,
) -> Result<SystemServiceResponse> {
    let yielded = load_fresh_yielded_to(db, &[model.id]).await.context_to()?;
    let yielded_to = if model.is_embedded {
        yielded.get(&model.id).cloned()
    } else {
        None
    };
    Ok(model_to_response(model, yielded_to))
}

async fn build_system_service_responses(
    db: &DatabaseConnection,
    models: Vec<system_service::Model>,
) -> Result<Vec<SystemServiceResponse>> {
    let service_ids: Vec<Uuid> = models
        .iter()
        .filter(|model| model.is_embedded)
        .map(|model| model.id)
        .collect();
    let yielded = load_fresh_yielded_to(db, &service_ids).await.context_to()?;

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

    let items = build_system_service_responses(db, services).await?;
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
    match svc {
        Some(model) => Ok(Some(build_system_service_response(db, model).await?)),
        None => Ok(None),
    }
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
    Ok(Some(build_system_service_response(db, updated).await?))
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
    build_system_service_response(db, updated).await
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
    build_system_service_response(db, updated).await
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

    if svc.is_embedded {
        bail!(SystemServiceQueryError::EmbeddedService);
    }

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

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Approve multiple pending system services.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_approve_system_services(
    db: &DatabaseConnection,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let services = system_service::Entity::find()
        .filter(system_service::Column::Id.is_in(ids.iter().copied()))
        .filter(system_service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, system_service::Model> =
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
        if svc.status != SystemServiceStatus::Pending {
            failed.push((*id, "system service is not in pending status".to_string()));
            continue;
        }
        let mut active: system_service::ActiveModel = svc.clone().into();
        active.status = Set(SystemServiceStatus::Approved);
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Reject multiple pending system services.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_reject_system_services(
    db: &DatabaseConnection,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let services = system_service::Entity::find()
        .filter(system_service::Column::Id.is_in(ids.iter().copied()))
        .filter(system_service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, system_service::Model> =
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
        if svc.status != SystemServiceStatus::Pending {
            failed.push((*id, "system service is not in pending status".to_string()));
            continue;
        }
        let mut active: system_service::ActiveModel = svc.clone().into();
        active.status = Set(SystemServiceStatus::Rejected);
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Deactivate multiple system services (soft-delete with certificate revocation).
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_deactivate_system_services(
    db: &DatabaseConnection,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let services = system_service::Entity::find()
        .filter(system_service::Column::Id.is_in(ids.iter().copied()))
        .filter(system_service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, system_service::Model> =
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
        if svc.is_embedded {
            failed.push((*id, "embedded services cannot be deactivated".to_string()));
            continue;
        }

        let txn = db.begin().await.context_to()?;

        let mut active: system_service::ActiveModel = svc.clone().into();
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
            .filter(system_service_certificate::Column::SystemServiceId.eq(*id))
            .filter(system_service_certificate::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

#[cfg(test)]
mod tests {
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, Set,
    };
    use time::OffsetDateTime;
    use uuid::Uuid;

    use uptrakit_shared_db::entity::system_service::SystemServiceStatus;
    use uptrakit_shared_db::entity::system_service_certificate::SystemRevocationReason;
    use uptrakit_shared_db::entity::{ca_certificate, system_service, system_service_certificate};

    use super::*;

    async fn setup_db() -> DatabaseConnection {
        uptrakit_crypto::enable_plaintext_mode();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    /// Insert a minimal CA certificate to satisfy the FK on `system_service_certificates`.
    async fn insert_ca(db: &DatabaseConnection, fingerprint: &str) {
        let now = OffsetDateTime::now_utc();
        ca_certificate::ActiveModel {
            fingerprint: Set(fingerprint.to_string()),
            cert_pem: Set("fake-cert-pem".to_string()),
            key_pem: Set(uptrakit_crypto::EncryptedString::plaintext_for_test(
                "fake-key-pem".to_string(),
            )),
            not_before: Set(now),
            not_after: Set(now + time::Duration::days(365)),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_service(
        db: &DatabaseConnection,
        id: Uuid,
        status: SystemServiceStatus,
    ) -> system_service::Model {
        let now = OffsetDateTime::now_utc();
        system_service::ActiveModel {
            id: Set(id),
            capabilities: Set(String::new()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("Test Service".to_string()),
            ip_address: Set(None),
            status: Set(status),
            enrollment_secret_hash: Set(format!("hash-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
    }

    async fn insert_certificate(
        db: &DatabaseConnection,
        service_id: Uuid,
        ca_fp: &str,
        serial: &str,
    ) -> system_service_certificate::Model {
        let now = OffsetDateTime::now_utc();
        system_service_certificate::ActiveModel {
            ca_fingerprint: Set(ca_fp.to_string()),
            serial_number: Set(serial.to_string()),
            system_service_id: Set(service_id),
            not_before: Set(now),
            not_after: Set(now + time::Duration::days(365)),
            revoked_at: Set(None),
            revocation_reason: Set(None),
            created_at: Set(now),
            last_seen_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn deactivate_sets_deactivated_at_and_revokes_certs() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        insert_ca(&db, "fp1").await;
        insert_service(&db, id, SystemServiceStatus::Approved).await;
        insert_certificate(&db, id, "fp1", "serial1").await;
        insert_certificate(&db, id, "fp1", "serial2").await;

        let result = deactivate_system_service(&db, id).await.unwrap();
        assert!(result, "deactivate must return true for an active service");

        // Service must have deactivated_at set.
        let svc = system_service::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(
            svc.deactivated_at.is_some(),
            "deactivated_at must be set after deactivation"
        );

        // All certificates must be revoked.
        let unrevoked = system_service_certificate::Entity::find()
            .filter(system_service_certificate::Column::SystemServiceId.eq(id))
            .filter(system_service_certificate::Column::RevokedAt.is_null())
            .all(&db)
            .await
            .unwrap();
        assert!(
            unrevoked.is_empty(),
            "all certificates must be revoked after service deactivation"
        );

        // Revocation reason must be ServiceDeactivated.
        let certs = system_service_certificate::Entity::find()
            .filter(system_service_certificate::Column::SystemServiceId.eq(id))
            .all(&db)
            .await
            .unwrap();
        for cert in certs {
            assert_eq!(
                cert.revocation_reason,
                Some(SystemRevocationReason::ServiceDeactivated)
            );
        }
    }

    #[tokio::test]
    async fn deactivate_already_deactivated_returns_false() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        insert_service(&db, id, SystemServiceStatus::Approved).await;

        let first = deactivate_system_service(&db, id).await.unwrap();
        assert!(first);

        let second = deactivate_system_service(&db, id).await.unwrap();
        assert!(
            !second,
            "deactivating an already-deactivated service must return false"
        );
    }

    #[tokio::test]
    async fn approve_non_pending_returns_not_pending() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        insert_service(&db, id, SystemServiceStatus::Approved).await;

        let err = approve_system_service(&db, id).await.unwrap_err();
        assert!(
            matches!(err.current_context(), SystemServiceQueryError::NotPending),
            "approving an already-approved service must return NotPending"
        );
    }

    #[tokio::test]
    async fn reject_pending_sets_rejected_and_deactivated_at() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        insert_service(&db, id, SystemServiceStatus::Pending).await;

        let result = reject_system_service(&db, id).await.unwrap();
        assert_eq!(
            result.status,
            uptrakit_shared_types::ServiceStatus::Rejected
        );

        let svc = system_service::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert!(
            svc.deactivated_at.is_some(),
            "rejected service must have deactivated_at set"
        );
    }

    #[tokio::test]
    async fn reject_already_approved_returns_not_pending() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        insert_service(&db, id, SystemServiceStatus::Approved).await;

        let err = reject_system_service(&db, id).await.unwrap_err();
        assert!(
            matches!(err.current_context(), SystemServiceQueryError::NotPending),
            "rejecting an already-approved service must return NotPending"
        );
    }

    #[tokio::test]
    async fn update_settings_zero_clears_to_null() {
        let db = setup_db().await;
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        // Insert with non-null values.
        system_service::ActiveModel {
            id: Set(id),
            capabilities: Set(String::new()),
            hostname: Set("host".to_string()),
            friendly_name: Set("svc".to_string()),
            ip_address: Set(None),
            status: Set(SystemServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(Some(60)),
            cert_lifetime_hours: Set(Some(24)),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let result = update_system_service_settings(&db, id, Some(0), Some(0))
            .await
            .unwrap()
            .unwrap();
        assert!(
            result.ping_interval_seconds.is_none(),
            "ping_interval_seconds must be cleared to None when Some(0) is passed"
        );
        assert!(
            result.cert_lifetime_hours.is_none(),
            "cert_lifetime_hours must be cleared to None when Some(0) is passed"
        );
    }
}
