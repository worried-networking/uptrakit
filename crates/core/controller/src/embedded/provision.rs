//! Auto-provisioning for embedded service records.
//!
//! On startup, each embedded service needs a row in the `system_services`
//! table (for system services like the scheduler) so that the
//! `ServiceConnectionRegistry` and `MessageProcessor` can reference a valid
//! service ID.

use std::collections::BTreeSet;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ActiveValue, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use uptrakit_internal_wire::Capability;
use uptrakit_shared_db::entity::system_service::{self, SystemServiceStatus};
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

#[cfg(test)]
mod tests {
    use super::*;

    async fn test_db() -> DatabaseConnection {
        use sea_orm::{ConnectOptions, ConnectionTrait, Database, Schema};

        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        let schema = Schema::new(db.get_database_backend());

        let stmt = schema.create_table_from_entity(system_service::Entity);
        db.execute(&stmt)
            .await
            .expect("create system_services table");
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
}
