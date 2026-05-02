use sea_orm::{
    ActiveModelTrait, ColumnTrait, FromQueryResult, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, RelationTrait, Set,
};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{host, host_software_item, service, service_host, update_history};
use uptrakit_web_api_types::host_tags::HostTagSummary;
use uptrakit_web_api_types::hosts::{
    HostAgentSummary, HostResponse, HostSoftwareStatusSummary, UpdateHostRequest,
};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::services::ServiceStatus;
use uuid::Uuid;

use crate::tenant_db::TenantDb;

// --- Private helpers ---

#[derive(Debug, FromQueryResult)]
struct HostSoftwareVersionRow {
    host_id: Uuid,
    installed_version: Option<String>,
    latest_version: Option<String>,
}

#[derive(Debug, FromQueryResult)]
struct HostSoftwareErrorCountRow {
    host_id: Uuid,
    error_count: i64,
}

fn host_to_response(
    h: host::Model,
    agents: Vec<HostAgentSummary>,
    tags: Vec<HostTagSummary>,
    software_status: HostSoftwareStatusSummary,
) -> HostResponse {
    let features: Vec<String> = h
        .host_features
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();
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
        tags,
        features,
        software_status,
    }
}

#[tracing::instrument(skip_all, fields(host_count = host_ids.len()))]
async fn load_host_software_statuses(
    tenant_db: &TenantDb,
    host_ids: &[Uuid],
) -> HashMap<Uuid, HostSoftwareStatusSummary> {
    if host_ids.is_empty() {
        return HashMap::new();
    }

    let software_rows = match tenant_db
        .find_via_tenant_join::<host_software_item::Entity, host::Entity>(
            host_software_item::Relation::Host.def(),
        )
        .select_only()
        .column(host_software_item::Column::HostId)
        .column(host_software_item::Column::InstalledVersion)
        .column(host_software_item::Column::LatestVersion)
        .filter(host_software_item::Column::HostId.is_in(host_ids.to_vec()))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .into_model::<HostSoftwareVersionRow>()
        .all(tenant_db.db())
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(?error, "Failed to load host software rows");
            return HashMap::new();
        }
    };

    let mut statuses: HashMap<Uuid, HostSoftwareStatusSummary> = HashMap::new();
    for row in software_rows {
        let status = statuses
            .entry(row.host_id)
            .or_insert_with(|| HostSoftwareStatusSummary {
                known: true,
                ..HostSoftwareStatusSummary::default()
            });
        status.known = true;
        if matches!(
            (&row.installed_version, &row.latest_version),
            (Some(installed), Some(latest)) if installed != latest
        ) {
            status.update_count = status.update_count.saturating_add(1);
        }
    }

    let error_rows = match tenant_db
        .find::<update_history::Entity>()
        .select_only()
        .column(update_history::Column::HostId)
        .column_as(update_history::Column::Id.count(), "error_count")
        .filter(update_history::Column::HostId.is_in(host_ids.to_vec()))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Failed))
        .group_by(update_history::Column::HostId)
        .into_model::<HostSoftwareErrorCountRow>()
        .all(tenant_db.db())
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(?error, "Failed to load host software error counts");
            return statuses;
        }
    };

    for row in error_rows {
        let status = statuses.entry(row.host_id).or_default();
        status.error_count = u32::try_from(row.error_count).unwrap_or(u32::MAX);
    }

    statuses
}

#[tracing::instrument(skip_all, fields(%host_id))]
pub(crate) async fn load_host_agents(tenant_db: &TenantDb, host_id: Uuid) -> Vec<HostAgentSummary> {
    let links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
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
                _ => {
                    tracing::warn!("unknown ServiceStatus variant; treating as Pending");
                    ServiceStatus::Pending
                }
            },
        })
        .collect()
}

// --- Public query functions ---

#[tracing::instrument(skip_all)]
pub async fn list_hosts(
    tenant_db: &TenantDb,
    params: &PaginationParams,
) -> Result<PaginatedResponse<HostResponse>, sea_orm::DbErr> {
    let pagination = params.resolve();

    let base_query = tenant_db
        .find::<host::Entity>()
        .filter(host::Column::DeactivatedAt.is_null())
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                host::Column::FriendlyName,
            )),
            sea_orm::sea_query::Order::Asc,
        );

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
        match tenant_db
            .find_via_tenant_join::<service_host::Entity, service::Entity>(
                service_host::Relation::Service.def(),
            )
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

    // Batch-load tags for all hosts on this page.
    let host_tags_map = super::host_tags::load_host_tags_batch(tenant_db, &host_ids).await;
    let host_software_status_map = load_host_software_statuses(tenant_db, &host_ids).await;

    let items: Vec<HostResponse> = hosts
        .into_iter()
        .map(|h| {
            let host_id = h.id;
            let agents: Vec<HostAgentSummary> = host_service_ids
                .get(&host_id)
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
                                _ => {
                                    tracing::warn!(
                                        "unknown ServiceStatus variant; treating as Pending"
                                    );
                                    ServiceStatus::Pending
                                }
                            },
                        })
                        .collect()
                })
                .unwrap_or_default();
            let tags = host_tags_map.get(&host_id).cloned().unwrap_or_default();
            let software_status = host_software_status_map
                .get(&host_id)
                .copied()
                .unwrap_or_default();
            host_to_response(h, agents, tags, software_status)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the host is not found or is deactivated.
#[tracing::instrument(skip_all)]
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
    let tags_map = super::host_tags::load_host_tags_batch(tenant_db, &[id]).await;
    let tags = tags_map.get(&id).cloned().unwrap_or_default();
    let software_status_map = load_host_software_statuses(tenant_db, &[id]).await;
    let software_status = software_status_map.get(&id).copied().unwrap_or_default();
    Ok(Some(host_to_response(h, agents, tags, software_status)))
}

/// Update the host's friendly name. Returns `None` if not found.
#[tracing::instrument(skip_all)]
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
    let tags_map = super::host_tags::load_host_tags_batch(tenant_db, &[id]).await;
    let tags = tags_map.get(&id).cloned().unwrap_or_default();
    let software_status_map = load_host_software_statuses(tenant_db, &[id]).await;
    let software_status = software_status_map.get(&id).copied().unwrap_or_default();
    Ok(Some(host_to_response(
        updated,
        agents,
        tags,
        software_status,
    )))
}

/// Soft-delete a host. Returns `true` if deactivated, `false` if not found.
#[tracing::instrument(skip_all)]
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

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Deactivate multiple hosts (soft-delete).
#[expect(
    clippy::type_complexity,
    reason = "complex SeaORM query return type; extracting aliases would increase verbosity"
)]
#[tracing::instrument(skip_all)]
pub async fn batch_deactivate_hosts(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> std::result::Result<(Vec<Uuid>, Vec<(Uuid, String)>), sea_orm::DbErr> {
    let hosts = tenant_db
        .find::<host::Entity>()
        .filter(host::Column::Id.is_in(ids.iter().copied()))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await?;

    let found: std::collections::HashMap<Uuid, host::Model> =
        hosts.into_iter().map(|h| (h.id, h)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, h) in found {
        let mut active: host::ActiveModel = h.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await?;
        succeeded.push(id);
    }

    Ok((succeeded, failed))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test helpers: panics on setup failure are acceptable"
    )]

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
            host_features: Set(None),
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
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
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
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
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

        let tenant_db = TenantDb::new(db.clone(), tenant_a);
        let agents = load_host_agents(&tenant_db, host_id).await;
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].friendly_name, "Agent A");
    }
}
