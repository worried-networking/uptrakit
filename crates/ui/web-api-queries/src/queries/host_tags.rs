use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect, Set, TransactionTrait,
};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{host_tag, host_tag_assignment};
use uptrakit_web_api_types::host_tags::{
    CreateHostTagRequest, HostTagResponse, HostTagSummary, ListHostTagsQuery, UpdateHostTagRequest,
};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

/// Curated palette of visually distinct, accessible colors.
const COLOR_PALETTE: &[&str] = &[
    "#3B82F6", "#EF4444", "#10B981", "#F59E0B", "#8B5CF6", "#EC4899", "#06B6D4", "#F97316",
    "#6366F1", "#14B8A6", "#E11D48", "#84CC16",
];

// ── Helpers ──────────────────────────────────────────────────────────────────

fn model_to_response(m: host_tag::Model, host_count: u64) -> HostTagResponse {
    HostTagResponse {
        id: m.id,
        name: m.name,
        color: m.color,
        description: m.description,
        created_at: m.created_at,
        updated_at: m.updated_at,
        host_count,
    }
}

fn model_to_summary(m: &host_tag::Model) -> HostTagSummary {
    HostTagSummary {
        id: m.id,
        name: m.name.clone(),
        color: m.color.clone(),
    }
}

/// Pick a color from the palette based on the count of existing active tags.
#[expect(
    clippy::indexing_slicing,
    reason = "index is computed as count % palette.len() so it is always in bounds"
)]
async fn auto_color(tenant_db: &TenantDb) -> String {
    let count = tenant_db
        .find::<host_tag::Entity>()
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .count(tenant_db.db())
        .await
        .unwrap_or(0);
    COLOR_PALETTE[(count as usize) % COLOR_PALETTE.len()].to_string()
}

/// Count hosts assigned to a tag (only active hosts).
async fn count_hosts_for_tag(db: &impl sea_orm::ConnectionTrait, tag_id: Uuid) -> u64 {
    host_tag_assignment::Entity::find()
        .filter(host_tag_assignment::Column::HostTagId.eq(tag_id))
        .count(db)
        .await
        .unwrap_or(0)
}

// ── Public query functions ───────────────────────────────────────────────────

/// List active host tags for a tenant.
#[tracing::instrument(skip_all)]
pub async fn list_host_tags(
    tenant_db: &TenantDb,
    params: &ListHostTagsQuery,
) -> Result<PaginatedResponse<HostTagResponse>, sea_orm::DbErr> {
    let pagination = PaginationParams {
        page: params.page,
        per_page: params.per_page,
    }
    .resolve();

    let mut base_query = tenant_db
        .find::<host_tag::Entity>()
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .order_by_asc(host_tag::Column::Name);

    if let Some(ref search) = params.search
        && !search.is_empty()
    {
        base_query = base_query.filter(host_tag::Column::Name.contains(search));
    }

    let total = base_query.clone().count(tenant_db.db()).await?;

    let tags = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    // Batch-load host counts.
    let tag_ids: Vec<Uuid> = tags.iter().map(|t| t.id).collect();
    let host_counts = batch_count_hosts(tenant_db.db(), &tag_ids).await;

    let items: Vec<HostTagResponse> = tags
        .into_iter()
        .map(|t| {
            let count = host_counts.get(&t.id).copied().unwrap_or(0);
            model_to_response(t, count)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Get a single active host tag.
#[tracing::instrument(skip_all)]
pub async fn get_host_tag(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<HostTagResponse>, sea_orm::DbErr> {
    let Some(tag) = tenant_db
        .find_by_id::<host_tag::Entity, _>(id)
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };
    let host_count = count_hosts_for_tag(tenant_db.db(), id).await;
    Ok(Some(model_to_response(tag, host_count)))
}

/// Create a new host tag.
#[tracing::instrument(skip_all)]
pub async fn create_host_tag(
    tenant_db: &TenantDb,
    req: &CreateHostTagRequest,
) -> Result<HostTagResponse, sea_orm::DbErr> {
    let now = OffsetDateTime::now_utc();
    let color = match &req.color {
        Some(c) => c.clone(),
        None => auto_color(tenant_db).await,
    };

    let model = host_tag::ActiveModel {
        id: Set(Uuid::now_v7()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name.clone()),
        color: Set(color),
        description: Set(req.description.clone()),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let tag = model.insert(tenant_db.db()).await?;
    Ok(model_to_response(tag, 0))
}

/// Update an existing host tag. Returns `None` if not found.
#[tracing::instrument(skip_all)]
pub async fn update_host_tag(
    tenant_db: &TenantDb,
    id: Uuid,
    req: &UpdateHostTagRequest,
) -> Result<Option<HostTagResponse>, sea_orm::DbErr> {
    let Some(tag) = tenant_db
        .find_by_id::<host_tag::Entity, _>(id)
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };

    let mut active: host_tag::ActiveModel = tag.into();

    if let Some(ref name) = req.name {
        active.name = Set(name.clone());
    }
    if let Some(ref color) = req.color {
        active.color = Set(color.clone());
    }
    if let Some(ref desc_val) = req.description {
        if desc_val.is_null() {
            active.description = Set(None);
        } else if let Some(s) = desc_val.as_str() {
            active.description = Set(Some(s.to_string()));
        }
    }
    active.updated_at = Set(OffsetDateTime::now_utc());

    let updated = active.update(tenant_db.db()).await?;
    let host_count = count_hosts_for_tag(tenant_db.db(), id).await;
    Ok(Some(model_to_response(updated, host_count)))
}

/// Soft-delete a host tag and hard-delete all its assignments.
#[tracing::instrument(skip_all)]
pub async fn delete_host_tag(tenant_db: &TenantDb, id: Uuid) -> Result<bool, sea_orm::DbErr> {
    let Some(tag) = tenant_db
        .find_by_id::<host_tag::Entity, _>(id)
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await?
    else {
        return Ok(false);
    };

    let txn = tenant_db.db().begin().await?;

    // Hard-delete assignments first.
    host_tag_assignment::Entity::delete_many()
        .filter(host_tag_assignment::Column::HostTagId.eq(id))
        .exec(&txn)
        .await?;

    // Soft-delete the tag.
    let now = OffsetDateTime::now_utc();
    let mut active: host_tag::ActiveModel = tag.into();
    active.deactivated_at = Set(Some(now));
    active.updated_at = Set(now);
    active.update(&txn).await?;

    txn.commit().await?;
    Ok(true)
}

/// Replace all tag assignments for a host with the given tag IDs.
#[tracing::instrument(skip_all)]
pub async fn set_host_tags(
    tenant_db: &TenantDb,
    host_id: Uuid,
    tag_ids: &[Uuid],
) -> Result<Vec<HostTagSummary>, sea_orm::DbErr> {
    let txn = tenant_db.db().begin().await?;

    // Remove all existing assignments for this host.
    host_tag_assignment::Entity::delete_many()
        .filter(host_tag_assignment::Column::HostId.eq(host_id))
        .exec(&txn)
        .await?;

    if !tag_ids.is_empty() {
        // Verify all tag_ids belong to this tenant and are active.
        let valid_tags = tenant_db
            .find::<host_tag::Entity>()
            .filter(host_tag::Column::Id.is_in(tag_ids.iter().copied()))
            .filter(host_tag::Column::DeactivatedAt.is_null())
            .all(&txn)
            .await?;

        let now = OffsetDateTime::now_utc();
        for tag in &valid_tags {
            let assignment = host_tag_assignment::ActiveModel {
                host_tag_id: Set(tag.id),
                host_id: Set(host_id),
                assigned_at: Set(now),
            };
            assignment.insert(&txn).await?;
        }
    }

    txn.commit().await?;

    // Return the current tags.
    let tags_map = load_host_tags_batch(tenant_db, &[host_id]).await;
    Ok(tags_map.get(&host_id).cloned().unwrap_or_default())
}

/// Batch-load tags for multiple hosts. Returns a map of host_id → tags.
#[tracing::instrument(skip_all)]
pub async fn load_host_tags_batch(
    tenant_db: &TenantDb,
    host_ids: &[Uuid],
) -> HashMap<Uuid, Vec<HostTagSummary>> {
    if host_ids.is_empty() {
        return HashMap::new();
    }

    // Load all assignments for these hosts.
    let assignments = match host_tag_assignment::Entity::find()
        .filter(host_tag_assignment::Column::HostId.is_in(host_ids.iter().copied()))
        .all(tenant_db.db())
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("Failed to load host tag assignments: {e}");
            return HashMap::new();
        }
    };

    if assignments.is_empty() {
        return HashMap::new();
    }

    // Collect unique tag IDs.
    let tag_ids: Vec<Uuid> = assignments
        .iter()
        .map(|a| a.host_tag_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    // Load all referenced tags (active only, tenant-scoped).
    let tags = match tenant_db
        .find::<host_tag::Entity>()
        .filter(host_tag::Column::Id.is_in(tag_ids))
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("Failed to load host tags: {e}");
            return HashMap::new();
        }
    };

    let tags_by_id: HashMap<Uuid, &host_tag::Model> = tags.iter().map(|t| (t.id, t)).collect();

    // Build the map.
    let mut result: HashMap<Uuid, Vec<HostTagSummary>> = HashMap::new();
    for assignment in &assignments {
        if let Some(tag) = tags_by_id.get(&assignment.host_tag_id) {
            result
                .entry(assignment.host_id)
                .or_default()
                .push(model_to_summary(tag));
        }
    }

    result
}

/// Batch count hosts per tag.
async fn batch_count_hosts(
    db: &impl sea_orm::ConnectionTrait,
    tag_ids: &[Uuid],
) -> HashMap<Uuid, u64> {
    if tag_ids.is_empty() {
        return HashMap::new();
    }

    let assignments = match host_tag_assignment::Entity::find()
        .filter(host_tag_assignment::Column::HostTagId.is_in(tag_ids.iter().copied()))
        .all(db)
        .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("Failed to batch count hosts for tags: {e}");
            return HashMap::new();
        }
    };

    let mut counts: HashMap<Uuid, u64> = HashMap::new();
    for a in &assignments {
        *counts.entry(a.host_tag_id).or_default() += 1;
    }
    counts
}

/// Batch delete multiple host tags (soft-delete + hard-delete assignments).
#[expect(clippy::type_complexity, reason = "complex SeaORM query return type")]
#[tracing::instrument(skip_all)]
pub async fn batch_delete_host_tags(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> Result<(Vec<Uuid>, Vec<(Uuid, String)>), sea_orm::DbErr> {
    let tags = tenant_db
        .find::<host_tag::Entity>()
        .filter(host_tag::Column::Id.is_in(ids.iter().copied()))
        .filter(host_tag::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await?;

    let found: HashMap<Uuid, host_tag::Model> = tags.into_iter().map(|t| (t.id, t)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();
    let txn = tenant_db.db().begin().await?;

    for (id, tag) in found {
        // Hard-delete assignments.
        host_tag_assignment::Entity::delete_many()
            .filter(host_tag_assignment::Column::HostTagId.eq(id))
            .exec(&txn)
            .await?;

        // Soft-delete the tag.
        let mut active: host_tag::ActiveModel = tag.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(&txn).await?;
        succeeded.push(id);
    }

    txn.commit().await?;
    Ok((succeeded, failed))
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test helpers: panics on setup failure are acceptable"
    )]

    use super::*;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection, Set};
    use uptrakit_shared_db::entity::{host, tenant};

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
    async fn create_and_get_host_tag() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let req = CreateHostTagRequest {
            name: "production".to_string(),
            color: Some("#3B82F6".to_string()),
            description: Some("Prod hosts".to_string()),
        };
        let tag = create_host_tag(&tenant_db, &req).await.unwrap();
        assert_eq!(tag.name, "production");
        assert_eq!(tag.color, "#3B82F6");
        assert_eq!(tag.host_count, 0);

        let fetched = get_host_tag(&tenant_db, tag.id).await.unwrap().unwrap();
        assert_eq!(fetched.name, "production");
    }

    #[tokio::test]
    async fn create_host_tag_auto_color() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let req = CreateHostTagRequest {
            name: "test".to_string(),
            color: None,
            description: None,
        };
        let tag = create_host_tag(&tenant_db, &req).await.unwrap();
        assert_eq!(tag.color, COLOR_PALETTE[0]);
    }

    #[tokio::test]
    async fn update_host_tag_name_and_description() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let tag = create_host_tag(
            &tenant_db,
            &CreateHostTagRequest {
                name: "old".to_string(),
                color: Some("#3B82F6".to_string()),
                description: None,
            },
        )
        .await
        .unwrap();

        let updated = update_host_tag(
            &tenant_db,
            tag.id,
            &UpdateHostTagRequest {
                name: Some("new".to_string()),
                color: None,
                description: Some(serde_json::Value::String("desc".to_string())),
            },
        )
        .await
        .unwrap()
        .unwrap();

        assert_eq!(updated.name, "new");
        assert_eq!(updated.description.as_deref(), Some("desc"));
    }

    #[tokio::test]
    async fn delete_host_tag_soft_deletes() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let tag = create_host_tag(
            &tenant_db,
            &CreateHostTagRequest {
                name: "deleteme".to_string(),
                color: Some("#EF4444".to_string()),
                description: None,
            },
        )
        .await
        .unwrap();

        assert!(delete_host_tag(&tenant_db, tag.id).await.unwrap());
        assert!(get_host_tag(&tenant_db, tag.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_host_tags_replaces_all() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host_record(&db, host_id, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        let tag1 = create_host_tag(
            &tenant_db,
            &CreateHostTagRequest {
                name: "tag1".to_string(),
                color: Some("#3B82F6".to_string()),
                description: None,
            },
        )
        .await
        .unwrap();

        let tag2 = create_host_tag(
            &tenant_db,
            &CreateHostTagRequest {
                name: "tag2".to_string(),
                color: Some("#EF4444".to_string()),
                description: None,
            },
        )
        .await
        .unwrap();

        // Set both tags.
        let result = set_host_tags(&tenant_db, host_id, &[tag1.id, tag2.id])
            .await
            .unwrap();
        assert_eq!(result.len(), 2);

        // Replace with just tag2.
        let result = set_host_tags(&tenant_db, host_id, &[tag2.id])
            .await
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "tag2");

        // Clear all.
        let result = set_host_tags(&tenant_db, host_id, &[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn load_host_tags_batch_returns_correct_map() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host1 = Uuid::now_v7();
        let host2 = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host_record(&db, host1, tenant_id).await;
        insert_host_record(&db, host2, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        let tag = create_host_tag(
            &tenant_db,
            &CreateHostTagRequest {
                name: "shared".to_string(),
                color: Some("#3B82F6".to_string()),
                description: None,
            },
        )
        .await
        .unwrap();

        set_host_tags(&tenant_db, host1, &[tag.id]).await.unwrap();

        let map = load_host_tags_batch(&tenant_db, &[host1, host2]).await;
        assert_eq!(map.get(&host1).map(|v| v.len()), Some(1));
        assert!(!map.contains_key(&host2));
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let db = setup_test_db().await;
        let tenant_a = Uuid::now_v7();
        let tenant_b = Uuid::now_v7();
        insert_tenant(&db, tenant_a).await;
        insert_tenant(&db, tenant_b).await;

        let db_a = TenantDb::new(db.clone(), tenant_a);
        let db_b = TenantDb::new(db.clone(), tenant_b);

        create_host_tag(
            &db_a,
            &CreateHostTagRequest {
                name: "a-tag".to_string(),
                color: Some("#3B82F6".to_string()),
                description: None,
            },
        )
        .await
        .unwrap();

        let list = list_host_tags(&db_b, &ListHostTagsQuery::default())
            .await
            .unwrap();
        assert!(list.items.is_empty());
    }

    #[tokio::test]
    async fn list_host_tags_search() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);

        for name in ["production", "staging", "dev"] {
            create_host_tag(
                &tenant_db,
                &CreateHostTagRequest {
                    name: name.to_string(),
                    color: Some("#3B82F6".to_string()),
                    description: None,
                },
            )
            .await
            .unwrap();
        }

        let result = list_host_tags(
            &tenant_db,
            &ListHostTagsQuery {
                search: Some("prod".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].name, "production");
    }
}
