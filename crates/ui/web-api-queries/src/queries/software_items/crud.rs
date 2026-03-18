//! CRUD operations for software items: create, list, get, update, delete, batch.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, RelationTrait, Set, TransactionTrait,
};
use std::collections::HashMap;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, prelude::*, software_item,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::software_items::{
    CreateSoftwareItemRequest, ListSoftwareItemsParams, SoftwareItemDetailResponse,
    SoftwareItemResponse, UpdateSoftwareItemRequest,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;
use crate::token_utils::generate_uuid;

use super::{
    SoftwareItemQueryError, build_detail_response, build_list_response, count_linked_hosts,
    host_update_available, load_item_hosts, load_latest_version_for_item, load_plugins,
};

// (installed_version, latest_version, installed_display_version, latest_release_metadata)
type VersionPair = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<serde_json::Value>,
);

#[derive(Debug, FromQueryResult)]
struct ItemHostCount {
    software_item_id: Uuid,
    count: i64,
}

#[derive(Debug, FromQueryResult)]
struct ItemPluginType {
    software_item_id: Uuid,
    plugin_type: String,
}

#[derive(Debug, FromQueryResult)]
struct InstalledVersionRow {
    software_item_id: Uuid,
    installed_version: Option<String>,
    latest_version: Option<String>,
    installed_display_version: Option<String>,
    latest_release_metadata: Option<serde_json::Value>,
}

/// Pre-loaded enrichment data for the list endpoint.
struct ListEnrichment {
    host_counts: HashMap<Uuid, u64>,
    plugins_map: HashMap<Uuid, Vec<String>>,
    latest_versions: HashMap<Uuid, String>,
    installed_map: HashMap<Uuid, Vec<VersionPair>>,
}

/// Bulk-load latest versions for multiple software items.
async fn bulk_load_latest_versions(
    db: &sea_orm::DatabaseConnection,
    item_ids: &[Uuid],
) -> HashMap<Uuid, String> {
    #[derive(Debug, FromQueryResult)]
    struct ItemLatestRow {
        software_item_id: Uuid,
        latest_version: Option<String>,
    }

    let rows: Vec<ItemLatestRow> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::LatestVersion)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .filter(host_software_item::Column::LatestVersion.is_not_null())
        .filter(host::Column::DeactivatedAt.is_null())
        .into_model::<ItemLatestRow>()
        .all(db)
        .await
        .unwrap_or_default();

    let mut map: HashMap<Uuid, String> = HashMap::new();
    for row in rows {
        if let Some(v) = row.latest_version {
            map.entry(row.software_item_id)
                .and_modify(|existing| {
                    if v > *existing {
                        *existing = v.clone();
                    }
                })
                .or_insert(v);
        }
    }
    map
}

/// Bulk-load the four enrichment queries needed by `list_software_items`.
async fn bulk_load_list_enrichment(
    db: &sea_orm::DatabaseConnection,
    item_ids: &[Uuid],
    host_id_filter: Option<Uuid>,
) -> super::Result<ListEnrichment> {
    use sea_orm::sea_query::Expr;

    let host_counts: HashMap<Uuid, u64> = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column_as(
            {
                use sea_orm::sea_query::ExprTrait;
                Expr::col(host_software_item::Column::HostId).count()
            },
            "count",
        )
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .filter(host::Column::DeactivatedAt.is_null())
        .group_by(host_software_item::Column::SoftwareItemId)
        .into_model::<ItemHostCount>()
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|row| (row.software_item_id, row.count as u64))
        .collect();

    let plugin_type_rows: Vec<ItemPluginType> = HostSoftwareItemPlugin::find()
        .select_only()
        .column(host_software_item_plugin::Column::SoftwareItemId)
        .column(host_software_item_plugin::Column::PluginType)
        .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .into_model::<ItemPluginType>()
        .all(db)
        .await
        .context_to()?;

    let mut plugins_map: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in plugin_type_rows {
        let entry = plugins_map.entry(row.software_item_id).or_default();
        if !entry.contains(&row.plugin_type) {
            entry.push(row.plugin_type);
        }
    }

    let latest_versions = bulk_load_latest_versions(db, item_ids).await;

    let mut installed_query = HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::InstalledVersion)
        .column(host_software_item::Column::LatestVersion)
        .column(host_software_item::Column::InstalledDisplayVersion)
        .column(host_software_item::Column::LatestReleaseMetadata)
        .join(
            sea_orm::JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(item_ids.to_vec()))
        .filter(host::Column::DeactivatedAt.is_null());

    if let Some(host_id) = host_id_filter {
        installed_query = installed_query.filter(host_software_item::Column::HostId.eq(host_id));
    }

    let installed_rows: Vec<InstalledVersionRow> = installed_query
        .into_model::<InstalledVersionRow>()
        .all(db)
        .await
        .context_to()?;

    let mut installed_map: HashMap<Uuid, Vec<VersionPair>> = HashMap::new();
    for row in installed_rows {
        installed_map
            .entry(row.software_item_id)
            .or_default()
            .push((
                row.installed_version,
                row.latest_version,
                row.installed_display_version,
                row.latest_release_metadata,
            ));
    }

    Ok(ListEnrichment {
        host_counts,
        plugins_map,
        latest_versions,
        installed_map,
    })
}

// ---------------------------------------------------------------------------
// Public query functions
// ---------------------------------------------------------------------------

/// Find a non-deactivated software item by ID, scoped to a tenant.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_active_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    id: Uuid,
) -> Option<software_item::Model> {
    SoftwareItem::find_by_id(id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .ok()
        .flatten()
}

/// Create a new software item (catalog entry only). Checks unique name constraint.
#[tracing::instrument(skip_all)]
pub async fn create_software_item(
    tenant_db: &TenantDb,
    req: CreateSoftwareItemRequest,
) -> super::Result<SoftwareItemResponse> {
    let txn = tenant_db.db().begin().await.context_to()?;

    let duplicate = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::Name.eq(&req.name))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(&txn)
        .await
        .context_to()?;

    if duplicate.is_some() {
        bail!(SoftwareItemQueryError::DuplicateItem);
    }

    let now = OffsetDateTime::now_utc();
    let model = software_item::ActiveModel {
        id: Set(generate_uuid()),
        tenant_id: Set(tenant_db.tenant_id),
        name: Set(req.name),
        featured: Set(req.featured),
        icon_url: Set(req.icon_url),
        last_checked_at: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    };

    let inserted = model.insert(&txn).await.context_to()?;
    txn.commit().await.context_to()?;

    Ok(build_list_response(
        &inserted,
        vec![],
        0,
        None,
        None,
        None,
        None,
        false,
    ))
}

/// Build the EXISTS subquery that checks whether a software item has at least one
/// active host assignment where `installed_version != latest_version` (both non-null).
fn build_updatable_exists_subquery() -> sea_orm::sea_query::SelectStatement {
    use sea_orm::sea_query::{BinOper, Expr, ExprTrait, Query};

    Query::select()
        .expr(Expr::val(1_i32))
        .from(host_software_item::Entity)
        .inner_join(
            host::Entity,
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::HostId,
            ))
            .equals((host::Entity, host::Column::Id)),
        )
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::SoftwareItemId,
            ))
            .equals((software_item::Entity, software_item::Column::Id)),
        )
        .and_where(Expr::col((host::Entity, host::Column::DeactivatedAt)).is_null())
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::InstalledVersion,
            ))
            .is_not_null(),
        )
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::LatestVersion,
            ))
            .is_not_null(),
        )
        .and_where(
            Expr::col((
                host_software_item::Entity,
                host_software_item::Column::InstalledVersion,
            ))
            .binary(
                BinOper::NotEqual,
                Expr::col((
                    host_software_item::Entity,
                    host_software_item::Column::LatestVersion,
                )),
            ),
        )
        .take()
}

#[tracing::instrument(skip_all)]
pub async fn list_software_items(
    tenant_db: &TenantDb,
    params: &ListSoftwareItemsParams,
) -> super::Result<PaginatedResponse<SoftwareItemResponse>> {
    use sea_orm::sea_query::Expr;

    let pagination = params.pagination().resolve();

    let mut base_query = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .order_by(
            sea_orm::sea_query::Func::lower(sea_orm::sea_query::Expr::col(
                software_item::Column::Name,
            )),
            sea_orm::sea_query::Order::Asc,
        );

    if let Some(featured) = params.featured {
        base_query = base_query.filter(software_item::Column::Featured.eq(featured));
    }

    if let Some(host_id) = params.host_id {
        base_query = base_query
            .join(
                sea_orm::JoinType::InnerJoin,
                host_software_item::Relation::SoftwareItem.def().rev(),
            )
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::DeactivatedAt.is_null());
    }

    if let Some(updatable) = params.updatable {
        let exists_sub = build_updatable_exists_subquery();
        if updatable {
            base_query = base_query.filter(Expr::exists(exists_sub));
        } else {
            base_query = base_query.filter(Expr::not_exists(exists_sub));
        }
    }

    if let Some(plugin_type) = &params.plugin_type {
        use sea_orm::sea_query::{BinOper, ExprTrait, Query};
        let plugin_type_sub = Query::select()
            .expr(Expr::val(1_i32))
            .from(host_software_item_plugin::Entity)
            .and_where(
                Expr::col((
                    host_software_item_plugin::Entity,
                    host_software_item_plugin::Column::SoftwareItemId,
                ))
                .equals((software_item::Entity, software_item::Column::Id)),
            )
            .and_where(
                Expr::col((
                    host_software_item_plugin::Entity,
                    host_software_item_plugin::Column::PluginType,
                ))
                .binary(BinOper::Equal, Expr::val(plugin_type.as_str())),
            )
            .take();
        base_query = base_query.filter(Expr::exists(plugin_type_sub));
    }

    let total = base_query
        .clone()
        .count(tenant_db.db())
        .await
        .context_to()?;

    let items = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await
        .context_to()?;

    if items.is_empty() {
        return Ok(PaginatedResponse::new(vec![], total, pagination));
    }

    let item_ids: Vec<Uuid> = items.iter().map(|i| i.id).collect();
    let host_id_filter = params.host_id;

    let mut enrichment =
        bulk_load_list_enrichment(tenant_db.db(), &item_ids, host_id_filter).await?;

    let response: Vec<SoftwareItemResponse> = items
        .iter()
        .map(|item| {
            let plugins = enrichment.plugins_map.remove(&item.id).unwrap_or_default();
            let host_count = enrichment.host_counts.get(&item.id).copied().unwrap_or(0);
            let latest_version = enrichment.latest_versions.get(&item.id).cloned();
            let update_available = enrichment
                .installed_map
                .get(&item.id)
                .map(|pairs| {
                    pairs
                        .iter()
                        .any(|(iv, lv, _, _)| host_update_available(iv.as_deref(), lv.as_deref()))
                })
                .unwrap_or(false);
            let (installed_version, installed_display_version, latest_release_metadata) =
                if host_id_filter.is_some() {
                    enrichment
                        .installed_map
                        .get(&item.id)
                        .and_then(|pairs| pairs.first())
                        .map(|(iv, _lv, idv, lrm)| (iv.clone(), idv.clone(), lrm.clone()))
                        .unwrap_or((None, None, None))
                } else {
                    (None, None, None)
                };
            build_list_response(
                item,
                plugins,
                host_count,
                installed_version,
                installed_display_version,
                latest_version,
                latest_release_metadata,
                update_available,
            )
        })
        .collect();

    Ok(PaginatedResponse::new(response, total, pagination))
}

/// Returns `None` if not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn get_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
) -> super::Result<Option<SoftwareItemDetailResponse>> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(None);
    };

    let hosts = load_item_hosts(tenant_db.db(), id).await;
    let host_count = hosts.len() as u64;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;
    let update_available = hosts.iter().any(|h| h.update_available);

    Ok(Some(build_detail_response(
        item,
        plugins,
        host_count,
        latest_version,
        update_available,
        hosts,
    )))
}

/// Partial update — only `name`, `featured`, and `icon_url` are updatable.
/// Returns `Err(NotFound)` if the item does not exist or is deactivated.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn update_software_item(
    tenant_db: &TenantDb,
    id: Uuid,
    req: UpdateSoftwareItemRequest,
) -> super::Result<SoftwareItemResponse> {
    let existing = find_active_item(tenant_db.db(), tenant_db.tenant_id, id)
        .await
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    if let Some(ref name) = req.name
        && name.is_empty()
    {
        bail!(SoftwareItemQueryError::EmptyName);
    }

    if let Some(ref new_name) = req.name
        && new_name != &existing.name
    {
        let duplicate = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
            .filter(software_item::Column::Name.eq(new_name))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .filter(software_item::Column::Id.ne(id))
            .one(tenant_db.db())
            .await
            .context_to()?;

        if duplicate.is_some() {
            bail!(SoftwareItemQueryError::DuplicateItem);
        }
    }

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = existing.into();

    if let Some(name) = req.name {
        model.name = Set(name);
    }
    if let Some(featured) = req.featured {
        model.featured = Set(featured);
    }
    match req.icon_url {
        Some(serde_json::Value::Null) => model.icon_url = Set(None),
        Some(serde_json::Value::String(s)) => model.icon_url = Set(Some(s)),
        _ => {}
    }
    model.updated_at = Set(now);

    let updated = model.update(tenant_db.db()).await.context_to()?;
    let plugins = load_plugins(tenant_db.db(), id).await;
    let host_count = count_linked_hosts(tenant_db.db(), id).await?;
    let latest_version = load_latest_version_for_item(tenant_db.db(), id).await;

    let update_available = if latest_version.is_some() {
        HostSoftwareItem::find()
            .join(
                sea_orm::JoinType::InnerJoin,
                host_software_item::Relation::Host.def(),
            )
            .filter(host_software_item::Column::SoftwareItemId.eq(id))
            .filter(host_software_item::Column::InstalledVersion.is_not_null())
            .filter(host::Column::DeactivatedAt.is_null())
            .all(tenant_db.db())
            .await
            .unwrap_or_default()
            .iter()
            .any(|h| {
                host_update_available(h.installed_version.as_deref(), h.latest_version.as_deref())
            })
    } else {
        false
    };

    Ok(build_list_response(
        &updated,
        plugins,
        host_count,
        None,
        None,
        latest_version,
        None,
        update_available,
    ))
}

/// Apply a [`SoftwareItemPatch`] to a software item row.
///
/// Only fields present in the patch (`Some(…)`) are written; the rest are left
/// unchanged. Sets `updated_at` when at least one field is modified. Does nothing
/// when the patch is empty.
#[tracing::instrument(skip_all, fields(%item_id))]
pub async fn apply_software_item_patch(
    db: &sea_orm::DatabaseConnection,
    item_id: Uuid,
    patch: &uptrakit_plugin_infrastructure_core::SoftwareItemPatch,
) -> super::Result<()> {
    if patch.is_empty() {
        return Ok(());
    }

    let item = SoftwareItem::find_by_id(item_id)
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(SoftwareItemQueryError::NotFound))?;

    let mut model: software_item::ActiveModel = item.into();

    if let Some(ref icon_url) = patch.icon_url {
        model.icon_url = Set(icon_url.clone());
    }

    model.updated_at = Set(OffsetDateTime::now_utc());
    model.update(db).await.context_to()?;
    Ok(())
}

/// Load featured, active software items for a tenant that have no icon URL set.
///
/// Used after autodiscovery to fire lifecycle plugins on items that may benefit
/// from enrichment (e.g. icon assignment).
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn load_items_needing_enrichment(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Vec<software_item::Model> {
    SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::Featured.eq(true))
        .filter(software_item::Column::IconUrl.is_null())
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .unwrap_or_default()
}

/// Soft-delete a software item. Returns `true` if deleted, `false` if not found.
#[tracing::instrument(skip_all, fields(%id))]
pub async fn delete_software_item(tenant_db: &TenantDb, id: Uuid) -> super::Result<bool> {
    let Some(item) = find_active_item(tenant_db.db(), tenant_db.tenant_id, id).await else {
        return Ok(false);
    };

    let now = OffsetDateTime::now_utc();
    let mut model: software_item::ActiveModel = item.into();
    model.deactivated_at = Set(Some(now));
    model.updated_at = Set(now);
    model.update(tenant_db.db()).await.context_to()?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

/// Feature multiple software items (set `featured = true`).
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_feature_software_items(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> super::Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let items = software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(ids.iter().copied()))
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, software_item::Model> =
        items.into_iter().map(|i| (i.id, i)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, item) in &found {
        let mut active: software_item::ActiveModel = item.clone().into();
        active.featured = Set(true);
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

/// Soft-delete multiple software items.
#[allow(clippy::type_complexity)]
#[tracing::instrument(skip_all)]
pub async fn batch_delete_software_items(
    tenant_db: &TenantDb,
    ids: &[Uuid],
) -> super::Result<(Vec<Uuid>, Vec<(Uuid, String)>)> {
    let items = software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(ids.iter().copied()))
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
        .context_to()?;

    let found: std::collections::HashMap<Uuid, software_item::Model> =
        items.into_iter().map(|i| (i.id, i)).collect();

    let mut succeeded = Vec::new();
    let mut failed: Vec<(Uuid, String)> = Vec::new();

    for id in ids {
        if !found.contains_key(id) {
            failed.push((*id, "not found".to_string()));
        }
    }

    let now = OffsetDateTime::now_utc();

    for (id, item) in &found {
        let mut active: software_item::ActiveModel = item.clone().into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(tenant_db.db()).await.context_to()?;
        succeeded.push(*id);
    }

    Ok((succeeded, failed))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use time::OffsetDateTime;
    use uptrakit_web_api_types::PluginRole;
    use uptrakit_web_api_types::software_items::{HostPluginRoleSummary, SoftwareItemHostSummary};

    #[test]
    fn build_list_response_formats_timestamps() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Node.js".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(
            &item,
            vec!["releases_github".to_string()],
            3,
            None,
            None,
            Some("22.0.0".to_string()),
            None,
            true,
        );

        assert_eq!(resp.name, "Node.js");
        assert_eq!(resp.plugins, vec!["releases_github"]);
        assert_eq!(resp.host_count, 3);
        assert!(resp.last_checked_at.is_some());
        assert!(resp.installed_version.is_none());
        assert_eq!(resp.latest_version.as_deref(), Some("22.0.0"));
        assert!(resp.update_available);
    }

    #[test]
    fn build_list_response_update_available_false_no_latest() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, vec![], 0, None, None, None, None, false);

        assert!(!resp.update_available);
        assert!(resp.installed_version.is_none());
        assert!(resp.latest_version.is_none());
    }

    #[test]
    fn build_list_response_with_installed_version() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: Some(now),
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(
            &item,
            vec!["package_manager_apt".to_string()],
            1,
            Some("1.24.0".to_string()),
            None,
            Some("1.26.0".to_string()),
            None,
            true,
        );

        assert_eq!(resp.installed_version.as_deref(), Some("1.24.0"));
        assert_eq!(resp.latest_version.as_deref(), Some("1.26.0"));
        assert!(resp.update_available);
    }

    #[test]
    fn host_update_available_semantics() {
        assert!(host_update_available(Some("1.0.0"), Some("2.0.0")));
        assert!(!host_update_available(Some("2.0.0"), Some("2.0.0")));
        assert!(!host_update_available(None, Some("2.0.0")));
        assert!(!host_update_available(Some("1.0.0"), None));
        assert!(!host_update_available(None, None));
    }

    #[test]
    fn build_detail_response_includes_hosts() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Redis".to_string(),
            featured: true,
            icon_url: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };
        let hosts = vec![SoftwareItemHostSummary {
            id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            hostname: "web-01".to_string(),
            friendly_name: "Web Server 1".to_string(),
            qualifier: None,
            plugins: vec![HostPluginRoleSummary {
                role: PluginRole::FetchReleases,
                ordinal: 0,
                plugin_config_id: Some(uuid::Uuid::now_v7()),
                plugin_config_name: Some("GitHub Releases".to_string()),
                plugin_type: "releases_github".to_string(),
                package_identifier: "redis/redis".to_string(),
                config_override: Some(serde_json::json!({"asset_patterns": ["redis.*linux"]})),
                execution_site: "auto".to_string(),
            }],
            installed_version: Some("7.2.4".to_string()),
            installed_version_detected_at: Some(now),
            installed_display_version: None,
            latest_version: Some("7.4.0".to_string()),
            latest_release_metadata: None,
            update_available: true,
            active_update_history_id: None,
            update_category: "unknown".to_string(),
            last_updated_at: None,
            linked_at: now,
        }];

        let resp = build_detail_response(
            item,
            vec!["releases_github".to_string()],
            1,
            Some("7.4.0".to_string()),
            true,
            hosts,
        );

        assert_eq!(resp.name, "Redis");
        assert_eq!(resp.plugins, vec!["releases_github"]);
        assert_eq!(resp.hosts.len(), 1);
        assert_eq!(resp.hosts[0].hostname, "web-01");
        assert_eq!(resp.hosts[0].plugins.len(), 1);
        assert_eq!(resp.hosts[0].plugins[0].role, PluginRole::FetchReleases);
        assert_eq!(resp.hosts[0].plugins[0].package_identifier, "redis/redis");
        assert_eq!(resp.hosts[0].installed_version, Some("7.2.4".to_string()));
        assert_eq!(resp.hosts[0].latest_version.as_deref(), Some("7.4.0"));
        assert!(resp.hosts[0].update_available);
        assert!(resp.update_available);
    }

    #[test]
    fn build_list_response_null_last_checked_at() {
        let now = OffsetDateTime::now_utc();
        let item = software_item::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::nil(),
            name: "Nginx".to_string(),
            featured: false,
            icon_url: None,
            last_checked_at: None,
            created_at: now,
            updated_at: now,
            deactivated_at: None,
        };

        let resp = build_list_response(&item, vec![], 0, None, None, None, None, false);

        assert!(!resp.featured);
        assert!(resp.last_checked_at.is_none());
        assert_eq!(resp.host_count, 0);
        assert!(resp.plugins.is_empty());
        assert!(!resp.update_available);
    }

    /// Mock [`PluginConfigOps`] for config-override tests.
    struct MockPluginOps;

    impl uptrakit_plugin_infrastructure_core::PluginMetadataOps for MockPluginOps {
        fn get(
            &self,
            _id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<&uptrakit_plugin_infrastructure_core::descriptor::PluginDescriptor> {
            None
        }

        fn all(&self) -> Vec<&uptrakit_plugin_infrastructure_core::descriptor::PluginDescriptor> {
            vec![]
        }
    }

    impl uptrakit_plugin_infrastructure_core::PluginConfigOps for MockPluginOps {
        fn validate_config(
            &self,
            _id: &uptrakit_shared_types::PluginTypeId,
            config: &serde_json::Value,
        ) -> std::result::Result<(), String> {
            if let Some(url) = config.get("api_base_url").and_then(|v| v.as_str())
                && url.starts_with("http://")
            {
                return Err("api_base_url must use HTTPS".to_string());
            }
            Ok(())
        }

        fn mask_config_secrets(
            &self,
            _id: &uptrakit_shared_types::PluginTypeId,
            config: &serde_json::Value,
        ) -> serde_json::Value {
            config.clone()
        }

        fn restore_config_secrets(
            &self,
            _id: &uptrakit_shared_types::PluginTypeId,
            _incoming: &mut serde_json::Value,
            _existing: &serde_json::Value,
        ) {
        }

        fn validate_package_identifier(
            &self,
            _id: &uptrakit_shared_types::PluginTypeId,
            _value: &str,
        ) -> std::result::Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn validate_config_override_valid_merge() {
        use super::super::host_assignments::validate_config_override;
        let ops = MockPluginOps;
        let base = serde_json::json!({});
        let override_val = serde_json::json!({"tag_strip_prefix": "release-"});
        assert!(validate_config_override(&ops, "releases_github", &base, &override_val).is_ok());
    }

    #[test]
    fn validate_config_override_invalid_merge() {
        use super::super::host_assignments::validate_config_override;
        let ops = MockPluginOps;
        let base = serde_json::json!({});
        let override_val = serde_json::json!({"api_base_url": "http://api.github.com"});
        assert!(validate_config_override(&ops, "releases_github", &base, &override_val).is_err());
    }

    #[test]
    fn validate_config_override_non_object_rejected() {
        use super::super::host_assignments::{ConfigOverrideError, validate_config_override};
        let ops = MockPluginOps;
        let base = serde_json::json!({});
        let override_val = serde_json::json!("not an object");
        let result = validate_config_override(&ops, "releases_github", &base, &override_val);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            ConfigOverrideError::NotAnObject
        ));
    }

    #[test]
    fn validate_execution_site_allows_auto() {
        use super::super::host_assignments::validate_execution_site;
        assert!(validate_execution_site("auto", &PluginRole::DetectVersion).is_ok());
        assert!(validate_execution_site("auto", &PluginRole::FetchReleases).is_ok());
        assert!(validate_execution_site("auto", &PluginRole::ExecuteUpdate).is_ok());
    }

    #[test]
    fn validate_execution_site_allows_agent() {
        use super::super::host_assignments::validate_execution_site;
        assert!(validate_execution_site("agent", &PluginRole::DetectVersion).is_ok());
        assert!(validate_execution_site("agent", &PluginRole::FetchReleases).is_ok());
        assert!(validate_execution_site("agent", &PluginRole::ExecuteUpdate).is_ok());
    }

    #[test]
    fn validate_execution_site_controller_only_for_fetch_releases() {
        use super::super::host_assignments::validate_execution_site;
        assert!(validate_execution_site("controller", &PluginRole::FetchReleases).is_ok());
        assert!(validate_execution_site("controller", &PluginRole::DetectVersion).is_err());
        assert!(validate_execution_site("controller", &PluginRole::ExecuteUpdate).is_err());
    }

    #[test]
    fn validate_execution_site_rejects_invalid() {
        use super::super::host_assignments::validate_execution_site;
        assert!(validate_execution_site("cloud", &PluginRole::DetectVersion).is_err());
        assert!(validate_execution_site("", &PluginRole::FetchReleases).is_err());
        assert!(validate_execution_site("SERVER", &PluginRole::ExecuteUpdate).is_err());
    }
}
