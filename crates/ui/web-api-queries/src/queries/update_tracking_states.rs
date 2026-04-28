//! Query helpers that load software and connectivity state for update-tracking
//! services.

use std::collections::HashMap;

use sea_orm::{ColumnTrait, FromQueryResult, QueryFilter, QuerySelect, RelationTrait as _};
use uuid::Uuid;

use uptrakit_shared_db::entity::{host, service, service_host};

// Re-export from the local software_states module for shared update-tracking use.
pub use super::software_states::load_software_states_for_tenant;
pub use super::software_states::load_software_states_page_for_tenant;

/// Projection: one row per (service, host) pair for agent connectivity queries.
#[derive(Debug, FromQueryResult)]
struct ServiceHostConnRow {
    service_id: Uuid,
    host_id: Uuid,
    client_version: Option<String>,
    last_seen_at: Option<time::OffsetDateTime>,
}

/// Per-service connectivity data used to synthesise `HostConnectivityUpdated` events.
#[derive(Debug)]
pub struct AgentConnectivityInfo {
    /// Service UUID.
    pub service_id: uuid::Uuid,
    /// All hosts linked to this service.
    pub host_ids: Vec<uuid::Uuid>,
    /// Agent binary version (`services.client_version`). `None` when never reported.
    pub client_version: Option<String>,
    /// Timestamp of the last agent activity (`services.last_seen_at`).
    pub last_seen_at: Option<time::OffsetDateTime>,
}

/// Load all approved, non-deactivated agent services for `tenant_id` that
/// carry the `software_discovery` capability, along with their linked hosts
/// and last-activity metadata.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn load_agent_connectivity_for_tenant(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<AgentConnectivityInfo>, sea_orm::DbErr> {
    use uptrakit_shared_db::entity::service::ServiceStatus;

    let tenant_db_local = crate::TenantDb::new(db.clone(), tenant_id);
    let rows: Vec<ServiceHostConnRow> = tenant_db_local
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .join(
            sea_orm::JoinType::InnerJoin,
            service_host::Relation::Host.def(),
        )
        .select_only()
        .column(service_host::Column::ServiceId)
        .column(service_host::Column::HostId)
        .column_as(service::Column::ClientVersion, "client_version")
        .column_as(service::Column::LastSeenAt, "last_seen_at")
        .filter(service::Column::Status.eq(ServiceStatus::Approved))
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(host::Column::DeactivatedAt.is_null())
        .filter(service::Column::Capabilities.like("%\"software_discovery\"%"))
        .into_model::<ServiceHostConnRow>()
        .all(db)
        .await?;

    let mut map: HashMap<Uuid, AgentConnectivityInfo> = HashMap::new();
    for row in rows {
        let entry = map
            .entry(row.service_id)
            .or_insert_with(|| AgentConnectivityInfo {
                service_id: row.service_id,
                host_ids: Vec::new(),
                client_version: row.client_version.clone(),
                last_seen_at: row.last_seen_at,
            });
        entry.host_ids.push(row.host_id);
        if row.last_seen_at > entry.last_seen_at {
            entry.client_version = row.client_version;
            entry.last_seen_at = row.last_seen_at;
        }
    }

    Ok(map.into_values().collect())
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::tenant;
    use uptrakit_wire::{Capability, service_profile::serialize_capabilities};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("default".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_service(db: &DatabaseConnection, tenant_id: Uuid, service_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set(serialize_capabilities(
                &[Capability::SoftwareDiscovery].into_iter().collect(),
            )),
            hostname: Set("agent-host".to_string()),
            friendly_name: Set("Agent".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(Some("1.2.3".to_string())),
            last_seen_at: Set(Some(now)),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_host(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        deactivated_at: Option<OffsetDateTime>,
    ) {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set(format!("Host {host_id}")),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(deactivated_at),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn load_agent_connectivity_for_tenant_excludes_deactivated_hosts() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let service_id = Uuid::now_v7();
        let active_host_id = Uuid::now_v7();
        let deactivated_host_id = Uuid::now_v7();

        insert_tenant(&db, tenant_id).await;
        insert_service(&db, tenant_id, service_id).await;
        insert_host(&db, tenant_id, active_host_id, None).await;
        insert_host(
            &db,
            tenant_id,
            deactivated_host_id,
            Some(OffsetDateTime::now_utc()),
        )
        .await;

        let now = OffsetDateTime::now_utc();
        for host_id in [active_host_id, deactivated_host_id] {
            service_host::ActiveModel {
                service_id: Set(service_id),
                host_id: Set(host_id),
                linked_at: Set(now),
            }
            .insert(&db)
            .await
            .unwrap();
        }

        let rows = load_agent_connectivity_for_tenant(&db, tenant_id)
            .await
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].service_id, service_id);
        assert_eq!(rows[0].host_ids, vec![active_host_id]);
    }
}
