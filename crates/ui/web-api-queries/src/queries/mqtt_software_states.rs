//! Query helpers that load software state data for MQTT push messages.

use std::collections::HashMap;

use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect, RelationTrait as _,
};
use uuid::Uuid;

use uptrakit_shared_db::entity::{prelude::*, service, service_host};

// Re-export the canonical software-state loader from the scheduler engine so
// that both the embedded-scheduler and the web-API code paths share one
// implementation.  The scheduler-engine crate owns the single source of truth.
pub use uptrakit_scheduler_engine::software_states::load_software_states_for_tenant;

/// Projection: one row per (service, host) pair for agent connectivity queries.
#[derive(Debug, FromQueryResult)]
struct ServiceHostConnRow {
    service_id: Uuid,
    host_id: Uuid,
    client_version: Option<String>,
    last_seen_at: Option<time::OffsetDateTime>,
}

/// Per-service connectivity data used to synthesise `HostConnectivityUpdated` events.
///
/// One entry per approved, non-deactivated agent service that has the
/// `software_discovery` capability.  The caller is responsible for filtering
/// to currently-connected services before building the events.
#[derive(Debug)]
pub struct AgentConnectivityInfo {
    /// Service UUID.
    pub service_id: uuid::Uuid,
    /// All hosts linked to this service.
    pub host_ids: Vec<uuid::Uuid>,
    /// Agent binary version (`services.client_version`).  `None` when never
    /// reported.
    pub client_version: Option<String>,
    /// Timestamp of the last agent activity (`services.last_seen_at`).
    pub last_seen_at: Option<time::OffsetDateTime>,
}

/// Load all approved, non-deactivated agent services for `tenant_id` that
/// carry the `software_discovery` capability, along with their linked hosts
/// and last-activity metadata.
///
/// The result is intended to be filtered by the caller against the live
/// `ServiceConnectionRegistry` before synthesising `HostConnectivityUpdated`
/// events.  Only one bulk query is performed (no N+1).
///
/// # Errors
///
/// Returns a [`sea_orm::DbErr`] if the database query fails.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn load_agent_connectivity_for_tenant(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<Vec<AgentConnectivityInfo>, sea_orm::DbErr> {
    use uptrakit_shared_db::entity::service::ServiceStatus;

    // Tenant-scoped via join on service (service_host has no tenant_id column).
    let tenant_db_local = crate::TenantDb::new(db.clone(), tenant_id);
    let rows: Vec<ServiceHostConnRow> = tenant_db_local
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .select_only()
        .column(service_host::Column::ServiceId)
        .column(service_host::Column::HostId)
        .column_as(service::Column::ClientVersion, "client_version")
        .column_as(service::Column::LastSeenAt, "last_seen_at")
        .filter(service::Column::Status.eq(ServiceStatus::Approved))
        .filter(service::Column::DeactivatedAt.is_null())
        // Only services with the software_discovery capability.
        .filter(service::Column::Capabilities.like("%\"software_discovery\"%"))
        .into_model::<ServiceHostConnRow>()
        .all(db)
        .await?;

    // Group by service_id, preferring the most-recent last_seen_at.
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
