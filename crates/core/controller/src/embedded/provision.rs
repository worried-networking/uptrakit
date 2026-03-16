//! Auto-provisioning for embedded service records.
//!
//! On startup, each embedded service needs a row in the appropriate table
//! so that the `ServiceConnectionRegistry` and `MessageProcessor` can
//! reference a valid service ID.
//!
//! - **System services** (scheduler) → `system_services` table
//! - **Tenant services** (embedded agent) → `services` table

use std::collections::BTreeSet;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use uptrakit_internal_wire::Capability;
use uptrakit_shared_db::entity::service;
use uptrakit_shared_db::entity::system_service::{self, SystemServiceStatus};
use uptrakit_shared_types::ServiceStatus;
use uuid::Uuid;

/// Find or create a system service record for an embedded service.
///
/// Lookup key: `service_app_name`. If a matching record exists, its ID is
/// returned. Otherwise, a new record is created with `Approved` status.
pub(crate) async fn provision_embedded_system_service(
    db: &DatabaseConnection,
    app_name: &str,
    friendly_name: &str,
    capabilities: &BTreeSet<Capability>,
    hostname: &str,
) -> rootcause::Result<Uuid> {
    // Look for an existing system service with this app_name.
    if let Some(existing) = system_service::Entity::find()
        .filter(system_service::Column::ServiceAppName.eq(app_name))
        .one(db)
        .await
        .context("query existing embedded system service")?
    {
        tracing::debug!(
            service_id = %existing.id,
            app_name,
            "reusing existing embedded system service record"
        );
        return Ok(existing.id);
    }

    // Create a new system service record.
    let now = time::OffsetDateTime::now_utc();
    let service_id = Uuid::now_v7();
    let caps_str = capabilities
        .iter()
        .map(|c| c.as_str().to_string())
        .collect::<Vec<_>>()
        .join(",");

    // Embedded services do not enroll via a secret — use a synthetic unique
    // hash that cannot collide with real Argon2id hashes.
    let synthetic_hash = format!("embedded:{service_id}");

    system_service::ActiveModel {
        id: ActiveValue::Set(service_id),
        capabilities: ActiveValue::Set(caps_str),
        hostname: ActiveValue::Set(hostname.to_string()),
        friendly_name: ActiveValue::Set(friendly_name.to_string()),
        ip_address: ActiveValue::Set(None),
        status: ActiveValue::Set(SystemServiceStatus::Approved),
        enrollment_secret_hash: ActiveValue::Set(synthetic_hash),
        client_version: ActiveValue::Set(None),
        last_seen_at: ActiveValue::Set(Some(now)),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
        deactivated_at: ActiveValue::Set(None),
        ping_interval_seconds: ActiveValue::Set(None),
        cert_lifetime_hours: ActiveValue::Set(None),
        system_enrollment_token_id: ActiveValue::Set(None),
        service_app_name: ActiveValue::Set(Some(app_name.to_string())),
    }
    .insert(db)
    .await
    .context("insert embedded system service record")?;

    tracing::info!(
        %service_id,
        app_name,
        friendly_name,
        "provisioned embedded system service"
    );

    Ok(service_id)
}

/// Find or create a tenant service record for an embedded service.
///
/// Lookup key: `service_app_name` within `tenant_id`. If a matching record
/// exists, its ID is returned. Otherwise, a new record is created with
/// `Approved` status.
pub(crate) async fn provision_embedded_tenant_service(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    app_name: &str,
    friendly_name: &str,
    capabilities: &BTreeSet<Capability>,
    hostname: &str,
) -> rootcause::Result<Uuid> {
    // Look for an existing tenant service with this app_name.
    if let Some(existing) = service::Entity::find()
        .filter(service::Column::TenantId.eq(tenant_id))
        .filter(service::Column::ServiceAppName.eq(app_name))
        .one(db)
        .await
        .context("query existing embedded tenant service")?
    {
        tracing::debug!(
            service_id = %existing.id,
            app_name,
            %tenant_id,
            "reusing existing embedded tenant service record"
        );
        return Ok(existing.id);
    }

    // Create a new tenant service record.
    let now = time::OffsetDateTime::now_utc();
    let service_id = Uuid::now_v7();
    let caps_str = capabilities
        .iter()
        .map(|c| c.as_str().to_string())
        .collect::<Vec<_>>()
        .join(",");

    let synthetic_hash = format!("embedded:{service_id}");

    service::ActiveModel {
        id: ActiveValue::Set(service_id),
        tenant_id: ActiveValue::Set(tenant_id),
        capabilities: ActiveValue::Set(caps_str),
        hostname: ActiveValue::Set(hostname.to_string()),
        friendly_name: ActiveValue::Set(friendly_name.to_string()),
        ip_address: ActiveValue::Set(None),
        status: ActiveValue::Set(ServiceStatus::Approved),
        enrollment_secret_hash: ActiveValue::Set(synthetic_hash),
        client_version: ActiveValue::Set(None),
        last_seen_at: ActiveValue::Set(Some(now)),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
        deactivated_at: ActiveValue::Set(None),
        ping_interval_seconds: ActiveValue::Set(None),
        cert_lifetime_hours: ActiveValue::Set(None),
        enrollment_token_id: ActiveValue::Set(None),
        service_app_name: ActiveValue::Set(Some(app_name.to_string())),
    }
    .insert(db)
    .await
    .context("insert embedded tenant service record")?;

    tracing::info!(
        %service_id,
        %tenant_id,
        app_name,
        friendly_name,
        "provisioned embedded tenant service"
    );

    Ok(service_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DatabaseConnection {
        use sea_orm::{ConnectOptions, Database};

        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("run migrations");
        db
    }

    #[tokio::test]
    async fn provision_creates_new_service() {
        let db = test_db().await;
        let caps: BTreeSet<Capability> = [Capability::Scheduler, Capability::SystemService].into();
        let id = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
        )
        .await
        .unwrap();

        let record = system_service::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.friendly_name, "Scheduler");
        assert_eq!(
            record.service_app_name.as_deref(),
            Some("uptrakit-scheduler")
        );
        assert_eq!(record.status, SystemServiceStatus::Approved);
    }

    #[tokio::test]
    async fn provision_reuses_existing_service() {
        let db = test_db().await;
        let caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        let id1 = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
        )
        .await
        .unwrap();
        let id2 = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
        )
        .await
        .unwrap();
        assert_eq!(id1, id2, "should reuse existing record");
    }

    /// Helper: create a default tenant for FK constraints.
    async fn create_tenant(db: &DatabaseConnection) -> Uuid {
        use sea_orm::ActiveModelTrait;
        use uptrakit_shared_db::entity::tenant;

        let tenant_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: ActiveValue::Set(tenant_id),
            name: ActiveValue::Set("test-tenant".to_string()),
            slug: ActiveValue::Set(format!("test-{tenant_id}")),
            is_default: ActiveValue::Set(false),
            created_at: ActiveValue::Set(now),
            updated_at: ActiveValue::Set(now),
            deactivated_at: ActiveValue::Set(None),
        }
        .insert(db)
        .await
        .expect("insert test tenant");
        tenant_id
    }

    #[tokio::test]
    async fn provision_tenant_service_creates_new() {
        let db = test_db().await;
        let tenant_id = create_tenant(&db).await;
        let caps: BTreeSet<Capability> =
            [Capability::SoftwareDiscovery, Capability::UpdateHooks].into();
        let id = provision_embedded_tenant_service(
            &db,
            tenant_id,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
        )
        .await
        .unwrap();

        let record = service::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.friendly_name, "Embedded Agent");
        assert_eq!(record.tenant_id, tenant_id);
        assert_eq!(record.service_app_name.as_deref(), Some("uptrakit-agent"));
        assert_eq!(record.status, ServiceStatus::Approved);
    }

    #[tokio::test]
    async fn provision_tenant_service_reuses_existing() {
        let db = test_db().await;
        let tenant_id = create_tenant(&db).await;
        let caps: BTreeSet<Capability> = [Capability::SoftwareDiscovery].into();
        let id1 = provision_embedded_tenant_service(
            &db,
            tenant_id,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
        )
        .await
        .unwrap();
        let id2 = provision_embedded_tenant_service(
            &db,
            tenant_id,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
        )
        .await
        .unwrap();
        assert_eq!(id1, id2, "should reuse existing record");
    }

    #[tokio::test]
    async fn provision_tenant_service_isolates_by_tenant() {
        let db = test_db().await;
        let t1 = create_tenant(&db).await;
        let t2 = create_tenant(&db).await;
        let caps: BTreeSet<Capability> = [Capability::SoftwareDiscovery].into();

        let id1 = provision_embedded_tenant_service(
            &db,
            t1,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
        )
        .await
        .unwrap();
        let id2 = provision_embedded_tenant_service(
            &db,
            t2,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
        )
        .await
        .unwrap();
        assert_ne!(id1, id2, "different tenants should get different services");
    }
}
