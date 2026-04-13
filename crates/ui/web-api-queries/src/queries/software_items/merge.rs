//! Merge preview query logic for software items.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, JoinType,
    QueryFilter, QueryOrder, QuerySelect, RelationTrait, Set, TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, prelude::*, software_item,
};
use uptrakit_web_api_types::software_items::{
    MergeSoftwareItemLinkSummary, MergeSoftwareItemSummary, MergeSoftwareItemsExecuteRequest,
    MergeSoftwareItemsExecuteResponse, MergeSoftwareItemsPreviewRequest,
    MergeSoftwareItemsPreviewResponse,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

use super::SoftwareItemQueryError;

#[derive(Debug, FromQueryResult)]
struct MergeLinkRow {
    software_item_id: Uuid,
    id: Uuid,
    host_id: Uuid,
    qualifier: Option<String>,
    hostname: String,
    friendly_name: String,
}

#[derive(Debug, FromQueryResult)]
struct MergePluginRow {
    software_item_id: Uuid,
    plugin_type: String,
}

#[derive(Clone, Debug)]
struct MergePluginAssignmentRow {
    plugin_type: String,
    plugin_config_id: Option<Uuid>,
    role: String,
    ordinal: i32,
    package_identifier: String,
    config: Option<serde_json::Value>,
    execution_site: String,
}

#[derive(Debug)]
struct MergePlan {
    candidates: Vec<MergeSoftwareItemSummary>,
    survivor: MergeSoftwareItemSummary,
    losers: Vec<MergeSoftwareItemSummary>,
    moved_links: Vec<MergeSoftwareItemLinkSummary>,
    skipped_duplicate_links: Vec<MergeSoftwareItemLinkSummary>,
}

fn equivalent_merge_link(
    left: &MergeSoftwareItemLinkSummary,
    right: &MergeSoftwareItemLinkSummary,
) -> bool {
    left.host_id == right.host_id && left.qualifier == right.qualifier
}

fn merge_link_identity(link: &MergeSoftwareItemLinkSummary) -> (Uuid, Option<String>) {
    (link.host_id, link.qualifier.clone())
}

fn merge_link_model_identity(link: &host_software_item::Model) -> (Uuid, Option<String>) {
    (link.host_id, link.qualifier.clone())
}

fn equivalent_plugin_assignment(
    left: &MergePluginAssignmentRow,
    right: &MergePluginAssignmentRow,
) -> bool {
    left.plugin_type == right.plugin_type
        && left.plugin_config_id == right.plugin_config_id
        && left.package_identifier == right.package_identifier
        && left.execution_site == right.execution_site
        && left.config == right.config
}

fn group_transfer_plan(
    survivor_links: &[MergeSoftwareItemLinkSummary],
    loser_links: Vec<MergeSoftwareItemLinkSummary>,
) -> (
    Vec<MergeSoftwareItemLinkSummary>,
    Vec<MergeSoftwareItemLinkSummary>,
) {
    let mut seen: HashSet<(Uuid, Option<String>)> =
        survivor_links.iter().map(merge_link_identity).collect();

    let mut moved_links = Vec::new();
    let mut skipped_duplicate_links = Vec::new();

    for link in loser_links {
        let duplicate = seen.contains(&merge_link_identity(&link))
            && survivor_links
                .iter()
                .chain(moved_links.iter())
                .any(|known_link| equivalent_merge_link(known_link, &link));

        if duplicate {
            skipped_duplicate_links.push(link);
            continue;
        }

        seen.insert(merge_link_identity(&link));
        moved_links.push(link);
    }

    (moved_links, skipped_duplicate_links)
}

async fn validate_duplicate_link_plugin_assignments<C: ConnectionTrait>(
    db: &C,
    survivor_id: Uuid,
    survivor_links: &[MergeSoftwareItemLinkSummary],
    loser_links: &[MergeSoftwareItemLinkSummary],
) -> super::Result<()> {
    if loser_links.is_empty() {
        return Ok(());
    }

    let mut surviving_link_by_identity: HashMap<(Uuid, Option<String>), Uuid> = survivor_links
        .iter()
        .map(|link| (merge_link_identity(link), link.id))
        .collect();
    let mut seen: HashSet<(Uuid, Option<String>)> =
        survivor_links.iter().map(merge_link_identity).collect();

    let relevant_host_ids: HashSet<Uuid> = loser_links.iter().map(|link| link.host_id).collect();
    let loser_link_ids: Vec<Uuid> = loser_links.iter().map(|link| link.id).collect();

    let duplicate_plugin_rows: Vec<host_software_item_plugin::Model> =
        HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostSoftwareItemId.is_in(loser_link_ids))
            .all(db)
            .await
            .context_to()?;

    let mut duplicate_plugin_rows_by_link_id: HashMap<Uuid, Vec<MergePluginAssignmentRow>> =
        HashMap::new();
    for row in duplicate_plugin_rows {
        duplicate_plugin_rows_by_link_id
            .entry(row.host_software_item_id)
            .or_default()
            .push(MergePluginAssignmentRow {
                plugin_type: row.plugin_type,
                plugin_config_id: row.plugin_config_id,
                role: row.role,
                ordinal: row.ordinal,
                package_identifier: row.package_identifier,
                config: row.config,
                execution_site: row.execution_site,
            });
    }

    let survivor_plugin_rows: Vec<host_software_item_plugin::Model> =
        HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::SoftwareItemId.eq(survivor_id))
            .filter(
                host_software_item_plugin::Column::HostId
                    .is_in(relevant_host_ids.into_iter().collect::<Vec<_>>()),
            )
            .all(db)
            .await
            .context_to()?;
    let mut survivor_rows_by_host_slot: HashMap<(Uuid, String, i32), MergePluginAssignmentRow> =
        survivor_plugin_rows
            .into_iter()
            .map(|row| {
                (
                    (row.host_id, row.role.clone(), row.ordinal),
                    MergePluginAssignmentRow {
                        plugin_type: row.plugin_type,
                        plugin_config_id: row.plugin_config_id,
                        role: row.role,
                        ordinal: row.ordinal,
                        package_identifier: row.package_identifier,
                        config: row.config,
                        execution_site: row.execution_site,
                    },
                )
            })
            .collect();

    for loser_link in loser_links {
        let loser_identity = merge_link_identity(loser_link);
        let duplicate = seen.contains(&loser_identity);

        if duplicate {
            surviving_link_by_identity
                .get(&loser_identity)
                .copied()
                .ok_or_else(|| {
                    report!(SoftwareItemQueryError::InvalidMergeRequest(
                        "matching survivor link missing during duplicate reconciliation"
                            .to_string(),
                    ))
                })?;
        }

        for loser_row in duplicate_plugin_rows_by_link_id
            .get(&loser_link.id)
            .into_iter()
            .flatten()
        {
            let survivor_key = (
                loser_link.host_id,
                loser_row.role.clone(),
                loser_row.ordinal,
            );
            if duplicate {
                if let Some(target_row) = survivor_rows_by_host_slot.get(&survivor_key)
                    && !equivalent_plugin_assignment(target_row, loser_row)
                {
                    bail!(SoftwareItemQueryError::InvalidMergeRequest(
                        "conflicting plugin assignments on duplicate host link".to_string(),
                    ));
                }
            } else if survivor_rows_by_host_slot.contains_key(&survivor_key) {
                bail!(SoftwareItemQueryError::InvalidMergeRequest(
                    "conflicting plugin assignments on duplicate host link".to_string(),
                ));
            } else {
                survivor_rows_by_host_slot.insert(survivor_key, loser_row.clone());
            }
        }

        if !duplicate {
            seen.insert(loser_identity.clone());
            surviving_link_by_identity.insert(loser_identity, loser_link.id);
        }
    }

    Ok(())
}

fn normalize_candidate_ids(candidate_ids: &[Uuid]) -> Vec<Uuid> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for candidate_id in candidate_ids {
        if seen.insert(*candidate_id) {
            normalized.push(*candidate_id);
        }
    }

    normalized
}

fn validate_candidate_ids(candidate_ids: &[Uuid], survivor_id: Uuid) -> super::Result<Vec<Uuid>> {
    let candidate_ids = normalize_candidate_ids(candidate_ids);

    if candidate_ids.len() < 2 {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "at least two distinct candidate_ids are required".to_string(),
        ));
    }

    if !candidate_ids.contains(&survivor_id) {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "survivor_id must be included in candidate_ids".to_string(),
        ));
    }

    Ok(candidate_ids)
}

fn deleted_candidate_ids(candidate_ids: &[Uuid], survivor_id: Uuid) -> Vec<Uuid> {
    candidate_ids
        .iter()
        .copied()
        .filter(|candidate_id| *candidate_id != survivor_id)
        .collect()
}

async fn load_active_candidates<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    candidate_ids: &[Uuid],
) -> super::Result<Vec<software_item::Model>> {
    let items = SoftwareItem::find()
        .filter(software_item::Column::Id.is_in(candidate_ids.to_vec()))
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    if items.len() != candidate_ids.len() {
        let found_ids: HashSet<Uuid> = items.iter().map(|item| item.id).collect();
        let missing_ids: Vec<Uuid> = candidate_ids
            .iter()
            .copied()
            .filter(|candidate_id| !found_ids.contains(candidate_id))
            .collect();

        bail!(SoftwareItemQueryError::InvalidMergeRequest(format!(
            "candidate items must all exist, belong to the tenant, and be active; unresolved ids: {}",
            missing_ids
                .iter()
                .map(Uuid::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    Ok(items)
}

async fn build_item_summary(
    item: &software_item::Model,
    host_count: u64,
    plugins: Vec<String>,
) -> super::Result<MergeSoftwareItemSummary> {
    Ok(MergeSoftwareItemSummary {
        id: item.id,
        name: item.name.clone(),
        host_count,
        plugins,
    })
}

async fn load_active_plugin_types<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    candidate_ids: &[Uuid],
) -> super::Result<HashMap<Uuid, Vec<String>>> {
    let rows: Vec<MergePluginRow> = HostSoftwareItemPlugin::find()
        .select_only()
        .column(host_software_item_plugin::Column::SoftwareItemId)
        .column(host_software_item_plugin::Column::PluginType)
        .join(
            JoinType::InnerJoin,
            host_software_item_plugin::Relation::HostSoftwareItem.def(),
        )
        .join(
            JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(candidate_ids.to_vec()))
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .order_by_asc(host_software_item_plugin::Column::SoftwareItemId)
        .order_by_asc(host_software_item_plugin::Column::PluginType)
        .into_model::<MergePluginRow>()
        .all(db)
        .await
        .context_to()?;

    let mut plugin_types: HashMap<Uuid, Vec<String>> = HashMap::new();
    for row in rows {
        let entry = plugin_types.entry(row.software_item_id).or_default();
        if !entry.contains(&row.plugin_type) {
            entry.push(row.plugin_type);
        }
    }

    Ok(plugin_types)
}

async fn load_candidate_links<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    candidate_ids: &[Uuid],
) -> super::Result<Vec<MergeLinkRow>> {
    HostSoftwareItem::find()
        .select_only()
        .column(host_software_item::Column::SoftwareItemId)
        .column(host_software_item::Column::Id)
        .column(host_software_item::Column::HostId)
        .column(host_software_item::Column::Qualifier)
        .column_as(host::Column::Hostname, "hostname")
        .column_as(host::Column::FriendlyName, "friendly_name")
        .join(
            JoinType::InnerJoin,
            host_software_item::Relation::Host.def(),
        )
        .filter(host_software_item::Column::SoftwareItemId.is_in(candidate_ids.to_vec()))
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .order_by_asc(host_software_item::Column::SoftwareItemId)
        .order_by_asc(host_software_item::Column::HostId)
        .order_by_asc(host_software_item::Column::Qualifier)
        .order_by_asc(host_software_item::Column::Id)
        .into_model::<MergeLinkRow>()
        .all(db)
        .await
        .context_to()
}

fn to_link_summary(row: MergeLinkRow) -> MergeSoftwareItemLinkSummary {
    MergeSoftwareItemLinkSummary {
        id: row.id,
        host_id: row.host_id,
        hostname: row.hostname,
        friendly_name: row.friendly_name,
        qualifier: row.qualifier,
    }
}

async fn build_merge_plan<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    candidate_ids: &[Uuid],
    survivor_id: Uuid,
) -> super::Result<MergePlan> {
    let candidate_items = load_active_candidates(db, tenant_id, candidate_ids).await?;
    let item_by_id: HashMap<Uuid, software_item::Model> = candidate_items
        .into_iter()
        .map(|item| (item.id, item))
        .collect();

    let mut link_rows = load_candidate_links(db, tenant_id, candidate_ids).await?;
    let plugin_types = load_active_plugin_types(db, tenant_id, candidate_ids).await?;
    let mut host_counts: HashMap<Uuid, u64> = HashMap::new();
    for row in &link_rows {
        *host_counts.entry(row.software_item_id).or_insert(0) += 1;
    }

    let mut candidates = Vec::with_capacity(candidate_ids.len());
    for candidate_id in candidate_ids {
        let item = item_by_id.get(candidate_id).ok_or_else(|| {
            report!(SoftwareItemQueryError::InvalidMergeRequest(
                "candidate items must all exist, belong to the tenant, and be active".to_string(),
            ))
        })?;
        candidates.push(
            build_item_summary(
                item,
                host_counts.get(candidate_id).copied().unwrap_or(0),
                plugin_types.get(candidate_id).cloned().unwrap_or_default(),
            )
            .await?,
        );
    }

    let survivor = candidates
        .iter()
        .find(|candidate| candidate.id == survivor_id)
        .cloned()
        .ok_or_else(|| {
            report!(SoftwareItemQueryError::InvalidMergeRequest(
                "survivor_id must be included in candidate_ids".to_string(),
            ))
        })?;

    let losers: Vec<MergeSoftwareItemSummary> = candidates
        .iter()
        .filter(|candidate| candidate.id != survivor_id)
        .cloned()
        .collect();

    let candidate_index: HashMap<Uuid, usize> = candidate_ids
        .iter()
        .enumerate()
        .map(|(index, candidate_id)| (*candidate_id, index))
        .collect();

    link_rows.sort_by(|left, right| {
        (
            candidate_index[&left.software_item_id],
            left.host_id,
            left.qualifier.clone(),
            left.id,
        )
            .cmp(&(
                candidate_index[&right.software_item_id],
                right.host_id,
                right.qualifier.clone(),
                right.id,
            ))
    });

    let mut survivor_links = Vec::new();
    let mut loser_links = Vec::new();

    for row in link_rows {
        let software_item_id = row.software_item_id;
        let summary = to_link_summary(row);
        if software_item_id == survivor_id {
            survivor_links.push(summary);
        } else {
            loser_links.push(summary);
        }
    }

    validate_duplicate_link_plugin_assignments(db, survivor_id, &survivor_links, &loser_links)
        .await?;

    let (moved_links, skipped_duplicate_links) = group_transfer_plan(&survivor_links, loser_links);
    Ok(MergePlan {
        candidates,
        survivor,
        losers,
        moved_links,
        skipped_duplicate_links,
    })
}

fn to_preview_response(plan: MergePlan) -> MergeSoftwareItemsPreviewResponse {
    MergeSoftwareItemsPreviewResponse {
        candidate_count: plan.candidates.len() as u64,
        loser_count: plan.losers.len() as u64,
        moved_link_count: plan.moved_links.len() as u64,
        skipped_duplicate_link_count: plan.skipped_duplicate_links.len() as u64,
        candidates: plan.candidates,
        survivor: plan.survivor,
        losers: plan.losers,
        moved_links: plan.moved_links,
        skipped_duplicate_links: plan.skipped_duplicate_links,
    }
}

async fn move_host_links<C: ConnectionTrait>(
    db: &C,
    survivor_id: Uuid,
    moved_link_ids: &[Uuid],
) -> super::Result<()> {
    if moved_link_ids.is_empty() {
        return Ok(());
    }

    let links = HostSoftwareItem::find()
        .filter(host_software_item::Column::Id.is_in(moved_link_ids.to_vec()))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    if links.len() != moved_link_ids.len() {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "merge candidates changed while moving host links".to_string(),
        ));
    }

    for link in links {
        let mut active: host_software_item::ActiveModel = link.into();
        active.software_item_id = Set(survivor_id);
        active.update(db).await.context_to()?;
    }

    Ok(())
}

async fn move_link_plugin_rows<C: ConnectionTrait>(
    db: &C,
    survivor_id: Uuid,
    moved_link_ids: &[Uuid],
) -> super::Result<()> {
    if moved_link_ids.is_empty() {
        return Ok(());
    }

    let now = OffsetDateTime::now_utc();
    let plugin_rows = HostSoftwareItemPlugin::find()
        .filter(
            host_software_item_plugin::Column::HostSoftwareItemId.is_in(moved_link_ids.to_vec()),
        )
        .all(db)
        .await
        .context_to()?;

    for plugin_row in plugin_rows {
        let mut active: host_software_item_plugin::ActiveModel = plugin_row.into();
        active.software_item_id = Set(survivor_id);
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
    }

    Ok(())
}

async fn delete_duplicate_links<C: ConnectionTrait>(
    db: &C,
    survivor_id: Uuid,
    skipped_duplicate_link_ids: &[Uuid],
) -> super::Result<()> {
    if skipped_duplicate_link_ids.is_empty() {
        return Ok(());
    }

    let duplicate_links = HostSoftwareItem::find()
        .filter(host_software_item::Column::Id.is_in(skipped_duplicate_link_ids.to_vec()))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    if duplicate_links.len() != skipped_duplicate_link_ids.len() {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "merge candidates changed while reconciling duplicate host links".to_string(),
        ));
    }

    let duplicate_host_ids: Vec<Uuid> = duplicate_links.iter().map(|link| link.host_id).collect();
    let survivor_links = HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(survivor_id))
        .filter(host_software_item::Column::HostId.is_in(duplicate_host_ids.clone()))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;
    let survivor_link_by_identity: HashMap<(Uuid, Option<String>), Uuid> = survivor_links
        .iter()
        .map(|link| (merge_link_model_identity(link), link.id))
        .collect();

    let now = OffsetDateTime::now_utc();
    let mut survivor_assignment_keys: HashSet<(Uuid, String, i32)> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(survivor_id))
        .filter(host_software_item_plugin::Column::HostId.is_in(duplicate_host_ids))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|row| (row.host_software_item_id, row.role, row.ordinal))
        .collect();

    let duplicate_link_by_id: HashMap<Uuid, host_software_item::Model> = duplicate_links
        .into_iter()
        .map(|link| (link.id, link))
        .collect();
    let duplicate_plugin_rows = HostSoftwareItemPlugin::find()
        .filter(
            host_software_item_plugin::Column::HostSoftwareItemId
                .is_in(skipped_duplicate_link_ids.to_vec()),
        )
        .all(db)
        .await
        .context_to()?;

    for plugin_row in duplicate_plugin_rows {
        let duplicate_link = duplicate_link_by_id
            .get(&plugin_row.host_software_item_id)
            .ok_or_else(|| {
                report!(SoftwareItemQueryError::InvalidMergeRequest(
                    "merge candidates changed while reconciling duplicate plugin rows".to_string(),
                ))
            })?;
        let target_link_id = survivor_link_by_identity
            .get(&merge_link_model_identity(duplicate_link))
            .copied()
            .ok_or_else(|| {
                report!(SoftwareItemQueryError::InvalidMergeRequest(
                    "matching survivor link missing during duplicate reconciliation".to_string(),
                ))
            })?;
        let assignment_key = (target_link_id, plugin_row.role.clone(), plugin_row.ordinal);

        if survivor_assignment_keys.contains(&assignment_key) {
            HostSoftwareItemPlugin::delete_by_id(plugin_row.id)
                .exec(db)
                .await
                .context_to()?;
            continue;
        }

        let mut active: host_software_item_plugin::ActiveModel = plugin_row.into();
        active.software_item_id = Set(survivor_id);
        active.host_software_item_id = Set(target_link_id);
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
        survivor_assignment_keys.insert(assignment_key);
    }

    let result = HostSoftwareItem::delete_many()
        .filter(host_software_item::Column::Id.is_in(skipped_duplicate_link_ids.to_vec()))
        .exec(db)
        .await
        .context_to()?;

    if result.rows_affected != skipped_duplicate_link_ids.len() as u64 {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "merge candidates changed while removing duplicate host links".to_string(),
        ));
    }

    Ok(())
}

async fn soft_delete_losers<C: ConnectionTrait>(
    db: &C,
    tenant_id: Uuid,
    deleted_ids: &[Uuid],
) -> super::Result<()> {
    if deleted_ids.is_empty() {
        return Ok(());
    }

    let losers = SoftwareItem::find()
        .filter(software_item::Column::Id.is_in(deleted_ids.to_vec()))
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

    if losers.len() != deleted_ids.len() {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "merge candidates changed while deleting loser items".to_string(),
        ));
    }

    let now = OffsetDateTime::now_utc();
    for loser in losers {
        let mut active: software_item::ActiveModel = loser.into();
        active.deactivated_at = Set(Some(now));
        active.updated_at = Set(now);
        active.update(db).await.context_to()?;
    }

    Ok(())
}

/// Preview a manual merge of software items for a tenant.
#[tracing::instrument(skip_all, fields(tenant_id = %tenant_db.tenant_id, survivor_id = %req.survivor_id))]
pub async fn preview_merge_software_items(
    tenant_db: &TenantDb,
    req: &MergeSoftwareItemsPreviewRequest,
) -> super::Result<MergeSoftwareItemsPreviewResponse> {
    let candidate_ids = validate_candidate_ids(&req.candidate_ids, req.survivor_id)?;
    let plan = build_merge_plan(
        tenant_db.db(),
        tenant_db.tenant_id,
        &candidate_ids,
        req.survivor_id,
    )
    .await?;
    Ok(to_preview_response(plan))
}

/// Execute a manual merge of software items for a tenant.
#[tracing::instrument(skip_all, fields(tenant_id = %tenant_db.tenant_id, survivor_id = %req.survivor_id))]
pub async fn execute_merge_software_items(
    tenant_db: &TenantDb,
    req: &MergeSoftwareItemsExecuteRequest,
) -> super::Result<MergeSoftwareItemsExecuteResponse> {
    let candidate_ids = validate_candidate_ids(&req.candidate_ids, req.survivor_id)?;
    let deleted_ids = deleted_candidate_ids(&candidate_ids, req.survivor_id);

    let txn = tenant_db.db().begin().await.context_to()?;
    let plan = build_merge_plan(&txn, tenant_db.tenant_id, &candidate_ids, req.survivor_id).await?;

    let moved_link_ids: Vec<Uuid> = plan.moved_links.iter().map(|link| link.id).collect();
    let skipped_duplicate_link_ids: Vec<Uuid> = plan
        .skipped_duplicate_links
        .iter()
        .map(|link| link.id)
        .collect();

    move_host_links(&txn, req.survivor_id, &moved_link_ids).await?;
    move_link_plugin_rows(&txn, req.survivor_id, &moved_link_ids).await?;
    delete_duplicate_links(&txn, req.survivor_id, &skipped_duplicate_link_ids).await?;
    soft_delete_losers(&txn, tenant_db.tenant_id, &deleted_ids).await?;
    txn.commit().await.context_to()?;

    Ok(MergeSoftwareItemsExecuteResponse {
        survivor_id: req.survivor_id,
        deleted_ids,
        moved_link_ids,
        skipped_duplicate_link_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::{equivalent_merge_link, group_transfer_plan, preview_merge_software_items};
    use sea_orm::{ActiveModelTrait, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, software_item, tenant,
    };
    use uptrakit_web_api_types::software_items::MergeSoftwareItemLinkSummary;
    use uptrakit_web_api_types::software_items::MergeSoftwareItemsPreviewRequest;
    use uuid::Uuid;

    use crate::tenant_db::TenantDb;

    fn link(host_id: Uuid, qualifier: Option<&str>) -> MergeSoftwareItemLinkSummary {
        let qualifier_text = qualifier.unwrap_or("root");
        MergeSoftwareItemLinkSummary {
            id: Uuid::now_v7(),
            host_id,
            hostname: format!("host-{}", &host_id.to_string()[..8]),
            friendly_name: format!("Link {qualifier_text}"),
            qualifier: qualifier.map(str::to_string),
        }
    }

    #[test]
    fn equivalent_links_require_matching_qualifier_semantics() {
        let host_id = Uuid::now_v7();

        assert!(equivalent_merge_link(
            &link(host_id, None),
            &link(host_id, None)
        ));
        assert!(equivalent_merge_link(
            &link(host_id, Some("container-a")),
            &link(host_id, Some("container-a")),
        ));
        assert!(!equivalent_merge_link(
            &link(host_id, None),
            &link(host_id, Some("container-a")),
        ));
        assert!(!equivalent_merge_link(
            &link(host_id, Some("container-a")),
            &link(host_id, Some("container-b")),
        ));
    }

    #[test]
    fn preview_groups_moves_and_skips() {
        let host_a = Uuid::now_v7();
        let host_b = Uuid::now_v7();
        let host_c = Uuid::now_v7();

        let survivor_skip = link(host_a, None);
        let survivor_qualified_skip = link(host_b, Some("stable"));

        let skipped_unqualified = link(host_a, None);
        let moved_qualified = link(host_a, Some("container-a"));
        let skipped_existing_qualified = link(host_b, Some("stable"));
        let moved_new_qualified = link(host_b, Some("beta"));
        let skipped_duplicate_loser = link(host_b, Some("beta"));
        let moved_unqualified = link(host_c, None);

        let (moved_links, skipped_duplicate_links) = group_transfer_plan(
            &[survivor_skip, survivor_qualified_skip],
            vec![
                skipped_unqualified.clone(),
                moved_qualified.clone(),
                skipped_existing_qualified.clone(),
                moved_new_qualified.clone(),
                skipped_duplicate_loser.clone(),
                moved_unqualified.clone(),
            ],
        );

        assert_eq!(
            moved_links.iter().map(|link| link.id).collect::<Vec<_>>(),
            vec![
                moved_qualified.id,
                moved_new_qualified.id,
                moved_unqualified.id,
            ]
        );
        assert_eq!(
            skipped_duplicate_links
                .iter()
                .map(|link| link.id)
                .collect::<Vec<_>>(),
            vec![
                skipped_unqualified.id,
                skipped_existing_qualified.id,
                skipped_duplicate_loser.id,
            ]
        );
    }

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    async fn insert_tenant(db: &DatabaseConnection, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("Test Tenant".to_string()),
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

    async fn insert_host(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        hostname: &str,
        deactivated_at: Option<OffsetDateTime>,
    ) {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(host_id.to_string()),
            hostname: Set(hostname.to_string()),
            friendly_name: Set(hostname.to_string()),
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

    async fn insert_item(db: &DatabaseConnection, tenant_id: Uuid, item_id: Uuid, name: &str) {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_link(
        db: &DatabaseConnection,
        link_id: Uuid,
        host_id: Uuid,
        item_id: Uuid,
        deactivated_at: Option<OffsetDateTime>,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item::ActiveModel {
            id: Set(link_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(deactivated_at),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_plugin_row(
        db: &DatabaseConnection,
        plugin_row_id: Uuid,
        host_id: Uuid,
        item_id: Uuid,
        link_id: Uuid,
        plugin_type: &str,
    ) {
        let now = OffsetDateTime::now_utc();
        host_software_item_plugin::ActiveModel {
            id: Set(plugin_row_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(link_id),
            plugin_config_id: Set(None),
            plugin_type: Set(plugin_type.to_string()),
            role: Set("detect_version".to_string()),
            ordinal: Set(0),
            package_identifier: Set("pkg".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn preview_uses_only_active_links_on_active_hosts() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let survivor_id = Uuid::now_v7();
        let loser_id = Uuid::now_v7();
        insert_item(&db, tenant_id, survivor_id, "Survivor").await;
        insert_item(&db, tenant_id, loser_id, "Loser").await;

        let now = OffsetDateTime::now_utc();
        let survivor_host = Uuid::now_v7();
        let loser_active_host = Uuid::now_v7();
        let loser_deactivated_link_host = Uuid::now_v7();
        let loser_deactivated_host = Uuid::now_v7();

        insert_host(&db, tenant_id, survivor_host, "survivor-host", None).await;
        insert_host(&db, tenant_id, loser_active_host, "loser-active-host", None).await;
        insert_host(
            &db,
            tenant_id,
            loser_deactivated_link_host,
            "loser-deactivated-link-host",
            None,
        )
        .await;
        insert_host(
            &db,
            tenant_id,
            loser_deactivated_host,
            "loser-deactivated-host",
            Some(now),
        )
        .await;

        insert_link(&db, Uuid::now_v7(), survivor_host, survivor_id, None).await;
        insert_link(&db, Uuid::now_v7(), survivor_host, survivor_id, Some(now)).await;
        insert_link(&db, Uuid::now_v7(), loser_active_host, loser_id, None).await;
        insert_link(
            &db,
            Uuid::now_v7(),
            loser_deactivated_link_host,
            loser_id,
            Some(now),
        )
        .await;
        insert_link(&db, Uuid::now_v7(), loser_deactivated_host, loser_id, None).await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let preview = preview_merge_software_items(
            &tenant_db,
            &MergeSoftwareItemsPreviewRequest {
                candidate_ids: vec![survivor_id, loser_id],
                survivor_id,
                seed_item_id: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(preview.candidate_count, 2);
        assert_eq!(preview.moved_link_count, 1);
        assert_eq!(preview.skipped_duplicate_link_count, 0);
        assert_eq!(preview.moved_links.len(), 1);
        assert_eq!(preview.moved_links[0].host_id, loser_active_host);

        let survivor = preview
            .candidates
            .iter()
            .find(|item| item.id == survivor_id)
            .unwrap();
        let loser = preview
            .candidates
            .iter()
            .find(|item| item.id == loser_id)
            .unwrap();

        assert_eq!(survivor.host_count, 1);
        assert_eq!(loser.host_count, 1);
    }

    #[tokio::test]
    async fn preview_plugin_summaries_ignore_inactive_only_assignments() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;

        let survivor_id = Uuid::now_v7();
        let loser_id = Uuid::now_v7();
        insert_item(&db, tenant_id, survivor_id, "Survivor").await;
        insert_item(&db, tenant_id, loser_id, "Loser").await;

        let now = OffsetDateTime::now_utc();
        let survivor_host = Uuid::now_v7();
        let loser_active_host = Uuid::now_v7();
        let loser_inactive_link_host = Uuid::now_v7();
        let loser_inactive_host = Uuid::now_v7();

        insert_host(&db, tenant_id, survivor_host, "survivor-host", None).await;
        insert_host(&db, tenant_id, loser_active_host, "loser-active-host", None).await;
        insert_host(
            &db,
            tenant_id,
            loser_inactive_link_host,
            "loser-inactive-link-host",
            None,
        )
        .await;
        insert_host(
            &db,
            tenant_id,
            loser_inactive_host,
            "loser-inactive-host",
            Some(now),
        )
        .await;

        let survivor_link = Uuid::now_v7();
        let loser_active_link = Uuid::now_v7();
        let loser_inactive_link = Uuid::now_v7();
        let loser_inactive_host_link = Uuid::now_v7();

        insert_link(&db, survivor_link, survivor_host, survivor_id, None).await;
        insert_link(&db, loser_active_link, loser_active_host, loser_id, None).await;
        insert_link(
            &db,
            loser_inactive_link,
            loser_inactive_link_host,
            loser_id,
            Some(now),
        )
        .await;
        insert_link(
            &db,
            loser_inactive_host_link,
            loser_inactive_host,
            loser_id,
            None,
        )
        .await;

        insert_plugin_row(
            &db,
            Uuid::now_v7(),
            survivor_host,
            survivor_id,
            survivor_link,
            "releases_github",
        )
        .await;
        insert_plugin_row(
            &db,
            Uuid::now_v7(),
            loser_active_host,
            loser_id,
            loser_active_link,
            "package_manager_apt",
        )
        .await;
        insert_plugin_row(
            &db,
            Uuid::now_v7(),
            loser_inactive_link_host,
            loser_id,
            loser_inactive_link,
            "releases_gitlab",
        )
        .await;
        insert_plugin_row(
            &db,
            Uuid::now_v7(),
            loser_inactive_host,
            loser_id,
            loser_inactive_host_link,
            "package_manager_dnf",
        )
        .await;

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let preview = preview_merge_software_items(
            &tenant_db,
            &MergeSoftwareItemsPreviewRequest {
                candidate_ids: vec![survivor_id, loser_id],
                survivor_id,
                seed_item_id: None,
            },
        )
        .await
        .unwrap();

        let survivor = preview
            .candidates
            .iter()
            .find(|item| item.id == survivor_id)
            .unwrap();
        let loser = preview
            .candidates
            .iter()
            .find(|item| item.id == loser_id)
            .unwrap();

        assert_eq!(survivor.plugins, vec!["releases_github"]);
        assert_eq!(loser.plugins, vec!["package_manager_apt"]);
    }
}
