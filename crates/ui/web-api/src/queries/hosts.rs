use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set,
};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{host, prelude::ServiceHost, service, service_host};
use uptrakit_web_api_types::hosts::{HostAgentSummary, HostResponse, UpdateHostRequest};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::services::ServiceStatus;
use uuid::Uuid;

use crate::tenant_db::TenantDb;

// --- Private helpers ---

fn host_to_response(h: host::Model, agents: Vec<HostAgentSummary>) -> HostResponse {
    HostResponse {
        id: h.id,
        machine_id: h.machine_id,
        hostname: h.hostname,
        friendly_name: h.friendly_name,
        os_type: h.os_type,
        os_version: h.os_version,
        architecture: h.architecture,
        ip_address: h.ip_address,
        last_seen_at: h.last_seen_at,
        created_at: h.created_at,
        updated_at: h.updated_at,
        agents,
    }
}

pub(crate) async fn load_host_agents(tenant_db: &TenantDb, host_id: Uuid) -> Vec<HostAgentSummary> {
    let links = match ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(links) => links,
        Err(e) => {
            tracing::warn!("Failed to load host agents: {}", e);
            return Vec::new();
        }
    };

    let service_ids: Vec<Uuid> = links.into_iter().map(|link| link.service_id).collect();
    if service_ids.is_empty() {
        return Vec::new();
    }

    let agents = match tenant_db
        .find::<service::Entity>()
        .filter(service::Column::Id.is_in(service_ids))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(agents) => agents,
        Err(e) => {
            tracing::warn!("Failed to load host agents: {}", e);
            return Vec::new();
        }
    };

    agents
        .into_iter()
        .map(|svc| HostAgentSummary {
            id: svc.id,
            friendly_name: svc.friendly_name,
            status: match svc.status {
                service::ServiceStatus::Pending => ServiceStatus::Pending,
                service::ServiceStatus::Approved => ServiceStatus::Approved,
                service::ServiceStatus::Rejected => ServiceStatus::Rejected,
                service::ServiceStatus::Deactivated => ServiceStatus::Deactivated,
            },
        })
        .collect()
}

// --- Public query functions ---

pub async fn list_hosts(
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> Result<PaginatedResponse<HostResponse>, sea_orm::DbErr> {
    let pagination = params.resolve();

    let base_query = tenant_db
        .find::<host::Entity>()
        .filter(host::Column::DeactivatedAt.is_null())
        .order_by_desc(host::Column::CreatedAt);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let hosts = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    // Batch-load agents for all hosts in one pass (2 queries total, not 2N).
    let host_ids: Vec<Uuid> = hosts.iter().map(|h| h.id).collect();

    let all_links = if host_ids.is_empty() {
        vec![]
    } else {
        match ServiceHost::find()
            .filter(service_host::Column::HostId.is_in(host_ids.clone()))
            .all(tenant_db.db())
            .await
        {
            Ok(links) => links,
            Err(e) => {
                tracing::warn!("Failed to load service-host links for page: {}", e);
                vec![]
            }
        }
    };

    // Build a map: host_id → list of service_ids
    let mut host_service_ids: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for link in &all_links {
        host_service_ids
            .entry(link.host_id)
            .or_default()
            .push(link.service_id);
    }

    let all_service_ids: Vec<Uuid> = all_links.iter().map(|l| l.service_id).collect();

    let services_by_id: HashMap<Uuid, service::Model> = if all_service_ids.is_empty() {
        HashMap::new()
    } else {
        match tenant_db
            .find::<service::Entity>()
            .filter(service::Column::Id.is_in(all_service_ids))
            .filter(service::Column::DeactivatedAt.is_null())
            .all(tenant_db.db())
            .await
        {
            Ok(svcs) => svcs.into_iter().map(|s| (s.id, s)).collect(),
            Err(e) => {
                tracing::warn!("Failed to load services for page: {}", e);
                HashMap::new()
            }
        }
    };

    let items: Vec<HostResponse> = hosts
        .into_iter()
        .map(|h| {
            let agents: Vec<HostAgentSummary> = host_service_ids
                .get(&h.id)
                .map(|svc_ids| {
                    svc_ids
                        .iter()
                        .filter_map(|sid| services_by_id.get(sid))
                        .map(|svc| HostAgentSummary {
                            id: svc.id,
                            friendly_name: svc.friendly_name.clone(),
                            status: match svc.status {
                                service::ServiceStatus::Pending => ServiceStatus::Pending,
                                service::ServiceStatus::Approved => ServiceStatus::Approved,
                                service::ServiceStatus::Rejected => ServiceStatus::Rejected,
                                service::ServiceStatus::Deactivated => ServiceStatus::Deactivated,
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();
            host_to_response(h, agents)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the host is not found or is deactivated.
pub async fn get_active_host(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<HostResponse>, sea_orm::DbErr> {
    let Some(h) = tenant_db
        .find_by_id::<host::Entity, _>(id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };
    let agents = load_host_agents(tenant_db, id).await;
    Ok(Some(host_to_response(h, agents)))
}

/// Update the host's friendly name. Returns `None` if not found.
pub async fn update_host(
    tenant_db: &TenantDb,
    id: Uuid,
    body: &UpdateHostRequest,
) -> Result<Option<HostResponse>, sea_orm::DbErr> {
    let Some(h) = tenant_db
        .find_by_id::<host::Entity, _>(id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };

    let mut active: host::ActiveModel = h.into();
    if let Some(ref name) = body.friendly_name {
        active.friendly_name = Set(name.clone());
    }
    active.updated_at = Set(OffsetDateTime::now_utc());

    let updated = active.update(tenant_db.db()).await?;
    let agents = load_host_agents(tenant_db, id).await;
    Ok(Some(host_to_response(updated, agents)))
}

/// Soft-delete a host. Returns `true` if deactivated, `false` if not found.
pub async fn deactivate_host(tenant_db: &TenantDb, id: Uuid) -> Result<bool, sea_orm::DbErr> {
    let Some(h) = tenant_db
        .find_by_id::<host::Entity, _>(id)
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut active: host::ActiveModel = h.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(tenant_db.db()).await?;
    Ok(true)
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use uptrakit_shared_db::entity::{host, service, service_host, tenant};

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, id: Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("Test Tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
    }

    async fn insert_host_record(db: &DatabaseConnection, id: Uuid, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(id.to_string()),
            hostname: Set("test-host".to_string()),
            friendly_name: Set("Test Host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");
    }

    #[tokio::test]
    async fn load_host_agents_filters_by_tenant() {
        let db = setup_test_db().await;
        let now = OffsetDateTime::now_utc();
        let host_id = uuid::Uuid::now_v7();
        let tenant_a = uuid::Uuid::now_v7();
        let tenant_b = uuid::Uuid::now_v7();

        insert_tenant(&db, tenant_a).await;
        insert_tenant(&db, tenant_b).await;
        insert_host_record(&db, host_id, tenant_a).await;

        let service_a = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_a),
            capabilities: Set("[]".to_string()),
            hostname: Set("host-a".to_string()),
            friendly_name: Set("Agent A".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("hash-a".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
        };
        let service_a = service_a.insert(&db).await.unwrap();

        let service_b = service::ActiveModel {
            id: Set(uuid::Uuid::now_v7()),
            tenant_id: Set(tenant_b),
            capabilities: Set("[]".to_string()),
            hostname: Set("host-b".to_string()),
            friendly_name: Set("Agent B".to_string()),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set("hash-b".to_string()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
        };
        let service_b = service_b.insert(&db).await.unwrap();

        let link_a = service_host::ActiveModel {
            service_id: Set(service_a.id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link_a.insert(&db).await.unwrap();

        let link_b = service_host::ActiveModel {
            service_id: Set(service_b.id),
            host_id: Set(host_id),
            linked_at: Set(now),
        };
        link_b.insert(&db).await.unwrap();

        let tenant_db = TenantDb::new_for_test(db.clone(), tenant_a);
        let agents = load_host_agents(&tenant_db, host_id).await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].friendly_name, "Agent A");
    }
}
