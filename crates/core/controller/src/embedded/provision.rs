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
    QueryOrder,
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
    embedded_owner_key: Uuid,
) -> rootcause::Result<Uuid> {
    let now = time::OffsetDateTime::now_utc();
    let caps_str = capabilities
        .iter()
        .map(|c| c.as_str().to_string())
        .collect::<Vec<_>>()
        .join(",");

    if let Some(existing) = system_service::Entity::find()
        .filter(system_service::Column::IsEmbedded.eq(true))
        .filter(system_service::Column::ServiceAppName.eq(Some(app_name.to_string())))
        .filter(system_service::Column::EmbeddedOwnerKey.eq(embedded_owner_key))
        .order_by_desc(system_service::Column::CreatedAt)
        .one(db)
        .await
        .context("query existing owned embedded system service")?
    {
        return refresh_embedded_system_service(
            db,
            existing,
            app_name,
            friendly_name,
            &caps_str,
            hostname,
            embedded_owner_key,
            now,
        )
        .await;
    }

    if let Some(legacy) = system_service::Entity::find()
        .filter(system_service::Column::IsEmbedded.eq(true))
        .filter(system_service::Column::ServiceAppName.eq(Some(app_name.to_string())))
        .filter(system_service::Column::EmbeddedOwnerKey.is_null())
        .order_by_desc(system_service::Column::CreatedAt)
        .one(db)
        .await
        .context("query legacy embedded system service")?
    {
        tracing::info!(
            service_id = %legacy.id,
            app_name,
            %embedded_owner_key,
            "claiming legacy embedded system service"
        );
        return refresh_embedded_system_service(
            db,
            legacy,
            app_name,
            friendly_name,
            &caps_str,
            hostname,
            embedded_owner_key,
            now,
        )
        .await;
    }

    let service_id = Uuid::now_v7();
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
        is_embedded: ActiveValue::Set(true),
        embedded_owner_key: ActiveValue::Set(Some(embedded_owner_key)),
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
    embedded_owner_key: Uuid,
) -> rootcause::Result<Uuid> {
    let now = time::OffsetDateTime::now_utc();
    let caps_str = capabilities
        .iter()
        .map(|c| c.as_str().to_string())
        .collect::<Vec<_>>()
        .join(",");

    if let Some(existing) = service::Entity::find()
        .filter(service::Column::TenantId.eq(tenant_id))
        .filter(service::Column::IsEmbedded.eq(true))
        .filter(service::Column::ServiceAppName.eq(Some(app_name.to_string())))
        .filter(service::Column::EmbeddedOwnerKey.eq(embedded_owner_key))
        .order_by_desc(service::Column::CreatedAt)
        .one(db)
        .await
        .context("query existing owned embedded tenant service")?
    {
        return refresh_embedded_tenant_service(
            db,
            existing,
            app_name,
            friendly_name,
            &caps_str,
            hostname,
            embedded_owner_key,
            now,
        )
        .await;
    }

    if let Some(legacy) = service::Entity::find()
        .filter(service::Column::TenantId.eq(tenant_id))
        .filter(service::Column::IsEmbedded.eq(true))
        .filter(service::Column::ServiceAppName.eq(Some(app_name.to_string())))
        .filter(service::Column::EmbeddedOwnerKey.is_null())
        .order_by_desc(service::Column::CreatedAt)
        .one(db)
        .await
        .context("query legacy embedded tenant service")?
    {
        tracing::info!(
            service_id = %legacy.id,
            %tenant_id,
            app_name,
            %embedded_owner_key,
            "claiming legacy embedded tenant service"
        );
        return refresh_embedded_tenant_service(
            db,
            legacy,
            app_name,
            friendly_name,
            &caps_str,
            hostname,
            embedded_owner_key,
            now,
        )
        .await;
    }

    let service_id = Uuid::now_v7();
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
        is_embedded: ActiveValue::Set(true),
        embedded_owner_key: ActiveValue::Set(Some(embedded_owner_key)),
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

async fn refresh_embedded_system_service(
    db: &DatabaseConnection,
    existing: system_service::Model,
    app_name: &str,
    friendly_name: &str,
    caps_str: &str,
    hostname: &str,
    embedded_owner_key: Uuid,
    now: time::OffsetDateTime,
) -> rootcause::Result<Uuid> {
    let service_id = existing.id;
    let synthetic_hash = format!("embedded:{service_id}");
    let mut active: system_service::ActiveModel = existing.into();
    active.capabilities = ActiveValue::Set(caps_str.to_string());
    active.hostname = ActiveValue::Set(hostname.to_string());
    active.friendly_name = ActiveValue::Set(friendly_name.to_string());
    active.status = ActiveValue::Set(SystemServiceStatus::Approved);
    active.enrollment_secret_hash = ActiveValue::Set(synthetic_hash);
    active.last_seen_at = ActiveValue::Set(Some(now));
    active.updated_at = ActiveValue::Set(now);
    active.deactivated_at = ActiveValue::Set(None);
    active.service_app_name = ActiveValue::Set(Some(app_name.to_string()));
    active.is_embedded = ActiveValue::Set(true);
    active.embedded_owner_key = ActiveValue::Set(Some(embedded_owner_key));
    active
        .update(db)
        .await
        .context("refresh embedded system service record")?;
    tracing::debug!(
        %service_id,
        app_name,
        "reused embedded system service record"
    );
    Ok(service_id)
}

async fn refresh_embedded_tenant_service(
    db: &DatabaseConnection,
    existing: service::Model,
    app_name: &str,
    friendly_name: &str,
    caps_str: &str,
    hostname: &str,
    embedded_owner_key: Uuid,
    now: time::OffsetDateTime,
) -> rootcause::Result<Uuid> {
    let service_id = existing.id;
    let synthetic_hash = format!("embedded:{service_id}");
    let mut active: service::ActiveModel = existing.into();
    active.capabilities = ActiveValue::Set(caps_str.to_string());
    active.hostname = ActiveValue::Set(hostname.to_string());
    active.friendly_name = ActiveValue::Set(friendly_name.to_string());
    active.status = ActiveValue::Set(ServiceStatus::Approved);
    active.enrollment_secret_hash = ActiveValue::Set(synthetic_hash);
    active.last_seen_at = ActiveValue::Set(Some(now));
    active.updated_at = ActiveValue::Set(now);
    active.deactivated_at = ActiveValue::Set(None);
    active.service_app_name = ActiveValue::Set(Some(app_name.to_string()));
    active.is_embedded = ActiveValue::Set(true);
    active.embedded_owner_key = ActiveValue::Set(Some(embedded_owner_key));
    active
        .update(db)
        .await
        .context("refresh embedded tenant service record")?;
    tracing::debug!(
        %service_id,
        app_name,
        "reused embedded tenant service record"
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
        let owner = Uuid::now_v7();
        let id = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
            owner,
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
        assert!(record.is_embedded);
        assert_eq!(record.embedded_owner_key, Some(owner));
    }

    #[tokio::test]
    async fn provision_reuses_existing_service() {
        let db = test_db().await;
        let caps: BTreeSet<Capability> = [Capability::Scheduler].into();
        let owner = Uuid::now_v7();
        let id1 = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
            owner,
        )
        .await
        .unwrap();
        let id2 = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
            owner,
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
        let owner = Uuid::now_v7();
        let id = provision_embedded_tenant_service(
            &db,
            tenant_id,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
            owner,
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
        assert!(record.is_embedded);
        assert_eq!(record.embedded_owner_key, Some(owner));
    }

    #[tokio::test]
    async fn provision_tenant_service_reuses_existing() {
        let db = test_db().await;
        let tenant_id = create_tenant(&db).await;
        let caps: BTreeSet<Capability> = [Capability::SoftwareDiscovery].into();
        let owner = Uuid::now_v7();
        let id1 = provision_embedded_tenant_service(
            &db,
            tenant_id,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
            owner,
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
            owner,
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
        let owner = Uuid::now_v7();

        let id1 = provision_embedded_tenant_service(
            &db,
            t1,
            "uptrakit-agent",
            "Embedded Agent",
            &caps,
            "localhost",
            owner,
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
            owner,
        )
        .await
        .unwrap();
        assert_ne!(id1, id2, "different tenants should get different services");
    }

    #[tokio::test]
    async fn provision_system_service_isolates_by_owner() {
        let db = test_db().await;
        let caps: BTreeSet<Capability> = [Capability::Scheduler].into();

        let id1 = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
            Uuid::now_v7(),
        )
        .await
        .unwrap();
        let id2 = provision_embedded_system_service(
            &db,
            "uptrakit-scheduler",
            "Scheduler",
            &caps,
            "localhost",
            Uuid::now_v7(),
        )
        .await
        .unwrap();

        assert_ne!(
            id1, id2,
            "different owners should get different system services"
        );
    }

    #[tokio::test]
    async fn provision_tenant_service_isolates_by_owner() {
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
            Uuid::now_v7(),
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
            Uuid::now_v7(),
        )
        .await
        .unwrap();

        assert_ne!(
            id1, id2,
            "different owners should get different tenant services"
        );
    }
}
