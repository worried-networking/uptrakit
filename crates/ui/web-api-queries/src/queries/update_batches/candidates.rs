//! Candidate discovery for batch updates.
//!
//! Provides functions for finding outdated software items on a host and
//! outdated hosts for a software item.

use rootcause::prelude::*;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter};
use std::collections::{HashMap, HashSet};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, prelude::*, software_item,
};
use uuid::Uuid;

use super::Result;
use crate::queries::update_dispatch::TriggerUpdateError;

/// A software item that is outdated on a particular host.
pub struct BatchUpdateCandidate {
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub host_id: Uuid,
    pub host_name: String,
    pub installed_version: String,
    pub latest_version: String,
    pub update_category: String,
}

/// Find all outdated items for a host, optionally filtered by update category.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn find_outdated_items_for_host(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    category_filter: Option<&str>,
    exclude_item_ids: Option<&[Uuid]>,
) -> Result<Vec<BatchUpdateCandidate>> {
    // Verify host exists and belongs to tenant
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotFound))?;

    // Load all host_software_items for this host that have both versions set and differ
    let mut query = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::InstalledVersion.is_not_null())
        .filter(host_software_item::Column::LatestVersion.is_not_null());

    if let Some(cat) = category_filter {
        query = query.filter(host_software_item::Column::UpdateCategory.eq(cat));
    }

    let links = query.all(db).await.context_to()?;

    if links.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load active software items for all links
    let link_ids: Vec<Uuid> = links.iter().map(|l| l.software_item_id).collect();
    let items: HashMap<Uuid, software_item::Model> = SoftwareItem::find()
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::Id.is_in(link_ids.clone()))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();

    // Batch-load execute_update plugin assignments for this host
    let execute_plugin_item_ids: HashSet<Uuid> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(link_ids))
        .filter(host_software_item_plugin::Column::Role.eq("execute_update"))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|p| p.software_item_id)
        .collect();

    // Filter to only outdated items with an execute_update plugin
    let mut candidates = Vec::new();
    for link in links {
        let installed = match link.installed_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        let latest = match link.latest_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        if installed == latest {
            continue;
        }

        // Exclude if requested
        if let Some(excludes) = exclude_item_ids
            && excludes.contains(&link.software_item_id)
        {
            continue;
        }

        // Skip inactive or missing software items
        let Some(item) = items.get(&link.software_item_id) else {
            continue;
        };

        // Skip items without an execute_update plugin
        if !execute_plugin_item_ids.contains(&link.software_item_id) {
            continue;
        }

        candidates.push(BatchUpdateCandidate {
            software_item_id: link.software_item_id,
            software_item_name: item.name.clone(),
            host_id,
            host_name: host_record.friendly_name.clone(),
            installed_version: installed.clone(),
            latest_version: latest.clone(),
            update_category: link.update_category.clone(),
        });
    }

    Ok(candidates)
}

/// Find all hosts where a software item is outdated.
#[tracing::instrument(skip_all, fields(%tenant_id))]
pub async fn find_outdated_hosts_for_item(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    item_id: Uuid,
    host_ids: Option<&[Uuid]>,
) -> Result<Vec<BatchUpdateCandidate>> {
    // Verify software item exists and is active
    let item = SoftwareItem::find_by_id(item_id)
        .filter(software_item::Column::TenantId.eq(tenant_id))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::SoftwareItemNotFound))?;

    // Load all host_software_items for this software item
    let mut query = HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item::Column::InstalledVersion.is_not_null())
        .filter(host_software_item::Column::LatestVersion.is_not_null());

    if let Some(ids) = host_ids {
        query = query.filter(host_software_item::Column::HostId.is_in(ids.to_vec()));
    }

    let links = query.all(db).await.context_to()?;

    if links.is_empty() {
        return Ok(vec![]);
    }

    // Batch-load host records
    let host_record_ids: Vec<Uuid> = links.iter().map(|l| l.host_id).collect();
    let hosts: HashMap<Uuid, host::Model> = Host::find()
        .filter(host::Column::Id.is_in(host_record_ids.clone()))
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    // Batch-load execute_update plugin assignments for this item across all hosts
    let execute_plugin_host_ids: HashSet<Uuid> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::HostId.is_in(host_record_ids))
        .filter(host_software_item_plugin::Column::Role.eq("execute_update"))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|p| p.host_id)
        .collect();

    let mut candidates = Vec::new();
    for link in links {
        let installed = match link.installed_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        let latest = match link.latest_version.as_ref() {
            Some(v) => v,
            None => continue,
        };
        if installed == latest {
            continue;
        }

        let Some(host_record) = hosts.get(&link.host_id) else {
            continue;
        };

        // Skip hosts without an execute_update plugin for this item
        if !execute_plugin_host_ids.contains(&link.host_id) {
            continue;
        }

        candidates.push(BatchUpdateCandidate {
            software_item_id: item_id,
            software_item_name: item.name.clone(),
            host_id: link.host_id,
            host_name: host_record.friendly_name.clone(),
            installed_version: installed.clone(),
            latest_version: latest.clone(),
            update_category: link.update_category.clone(),
        });
    }

    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::queries::update_batches::tests::{insert_base_fixture, setup_db};
    use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        host_software_item, host_software_item_plugin, plugin_config, software_item,
    };
    use uuid::Uuid;

    // -- find_outdated_items_for_host --

    #[tokio::test]
    async fn find_outdated_items_empty_when_versions_match() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(f.host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(f.item_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: host_software_item::ActiveModel = hsi.into();
        active.installed_version = Set(Some("1.1.0".to_string())); // same as latest
        active.update(&db).await.unwrap();

        let candidates = find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, None)
            .await
            .unwrap();
        assert!(
            candidates.is_empty(),
            "expected empty; got {}",
            candidates.len()
        );
    }

    #[tokio::test]
    async fn find_outdated_items_returns_candidate_when_outdated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let candidates = find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, None)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].software_item_id, f.item_id);
        assert_eq!(candidates[0].installed_version, "1.0.0");
        assert_eq!(candidates[0].latest_version, "1.1.0");
    }

    #[tokio::test]
    async fn find_outdated_items_filters_by_category() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await; // item_id has category "security"
        let now = OffsetDateTime::now_utc();

        // Add a second software item and link it to the same host with category "feature".
        let item2_id = Uuid::now_v7();
        let pc2_id = Uuid::now_v7();
        software_item::ActiveModel {
            id: Set(item2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-app-2".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        plugin_config::ActiveModel {
            id: Set(pc2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-plugin-2".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        let hsi2_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi2_id),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(pc2_id)),
            package_identifier: Set(Some("test-app-2".to_string())),
            installed_version: Set(Some("2.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(Some("2.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("feature".to_string()),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            host_software_item_id: Set(hsi2_id),
            plugin_config_id: Set(Some(pc2_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo2".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .unwrap();

        // Filter by "security" -- should return only the first item.
        let candidates =
            find_outdated_items_for_host(&db, f.tenant_id, f.host_id, Some("security"), None)
                .await
                .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].software_item_id, f.item_id);

        // Filter by "feature" -- should return only the second item.
        let candidates_feature =
            find_outdated_items_for_host(&db, f.tenant_id, f.host_id, Some("feature"), None)
                .await
                .unwrap();
        assert_eq!(candidates_feature.len(), 1);
        assert_eq!(candidates_feature[0].software_item_id, item2_id);
    }

    #[tokio::test]
    async fn find_outdated_items_excludes_specified_ids() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let candidates =
            find_outdated_items_for_host(&db, f.tenant_id, f.host_id, None, Some(&[f.item_id]))
                .await
                .unwrap();
        assert!(
            candidates.is_empty(),
            "excluded item must not appear in results"
        );
    }

    // -- find_outdated_hosts_for_item --

    #[tokio::test]
    async fn find_outdated_hosts_empty_when_up_to_date() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(f.host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(f.item_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: host_software_item::ActiveModel = hsi.into();
        active.installed_version = Set(Some("1.1.0".to_string())); // same as latest
        active.update(&db).await.unwrap();

        let candidates = find_outdated_hosts_for_item(&db, f.tenant_id, f.item_id, None)
            .await
            .unwrap();
        assert!(
            candidates.is_empty(),
            "expected empty; got {}",
            candidates.len()
        );
    }

    #[tokio::test]
    async fn find_outdated_hosts_returns_candidate_when_outdated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;

        let candidates = find_outdated_hosts_for_item(&db, f.tenant_id, f.item_id, None)
            .await
            .unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].host_id, f.host_id);
        assert_eq!(candidates[0].software_item_id, f.item_id);
        assert_eq!(candidates[0].installed_version, "1.0.0");
        assert_eq!(candidates[0].latest_version, "1.1.0");
    }
}
