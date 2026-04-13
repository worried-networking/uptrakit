//! Merge preview query logic for software items.

use rootcause::prelude::*;
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QueryOrder, QuerySelect,
    RelationTrait,
};
use std::collections::{HashMap, HashSet};
use uptrakit_shared_db::entity::{host, host_software_item, prelude::*, software_item};
use uptrakit_web_api_types::software_items::{
    MergeSoftwareItemLinkSummary, MergeSoftwareItemSummary, MergeSoftwareItemsPreviewRequest,
    MergeSoftwareItemsPreviewResponse,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

use super::{SoftwareItemQueryError, count_linked_hosts, load_plugins};

#[derive(Debug, FromQueryResult)]
struct MergeLinkRow {
    software_item_id: Uuid,
    id: Uuid,
    host_id: Uuid,
    qualifier: Option<String>,
    hostname: String,
    friendly_name: String,
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

fn validate_preview_request(req: &MergeSoftwareItemsPreviewRequest) -> super::Result<Vec<Uuid>> {
    let candidate_ids = normalize_candidate_ids(&req.candidate_ids);

    if candidate_ids.len() < 2 {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "at least two distinct candidate_ids are required".to_string(),
        ));
    }

    if !candidate_ids.contains(&req.survivor_id) {
        bail!(SoftwareItemQueryError::InvalidMergeRequest(
            "survivor_id must be included in candidate_ids".to_string(),
        ));
    }

    Ok(candidate_ids)
}

async fn load_active_candidates(
    tenant_db: &TenantDb,
    candidate_ids: &[Uuid],
) -> super::Result<Vec<software_item::Model>> {
    let items = SoftwareItem::find()
        .filter(software_item::Column::Id.is_in(candidate_ids.to_vec()))
        .filter(software_item::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
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
    db: &sea_orm::DatabaseConnection,
    item: &software_item::Model,
) -> super::Result<MergeSoftwareItemSummary> {
    let mut plugins = load_plugins(db, item.id).await;
    plugins.sort();

    Ok(MergeSoftwareItemSummary {
        id: item.id,
        name: item.name.clone(),
        host_count: count_linked_hosts(db, item.id).await?,
        plugins,
    })
}

async fn load_candidate_links(
    db: &sea_orm::DatabaseConnection,
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

/// Preview a manual merge of software items for a tenant.
#[tracing::instrument(skip_all, fields(tenant_id = %tenant_db.tenant_id, survivor_id = %req.survivor_id))]
pub async fn preview_merge_software_items(
    tenant_db: &TenantDb,
    req: &MergeSoftwareItemsPreviewRequest,
) -> super::Result<MergeSoftwareItemsPreviewResponse> {
    let candidate_ids = validate_preview_request(req)?;
    let candidate_items = load_active_candidates(tenant_db, &candidate_ids).await?;
    let item_by_id: HashMap<Uuid, software_item::Model> = candidate_items
        .into_iter()
        .map(|item| (item.id, item))
        .collect();

    let mut candidates = Vec::with_capacity(candidate_ids.len());
    for candidate_id in &candidate_ids {
        let item = item_by_id.get(candidate_id).ok_or_else(|| {
            report!(SoftwareItemQueryError::InvalidMergeRequest(
                "candidate items must all exist, belong to the tenant, and be active".to_string(),
            ))
        })?;
        candidates.push(build_item_summary(tenant_db.db(), item).await?);
    }

    let survivor = candidates
        .iter()
        .find(|candidate| candidate.id == req.survivor_id)
        .cloned()
        .ok_or_else(|| {
            report!(SoftwareItemQueryError::InvalidMergeRequest(
                "survivor_id must be included in candidate_ids".to_string(),
            ))
        })?;

    let losers: Vec<MergeSoftwareItemSummary> = candidates
        .iter()
        .filter(|candidate| candidate.id != req.survivor_id)
        .cloned()
        .collect();

    let candidate_index: HashMap<Uuid, usize> = candidate_ids
        .iter()
        .enumerate()
        .map(|(index, candidate_id)| (*candidate_id, index))
        .collect();

    let mut link_rows =
        load_candidate_links(tenant_db.db(), tenant_db.tenant_id, &candidate_ids).await?;
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
        if software_item_id == req.survivor_id {
            survivor_links.push(summary);
        } else {
            loser_links.push(summary);
        }
    }

    let (moved_links, skipped_duplicate_links) = group_transfer_plan(&survivor_links, loser_links);

    Ok(MergeSoftwareItemsPreviewResponse {
        candidate_count: candidates.len() as u64,
        loser_count: losers.len() as u64,
        moved_link_count: moved_links.len() as u64,
        skipped_duplicate_link_count: skipped_duplicate_links.len() as u64,
        candidates,
        survivor,
        losers,
        moved_links,
        skipped_duplicate_links,
    })
}

#[cfg(test)]
mod tests {
    use super::{equivalent_merge_link, group_transfer_plan};
    use uptrakit_web_api_types::software_items::MergeSoftwareItemLinkSummary;
    use uuid::Uuid;

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
}
