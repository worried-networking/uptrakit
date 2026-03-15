//! Processing incoming discovery results: creating pending software items
//! and upserting host-software-item links.

use super::default_configs::find_or_create_default_plugin_config;
use super::{AutodiscoveryError, Result};
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::collections::HashSet;
use time::OffsetDateTime;
use uptrakit_internal_wire::{DiscoveryPluginResult, DiscoveryTarget};
use uptrakit_shared_db::entity::{
    host_software_item, host_software_item_plugin, prelude::*, software_ignore, software_item,
};
use uptrakit_shared_db::is_unique_constraint_violation;
use uuid::Uuid;

/// Grouped arguments for a discovered item's identity fields.
pub(super) struct DiscoveredItemInfo<'a> {
    pub package_identifier: &'a str,
    pub name: &'a str,
    pub installed_version: &'a str,
    pub featured: bool,
    /// Qualifier for the `host_software_items` row (e.g. Docker container name).
    /// `None` = unqualified (default for non-Docker items).
    pub qualifier: Option<&'a str>,
    /// Package identifier stored in `host_software_item_plugins` for per-container operations.
    /// `None` = use `package_identifier` (existing behavior for non-Docker items).
    pub plugin_package_identifier: Option<&'a str>,
}

impl<'a> DiscoveredItemInfo<'a> {
    /// Returns the package identifier to store in `host_software_item_plugin.package_identifier`.
    ///
    /// For Docker items, this is `plugin_package_identifier` (e.g. `nginx:latest#web-server`).
    /// For all other items, this falls back to `package_identifier`.
    fn effective_plugin_pkg_id(&self) -> &str {
        self.plugin_package_identifier
            .unwrap_or(self.package_identifier)
    }
}

/// Process a single plugin's discovery results.
///
/// Routes each item to the correct processing path based on its `targets`
/// and the result's `plugin_config_id`.
pub(super) async fn process_plugin_result(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    result: &DiscoveryPluginResult,
    ignore_set: &HashSet<String>,
) -> Result<()> {
    for item in &result.discoveries {
        // Check the tenant-wide name-based ignore list before any processing.
        if ignore_set.contains(&item.name) {
            tracing::debug!(
                name = %item.name,
                package_identifier = %item.package_identifier,
                "skipping ignored autodiscovery item (name-based ignore)"
            );
            continue;
        }

        let item_info = DiscoveredItemInfo {
            package_identifier: &item.package_identifier,
            name: &item.name,
            installed_version: &item.installed_version,
            featured: item.featured,
            qualifier: item.qualifier.as_deref(),
            plugin_package_identifier: item.plugin_package_identifier.as_deref(),
        };
        if !item.targets.is_empty() {
            process_targets_discovery(db, tenant_id, host_id, &item_info, &item.targets, now)
                .await?;
        } else if let Some(existing_pc_id) = result.plugin_config_id {
            process_one_discovery(
                db,
                tenant_id,
                host_id,
                existing_pc_id,
                &result.plugin_type.to_string(),
                item_info,
                now,
            )
            .await?;
        } else {
            tracing::warn!(
                plugin_type = %result.plugin_type,
                package_identifier = %item.package_identifier,
                "discovery item has no targets and no plugin_config_id; skipping"
            );
        }
    }

    Ok(())
}

/// Pre-load the tenant-wide autodiscovery ignore set.
///
/// Returns a `HashSet` of software item display names that should be skipped.
/// Bounded by the number of names explicitly ignored by the user (typically small).
pub(super) async fn load_ignore_set(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
) -> Result<HashSet<String>> {
    let rules = SoftwareIgnore::find()
        .filter(software_ignore::Column::TenantId.eq(tenant_id))
        .all(db)
        .await
        .context_to()?;
    Ok(rules.into_iter().map(|r| r.name).collect())
}

/// Process a discovered item that carries explicit `DiscoveryTarget` values.
///
/// For each target:
/// 1. For non-package-manager types: find-or-create a plugin config matching
///    the target's type and JSON config. For package managers: skip config
///    creation (they use `plugin_type_settings` at the tenant level).
/// 2. Check the ignore list.
/// 3. Upsert or create the software item and host link.
/// 4. Create role assignments per the target's `roles` list.
async fn process_targets_discovery(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    item: &DiscoveredItemInfo<'_>,
    targets: &[DiscoveryTarget],
    now: OffsetDateTime,
) -> Result<()> {
    for target in targets {
        let target_plugin_type_str = target.plugin_type.to_string();

        // Use the target's package_identifier override, or fall back to the item's.
        let pkg_id = target
            .package_identifier
            .as_deref()
            .unwrap_or(item.package_identifier);

        let execution_site = target.execution_site.as_deref().unwrap_or("auto");

        // Package manager types use plugin_type_settings and do not need
        // per-config rows. Credential-bearing types still get plugin_configs.
        let pc_id = if target.plugin_type.is_package_manager() {
            None
        } else {
            Some(
                find_or_create_default_plugin_config(
                    db,
                    tenant_id,
                    &target_plugin_type_str,
                    &target.plugin_config,
                    &target.plugin_config_name,
                )
                .await?,
            )
        };

        // Build target-specific item info (may override package_identifier).
        let target_item = DiscoveredItemInfo {
            package_identifier: pkg_id,
            name: item.name,
            installed_version: item.installed_version,
            featured: item.featured,
            qualifier: item.qualifier,
            plugin_package_identifier: item.plugin_package_identifier,
        };

        // Find-or-create the software item and host link.
        let Some((software_item_id, hsi_id)) = find_or_create_software_item(
            db,
            tenant_id,
            host_id,
            pc_id,
            &target_plugin_type_str,
            &target_item,
            now,
        )
        .await?
        else {
            continue;
        };

        // Create role assignments from the target's role list.
        for role in &target.roles {
            let plugin_link = host_software_item_plugin::ActiveModel {
                id: Set(Uuid::now_v7()),
                host_id: Set(host_id),
                software_item_id: Set(software_item_id),
                host_software_item_id: Set(hsi_id),
                plugin_config_id: Set(pc_id),
                plugin_type: Set(target_plugin_type_str.clone()),
                role: Set(role.as_str().to_string()),
                ordinal: Set(0),
                package_identifier: Set(target_item.effective_plugin_pkg_id().to_string()),
                config: Set(target.config_override.clone()),
                execution_site: Set(execution_site.to_owned()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            if let Err(e) = HostSoftwareItemPlugin::insert(plugin_link).exec(db).await
                && !is_unique_constraint_violation(&e)
            {
                return Err(report!(AutodiscoveryError::Db(e)));
            }
        }
    }

    Ok(())
}

/// Find-or-create a software item + host link.
///
/// Returns `Some((software_item_id, hsi_id))` when a new link was created (caller must then
/// create role assignments), or `None` if the existing link was updated in-place.
///
/// `plugin_config_id` is `None` for package manager types (which use
/// `plugin_type_settings` instead of per-config rows). In that case, Phase 1/2
/// lookups match on `(plugin_type, package_identifier)` with a NULL
/// `plugin_config_id` rather than `(plugin_config_id, package_identifier)`.
///
/// Three-phase lookup:
///
/// 1. If this host already has a `host_software_item_plugin` row for
///    the matching identity key, update `installed_version` in place
///    **if** the linked `software_item` is still active. If the item has been
///    discarded (soft-deleted), the orphaned link is removed and the function falls
///    through to phases 2/3 so a fresh pending item is created.
/// 2. If *any other* host in the tenant has the same assignment backed by an active
///    software item, reuse it and insert a new `host_software_item` link for this host.
/// 3. Otherwise create a new pending `software_item` and `host_software_item`.
///    If the insert hits a `(tenant_id, name)` unique-constraint violation (e.g.
///    a second `DiscoveryTarget` for the same `DiscoveredSoftware` item races
///    through Phase 3 first), fall back to a `(tenant_id, name)` lookup so both
///    targets end up sharing the same `software_item` row.
async fn find_or_create_software_item(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_config_id: Option<Uuid>,
    plugin_type_str: &str,
    item: &DiscoveredItemInfo<'_>,
    now: OffsetDateTime,
) -> Result<Option<(Uuid, Uuid)>> {
    let package_identifier = item.package_identifier;
    let name = item.name;
    let installed_version = item.installed_version;
    // Use the container-qualified identifier for plugin link lookup so that
    // two containers on the same host using the same image are tracked separately.
    let lookup_pkg_id = item.effective_plugin_pkg_id();

    // Phase 1: Check if this specific host already tracks this package.
    // For package managers (plugin_config_id = None), match on
    // (plugin_type, package_identifier) with NULL plugin_config_id;
    // for credential-bearing types, match on (plugin_config_id, package_identifier).
    let mut phase1_query = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::PackageIdentifier.eq(lookup_pkg_id));
    phase1_query = match plugin_config_id {
        Some(pc_id) => {
            phase1_query.filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
        }
        None => phase1_query
            .filter(host_software_item_plugin::Column::PluginConfigId.is_null())
            .filter(host_software_item_plugin::Column::PluginType.eq(plugin_type_str)),
    };
    let existing_plugin_link = phase1_query.one(db).await.context_to()?;

    if let Some(plugin_link) = existing_plugin_link {
        let linked_item = SoftwareItem::find()
            .filter(software_item::Column::Id.eq(plugin_link.software_item_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(db)
            .await
            .context_to()?;

        if let Some(linked_item) = linked_item {
            // Only update installed_version for non-featured (not yet approved) items.
            // Featured items get their version from the DetectVersion scheduled
            // task using the user's assigned plugin config.
            if !linked_item.featured {
                let mut hsi_query = HostSoftwareItem::find()
                    .filter(host_software_item::Column::HostId.eq(host_id))
                    .filter(
                        host_software_item::Column::SoftwareItemId.eq(plugin_link.software_item_id),
                    );
                hsi_query = match item.qualifier {
                    Some(q) => hsi_query.filter(host_software_item::Column::Qualifier.eq(q)),
                    None => hsi_query.filter(host_software_item::Column::Qualifier.is_null()),
                };
                if let Some(hsi) = hsi_query.one(db).await.context_to()? {
                    let mut active: host_software_item::ActiveModel = hsi.into();
                    active.installed_version = Set(Some(installed_version.to_string()));
                    active.installed_version_detected_at = Set(Some(now));
                    active.update(db).await.context_to()?;
                }
            } else {
                tracing::debug!(
                    software_item_id = %linked_item.id,
                    featured = linked_item.featured,
                    %package_identifier,
                    "skipping installed_version update for featured software item"
                );
            }
            return Ok(None);
        }

        // The linked software item was discarded; remove the orphaned link.
        tracing::debug!(
            plugin_config_id = ?plugin_config_id,
            plugin_type = %plugin_type_str,
            package_identifier = %package_identifier,
            "removing orphaned host link for discarded software item; will re-discover"
        );
        let mut orphan_query = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(plugin_link.software_item_id));
        orphan_query = match item.qualifier {
            Some(q) => orphan_query.filter(host_software_item::Column::Qualifier.eq(q)),
            None => orphan_query.filter(host_software_item::Column::Qualifier.is_null()),
        };
        if let Some(hsi) = orphan_query.one(db).await.context_to()? {
            // Explicitly remove plugin link rows before deleting the HSI row.
            // SQLite disables FK cascade enforcement inside the migration
            // transaction (PRAGMA foreign_keys is a no-op inside BEGIN), so
            // we must delete child rows manually to maintain integrity.
            HostSoftwareItemPlugin::delete_many()
                .filter(host_software_item_plugin::Column::HostSoftwareItemId.eq(hsi.id))
                .exec(db)
                .await
                .context_to()?;
            let hsi_active: host_software_item::ActiveModel = hsi.into();
            hsi_active.delete(db).await.context_to()?;
        }
        // Fall through to phases 2/3.
    }

    // Phase 2: Check if any other host in this tenant already has
    // a matching assignment for this package.
    let mut phase2_query = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::PackageIdentifier.eq(lookup_pkg_id));
    phase2_query = match plugin_config_id {
        Some(pc_id) => {
            phase2_query.filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
        }
        None => phase2_query
            .filter(host_software_item_plugin::Column::PluginConfigId.is_null())
            .filter(host_software_item_plugin::Column::PluginType.eq(plugin_type_str)),
    };
    let candidate_links: Vec<Uuid> = phase2_query
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|l| l.software_item_id)
        .collect();

    let existing_item = if candidate_links.is_empty() {
        None
    } else {
        SoftwareItem::find()
            .filter(software_item::Column::Id.is_in(candidate_links))
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(db)
            .await
            .context_to()?
    };

    let software_item_id = if let Some(existing) = existing_item {
        existing.id
    } else {
        // Phase 3: Create a new pending software item.
        let new_id = Uuid::now_v7();
        let new_item = software_item::ActiveModel {
            id: Set(new_id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(item.featured),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        match SoftwareItem::insert(new_item).exec(db).await {
            Ok(_) => {
                tracing::debug!(
                    package_identifier = %package_identifier,
                    "created pending software item from discovery"
                );
                new_id
            }
            Err(e) if is_unique_constraint_violation(&e) => {
                // Another target for the same DiscoveredSoftware (or a concurrent
                // request) already created a software_item with this name.
                tracing::debug!(
                    package_identifier = %package_identifier,
                    name = %name,
                    "software_item name collision on insert; reusing existing item"
                );
                SoftwareItem::find()
                    .filter(software_item::Column::TenantId.eq(tenant_id))
                    .filter(software_item::Column::Name.eq(name))
                    .filter(software_item::Column::DeactivatedAt.is_null())
                    .one(db)
                    .await
                    .context_to()?
                    .ok_or_else(|| {
                        report!(AutodiscoveryError::Db(sea_orm::DbErr::RecordNotFound(
                            format!("software_item with name '{name}' not found after collision")
                        )))
                    })?
                    .id
            }
            Err(e) => return Err(report!(AutodiscoveryError::Db(e))),
        }
    };

    // Insert host_software_item link.
    let hsi_id = Uuid::now_v7();
    let link = host_software_item::ActiveModel {
        id: Set(hsi_id),
        host_id: Set(host_id),
        software_item_id: Set(software_item_id),
        qualifier: Set(item.qualifier.map(|s| s.to_string())),
        plugin_config_id: Set(plugin_config_id),
        package_identifier: Set(Some(item.effective_plugin_pkg_id().to_string())),
        installed_version: Set(Some(installed_version.to_string())),
        installed_version_detected_at: Set(Some(now)),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        deactivated_at: Set(None),
    };
    match HostSoftwareItem::insert(link).exec(db).await {
        Ok(_) => {}
        Err(e) if is_unique_constraint_violation(&e) => {
            // Either a concurrent task or a second DiscoveryTarget for the same software item
            // already inserted this (host_id, software_item_id, qualifier) row.
            // Look up the existing row's surrogate id so the caller can still create plugin assignments.
            let mut existing_query = HostSoftwareItem::find()
                .filter(host_software_item::Column::HostId.eq(host_id))
                .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id));
            existing_query = match item.qualifier {
                Some(q) => existing_query.filter(host_software_item::Column::Qualifier.eq(q)),
                None => existing_query.filter(host_software_item::Column::Qualifier.is_null()),
            };
            let existing_hsi_id = existing_query.one(db).await.context_to()?.map(|hsi| hsi.id);
            if let Some(existing_id) = existing_hsi_id {
                return Ok(Some((software_item_id, existing_id)));
            }
            // Row disappeared between insert failure and lookup -- return None to skip.
            return Ok(None);
        }
        Err(e) => return Err(report!(AutodiscoveryError::Db(e))),
    }

    Ok(Some((software_item_id, hsi_id)))
}

/// Process a single discovered software item using the config-ID path.
///
/// Used when items have no targets and the enclosing result has a pre-existing
/// `plugin_config_id`. Creates all three standard role assignments.
async fn process_one_discovery(
    db: &sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    plugin_config_id: Uuid,
    plugin_type_str: &str,
    args: DiscoveredItemInfo<'_>,
    now: OffsetDateTime,
) -> Result<()> {
    let Some((software_item_id, hsi_id)) = find_or_create_software_item(
        db,
        tenant_id,
        host_id,
        Some(plugin_config_id),
        plugin_type_str,
        &args,
        now,
    )
    .await?
    else {
        return Ok(());
    };

    // Create role plugin assignments for all three standard roles.
    for role in ["detect_version", "fetch_releases", "execute_update"] {
        let plugin_link = host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(hsi_id),
            plugin_config_id: Set(Some(plugin_config_id)),
            plugin_type: Set(plugin_type_str.to_string()),
            role: Set(role.to_string()),
            ordinal: Set(0),
            package_identifier: Set(args.effective_plugin_pkg_id().to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        };
        if let Err(e) = HostSoftwareItemPlugin::insert(plugin_link).exec(db).await
            && !is_unique_constraint_violation(&e)
        {
            return Err(report!(AutodiscoveryError::Db(e)));
        }
    }

    Ok(())
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use crate::queries::autodiscovery::tests_common::{
        all_roles, insert_host, insert_host_link, insert_plugin_config, insert_software_item,
        insert_tenant, phs_result_no_targets, phs_result_with_apt_target,
        phs_result_with_github_target, phs_result_with_two_targets, setup_db,
    };
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter};
    use uptrakit_internal_wire::{
        DiscoveredSoftware as WireDiscoveredSoftware, DiscoveryPluginResult, DiscoveryTarget,
        PluginType,
    };
    use uptrakit_shared_db::entity::plugin_config;

    /// When `find_or_create_software_item` encounters a host link pointing to a
    /// deactivated software item, it must delete the orphaned link and create a
    /// fresh pending item.
    #[tokio::test]
    async fn process_one_discovery_orphaned_link_creates_new_pending() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let old_item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, old_item_id, tenant_id, "curl", Some(now)).await;
        insert_host_link(&db, host_id, old_item_id, pc_id, "curl").await;

        let args = DiscoveredItemInfo {
            package_identifier: "curl",
            name: "curl",
            installed_version: "8.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
        };

        process_one_discovery(
            &db,
            tenant_id,
            host_id,
            pc_id,
            "package_manager_homebrew",
            args,
            now,
        )
        .await
        .expect("process_one_discovery");

        let orphan_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(old_item_id))
            .count(&db)
            .await
            .expect("orphan count");
        assert_eq!(orphan_count, 0, "orphaned host link must be deleted");

        let active_items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("active items");
        assert_eq!(
            active_items.len(),
            1,
            "expected exactly one new pending item"
        );
        assert!(
            !active_items[0].featured,
            "new discovery items should preserve plugin-provided featured=false"
        );

        let new_link_count = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("curl"))
            .count(&db)
            .await
            .expect("new link count");
        assert_eq!(
            new_link_count, 3,
            "expected plugin link rows for all three roles"
        );
    }

    /// `process_one_discovery` must update `installed_version` in place when the
    /// existing host link points to an active software item.
    #[tokio::test]
    async fn process_one_discovery_active_link_updates_version() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, item_id, tenant_id, "wget", None).await;
        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
        };

        process_one_discovery(
            &db,
            tenant_id,
            host_id,
            pc_id,
            "package_manager_homebrew",
            args,
            now,
        )
        .await
        .expect("process_one_discovery");

        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("items");
        assert_eq!(items.len(), 1, "no new item should be created");
        assert_eq!(items[0].id, item_id, "the original item must be retained");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("link query")
            .expect("link must exist");
        assert_eq!(
            link.installed_version.as_deref(),
            Some("2.0.0"),
            "installed_version must be updated to the new value"
        );
    }

    /// When a software item is featured (approved), periodic re-discovery
    /// must NOT overwrite `installed_version`. The proper `DetectVersion`
    /// scheduled task handles version detection for featured items using
    /// the user's assigned plugin config.
    #[tokio::test]
    async fn process_one_discovery_featured_item_skips_version_update() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        // Insert a featured software item (user has approved it).
        let model = software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("wget".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        SoftwareItem::insert(model)
            .exec(&db)
            .await
            .expect("insert featured software_item");

        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
        };

        process_one_discovery(
            &db,
            tenant_id,
            host_id,
            pc_id,
            "package_manager_homebrew",
            args,
            now,
        )
        .await
        .expect("process_one_discovery");

        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("items");
        assert_eq!(items.len(), 1, "no new item should be created");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("link query")
            .expect("link must exist");
        assert_eq!(
            link.installed_version.as_deref(),
            Some("1.0.0"),
            "installed_version must NOT be overwritten for featured items"
        );
    }

    /// When a software item is featured (manually created and enabled),
    /// periodic re-discovery must NOT overwrite `installed_version`.
    #[tokio::test]
    async fn process_one_discovery_manual_featured_item_skips_version_update() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        // Insert a manually created featured software item.
        let model = software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("wget".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        SoftwareItem::insert(model)
            .exec(&db)
            .await
            .expect("insert featured software_item");

        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
        };

        process_one_discovery(
            &db,
            tenant_id,
            host_id,
            pc_id,
            "package_manager_homebrew",
            args,
            now,
        )
        .await
        .expect("process_one_discovery");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("link query")
            .expect("link must exist");
        assert_eq!(
            link.installed_version.as_deref(),
            Some("1.0.0"),
            "installed_version must NOT be overwritten for manual items"
        );
    }

    // -- Target-based processing tests --

    /// A GitHub PHS item (with target) must create a `github_releases` plugin config.
    #[tokio::test]
    async fn target_based_github_creates_plugin_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("releases_github"))
            .all(&db)
            .await
            .expect("query configs");
        assert_eq!(configs.len(), 1, "expected one github_releases config");
        assert_eq!(configs[0].name, "BookLore/BookLore");

        let cfg_json = &configs[0].config;
        assert_eq!(cfg_json["owner"], "BookLore");
        assert_eq!(cfg_json["repo"], "BookLore");

        let hsi_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query hsi links");
        assert_eq!(hsi_links.len(), 1, "expected one host_software_item");
        assert_eq!(hsi_links[0].installed_version.as_deref(), Some("1.18.5"));

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("booklore"))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three role-based plugin links"
        );
    }

    #[tokio::test]
    async fn target_based_github_preserves_featured_true_on_initial_creation() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result = DiscoveryPluginResult {
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: "booklore".to_string(),
                name: "BookLore".to_string(),
                installed_version: "1.18.5".to_string(),
                targets: vec![DiscoveryTarget {
                    plugin_type: PluginType::ReleasesGithub,
                    plugin_config: serde_json::json!({
                        "owner": "BookLore",
                        "repo": "BookLore",
                        "tag_strip_prefix": "v",
                        "include_prereleases": false,
                        "asset_patterns": [],
                    }),
                    plugin_config_name: "BookLore/BookLore".to_string(),
                    roles: all_roles(),
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }],
                extra: None,
                featured: true,
                qualifier: None,
                plugin_package_identifier: None,
            }],
        };

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        let item = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Name.eq("BookLore"))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(&db)
            .await
            .expect("query software item")
            .expect("software item should exist");
        assert!(
            item.featured,
            "target-based discovery must preserve featured=true"
        );
    }

    /// An APT PHS item (with target) no longer creates a plugin_config;
    /// package manager types use plugin_type_settings at the tenant level.
    /// HSIP rows are created with `plugin_config_id = NULL`.
    #[tokio::test]
    async fn target_based_apt_creates_hsip_without_plugin_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result = phs_result_with_apt_target("grafana", "Grafana", "10.2.3");

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        // No plugin_config should be created for package manager types.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package_manager_apt"))
            .all(&db)
            .await
            .expect("query configs");
        assert!(
            configs.is_empty(),
            "package managers no longer create plugin_configs"
        );

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("grafana"))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three role-based plugin links"
        );
        // All links should have NULL plugin_config_id.
        for link in &plugin_links {
            assert!(
                link.plugin_config_id.is_none(),
                "package manager HSIP rows must have plugin_config_id = NULL"
            );
            assert_eq!(
                link.plugin_type, "package_manager_apt",
                "plugin_type must be set on the HSIP row"
            );
        }

        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query hsi");
        assert_eq!(hsi.len(), 1, "expected one host_software_item");
        assert_eq!(hsi[0].installed_version.as_deref(), Some("10.2.3"));
    }

    /// An item with no targets and no plugin_config_id must be skipped.
    #[tokio::test]
    async fn no_targets_no_config_id_skips_item() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        let result = phs_result_no_targets("n8n");

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query configs");
        assert!(
            configs.is_empty(),
            "no config should be created for skipped item"
        );

        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query items");
        assert!(items.is_empty(), "no software item should be created");
    }

    /// Two hosts discovering the same GitHub PHS app must share one plugin config.
    #[tokio::test]
    async fn target_based_two_hosts_share_github_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host1 = Uuid::now_v7();
        let host2 = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host1, tenant_id).await;
        insert_host(&db, host2, tenant_id).await;

        let result1 =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");
        let result2 =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_plugin_result(&db, tenant_id, host1, now, &result1, &HashSet::new())
            .await
            .expect("host1");
        process_plugin_result(&db, tenant_id, host2, now, &result2, &HashSet::new())
            .await
            .expect("host2");

        let config_count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("releases_github"))
            .count(&db)
            .await
            .expect("count configs");
        assert_eq!(
            config_count, 1,
            "both hosts must share a single plugin config"
        );

        let host1_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host1))
            .count(&db)
            .await
            .expect("host1 links");
        let host2_links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host2))
            .count(&db)
            .await
            .expect("host2 links");
        assert_eq!(host1_links, 1);
        assert_eq!(host2_links, 1);
    }

    /// Config-ID-based path: items with empty targets and a pre-existing
    /// plugin_config_id must use that config for all roles.
    #[tokio::test]
    async fn config_id_path_uses_existing_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        let result = DiscoveryPluginResult {
            plugin_type: PluginType::PackageManagerHomebrew,
            plugin_config_id: Some(pc_id),
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: "wget".to_string(),
                name: "Wget".to_string(),
                installed_version: "1.21.4".to_string(),
                targets: vec![],
                extra: None,
                featured: false,
                qualifier: None,
                plugin_package_identifier: None,
            }],
        };

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three role-based plugin links"
        );

        for link in &plugin_links {
            assert_eq!(link.package_identifier, "wget");
        }
    }

    #[tokio::test]
    async fn config_id_path_preserves_featured_true_on_initial_creation() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        let result = DiscoveryPluginResult {
            plugin_type: PluginType::PackageManagerHomebrew,
            plugin_config_id: Some(pc_id),
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: "cargo".to_string(),
                name: "Cargo".to_string(),
                installed_version: "1.86.0".to_string(),
                targets: vec![],
                extra: None,
                featured: true,
                qualifier: None,
                plugin_package_identifier: None,
            }],
        };

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        let item = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Name.eq("Cargo"))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .one(&db)
            .await
            .expect("query software item")
            .expect("software item should exist");
        assert!(
            item.featured,
            "plugin-provided featured=true must be preserved"
        );
    }

    #[tokio::test]
    async fn config_id_path_does_not_overwrite_existing_featured_state() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        let existing_item = software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("Cargo".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        SoftwareItem::insert(existing_item)
            .exec(&db)
            .await
            .expect("insert software item");
        insert_host_link(&db, host_id, item_id, pc_id, "cargo").await;

        let result = DiscoveryPluginResult {
            plugin_type: PluginType::PackageManagerHomebrew,
            plugin_config_id: Some(pc_id),
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: "cargo".to_string(),
                name: "Cargo".to_string(),
                installed_version: "1.86.0".to_string(),
                targets: vec![],
                extra: None,
                featured: true,
                qualifier: None,
                plugin_package_identifier: None,
            }],
        };

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result");

        let item = SoftwareItem::find_by_id(item_id)
            .one(&db)
            .await
            .expect("query software item")
            .expect("software item should exist");
        assert!(
            !item.featured,
            "rediscovery must not overwrite an existing featured choice"
        );
    }

    /// Target-based ignore rules work correctly: items on the ignore list
    /// for a target's software item name are skipped.
    #[tokio::test]
    async fn target_based_ignore_rules_work() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        // First, create the apt config by processing a normal item.
        let result1 = phs_result_with_apt_target("curl", "cURL", "8.0.0");
        let ignore_set = HashSet::new();
        process_plugin_result(&db, tenant_id, host_id, now, &result1, &ignore_set)
            .await
            .expect("first process");

        // Add "Wget" to the tenant-wide name-based ignore list.
        super::super::ignore_rules::create_or_ignore_ignore_rule(&db, tenant_id, "Wget", None)
            .await
            .expect("create ignore rule");

        // Reload the ignore set.
        let ignore_set = load_ignore_set(&db, tenant_id)
            .await
            .expect("load ignore set");

        // Now try to discover "wget" (name: "Wget") via the same apt target path.
        let result2 = phs_result_with_apt_target("wget", "Wget", "1.21.4");
        process_plugin_result(&db, tenant_id, host_id, now, &result2, &ignore_set)
            .await
            .expect("second process");

        // wget must NOT have been created.
        let wget_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("wget"))
            .count(&db)
            .await
            .expect("count");
        assert_eq!(wget_links, 0, "ignored item must not be created");
    }

    /// Regression test: a PHS item with both a GitHub `FetchReleases` target and
    /// a Shell `[DetectVersion, ExecuteUpdate]` target must produce exactly one
    /// `software_items` row, one `host_software_items` row, two `plugin_configs`,
    /// and three `host_software_item_plugins` rows (one per role).
    ///
    /// This exercises the Phase 3 name-collision fallback introduced to fix the
    /// `(tenant_id, name)` unique-index violation that previously aborted the
    /// entire processing loop when the Shell target tried to insert a
    /// `software_items` row whose name already existed from the GitHub target.
    #[tokio::test]
    async fn target_based_phs_two_targets_share_one_software_item() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result =
            phs_result_with_two_targets("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");

        process_plugin_result(&db, tenant_id, host_id, now, &result, &HashSet::new())
            .await
            .expect("process_plugin_result must not fail on name collision");

        // Exactly one software_items row.
        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::DeactivatedAt.is_null())
            .all(&db)
            .await
            .expect("query software_items");
        assert_eq!(items.len(), 1, "expected exactly one software_items row");
        assert_eq!(items[0].name, "BookLore");

        // Exactly one host_software_items link.
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query host_software_items");
        assert_eq!(
            hsi.len(),
            1,
            "expected exactly one host_software_items link"
        );

        // Exactly two plugin_configs: releases_github + generic_shell.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query plugin_configs");
        assert_eq!(configs.len(), 2, "expected two plugin_configs");
        let config_types: std::collections::HashSet<String> =
            configs.iter().map(|c| c.plugin_type.clone()).collect();
        assert!(
            config_types.contains("releases_github"),
            "expected a releases_github config"
        );
        assert!(
            config_types.contains("generic_shell"),
            "expected a generic_shell config"
        );

        // Exactly three host_software_item_plugins rows.
        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query host_software_item_plugins");
        assert_eq!(
            plugin_links.len(),
            3,
            "expected three host_software_item_plugins rows"
        );

        // Validate individual role assignments.
        let github_config_id = configs
            .iter()
            .find(|c| c.plugin_type == "releases_github")
            .unwrap()
            .id;
        let shell_config_id = configs
            .iter()
            .find(|c| c.plugin_type == "generic_shell")
            .unwrap()
            .id;

        let fetch = plugin_links
            .iter()
            .find(|l| l.role == "fetch_releases")
            .expect("fetch_releases role must exist");
        assert_eq!(fetch.plugin_config_id, Some(github_config_id));
        assert_eq!(fetch.package_identifier, "BookLore/BookLore");

        let detect = plugin_links
            .iter()
            .find(|l| l.role == "detect_version")
            .expect("detect_version role must exist");
        assert_eq!(detect.plugin_config_id, Some(shell_config_id));
        assert_eq!(detect.package_identifier, "booklore");

        let update = plugin_links
            .iter()
            .find(|l| l.role == "execute_update")
            .expect("execute_update role must exist");
        assert_eq!(update.plugin_config_id, Some(shell_config_id));
        assert_eq!(update.package_identifier, "booklore");
    }
}
