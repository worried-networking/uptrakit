//! Processing incoming discovery results: creating pending software items
//! and upserting host-software-item links.

use super::default_configs::find_or_create_default_plugin_config;
use super::reconcile::HostSoftwareItemLinkView;
use super::{AutodiscoveryError, Result};
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, sea_query::Expr};
use std::collections::HashSet;
use time::OffsetDateTime;
use uptrakit_audit_log::{
    AuditActionType, AuditEmitter, AuditEntry, AuditOutcome, AuditView, Event,
};
use uptrakit_plugin_infrastructure_registry::is_package_manager_plugin;
use uptrakit_shared_db::entity::{
    host_software_item, host_software_item_plugin, prelude::*, software_ignore, software_item,
};
use uptrakit_shared_db::is_unique_constraint_violation;
use uptrakit_wire::{DiscoveryPluginResult, DiscoveryTarget};
use uuid::Uuid;

use crate::queries::software_items::SoftwareItemView;

/// Builds and fires one `Event`-kind audit entry (system actor, tenant scope,
/// `Success` outcome) for a reactivation site. Fire-and-forget: `discovery_items`
/// write paths run against a plain `DatabaseConnection` with no wrapping
/// transaction, so there is no in-tx `emit_stateful` write available -- see the
/// module-scope note on `HOST_SOFTWARE_ITEM_REACTIVATE`/`SOFTWARE_ITEM_REACTIVATE`
/// in `action_type.rs`.
fn emit_reactivation_event<V: AuditView>(
    audit: &AuditEmitter,
    tenant_id: Uuid,
    action: impl Into<AuditActionType>,
    view: &V,
) {
    match AuditEntry::<Event>::builder_event(action)
        .tenant_scope(tenant_id)
        .actor_system()
        .outcome(AuditOutcome::Success)
        .target(
            V::TARGET_TYPE,
            view.audit_target_id(),
            view.audit_target_display(),
        )
        .build()
    {
        Ok(entry) => audit.emit_event(entry),
        Err(err) => tracing::warn!(error = %err, "dropping invalid reactivation audit entry"),
    }
}

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
    /// Plugin-provided display version for the installed version (e.g. Docker image publish date).
    /// `None` when the plugin cannot determine a display version during discovery.
    pub installed_display_version: Option<&'a str>,
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

/// Shared per-call context for the discovery write path.
///
/// Groups the fields common to every write site so that downstream helper
/// functions stay within the workspace's argument-count lint. `discovery_source`
/// is the *discovering* plugin type (`result.plugin_type`) -- for PHS targets
/// this remains `discovery.proxmox-helper-scripts`, never the target's own
/// management plugin type.
struct DiscoveryContext<'a> {
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    discovery_source: &'a str,
    audit: &'a AuditEmitter,
}

/// Process a single plugin's discovery results.
///
/// Routes each item to the correct processing path based on its `targets`
/// and the result's `plugin_config_id`.
pub(super) async fn process_plugin_result(
    db: &sea_orm::DatabaseConnection,
    audit: &AuditEmitter,
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
            installed_display_version: item.installed_display_version.as_deref(),
        };
        let discovery_source = result.plugin_type.to_string();
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: &discovery_source,
            audit,
        };
        if !item.targets.is_empty() {
            process_targets_discovery(db, &ctx, &item_info, &item.targets).await?;
        } else if let Some(existing_pc_id) = result.plugin_config_id {
            process_one_discovery(
                db,
                &ctx,
                existing_pc_id,
                result.plugin_type.as_str(),
                item_info,
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
    ctx: &DiscoveryContext<'_>,
    item: &DiscoveredItemInfo<'_>,
    targets: &[DiscoveryTarget],
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
        let pc_id = if is_package_manager_plugin(&target.plugin_type) {
            None
        } else {
            Some(
                find_or_create_default_plugin_config(
                    db,
                    ctx.tenant_id,
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
            installed_display_version: item.installed_display_version,
        };

        // Find-or-create the software item and host link.
        let Some((software_item_id, hsi_id)) =
            find_or_create_software_item(db, ctx, pc_id, &target_plugin_type_str, &target_item)
                .await?
        else {
            continue;
        };

        // Create role assignments from the target's role list.
        for role in &target.roles {
            let plugin_link = host_software_item_plugin::ActiveModel {
                id: Set(Uuid::now_v7()),
                host_id: Set(ctx.host_id),
                software_item_id: Set(software_item_id),
                host_software_item_id: Set(hsi_id),
                plugin_config_id: Set(pc_id),
                plugin_type: Set(target_plugin_type_str.clone()),
                role: Set(role.as_str().to_string()),
                ordinal: Set(0),
                package_identifier: Set(target_item.effective_plugin_pkg_id().to_string()),
                config: Set(target.config_override.clone()),
                execution_site: Set(execution_site.to_owned()),
                created_at: Set(ctx.now),
                updated_at: Set(ctx.now),
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
/// Three-phase lookup. Match phases intentionally see deactivated rows so that
/// rediscovery reactivates them in place (spec §3) rather than falling through
/// to insert and creating duplicates:
///
/// 1. If this host already has a `host_software_item_plugin` row for the matching
///    identity key, update `installed_version` in place. If the linked
///    `host_software_item` row is deactivated, reactivate it (clear
///    `deactivated_at`, reset `missing_since`); if an ACTIVE row for the same
///    `(host, software_item, qualifier)` key also exists (collision rule 1),
///    prefer and update that active row instead, leaving the deactivated
///    duplicate untouched. If the linked `software_item` itself is deactivated,
///    attempt cascade reactivation: if an ACTIVE item with the same
///    `(tenant_id, name)` already exists (collision rule 2), re-point the link
///    to that active item and leave the originally deactivated item dormant;
///    otherwise reactivate the deactivated item in place.
/// 2. If *any other* host in the tenant has the same assignment backed by an active
///    software item, reuse it and insert a new `host_software_item` link for this host.
/// 3. Otherwise, if a DEACTIVATED `software_item` with the same name exists, reactivate
///    it in place; else create a new pending `software_item`. If the insert hits a
///    `(tenant_id, name)` unique-constraint violation (e.g. a second `DiscoveryTarget`
///    for the same `DiscoveredSoftware` item races through Phase 3 first), fall back
///    to a `(tenant_id, name)` lookup so both targets end up sharing the same
///    `software_item` row.
async fn find_or_create_software_item(
    db: &sea_orm::DatabaseConnection,
    ctx: &DiscoveryContext<'_>,
    plugin_config_id: Option<Uuid>,
    plugin_type_str: &str,
    item: &DiscoveredItemInfo<'_>,
) -> Result<Option<(Uuid, Uuid)>> {
    let tenant_id = ctx.tenant_id;
    let host_id = ctx.host_id;
    let now = ctx.now;
    let discovery_source = ctx.discovery_source;
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
        // Match deactivated software_item rows too -- rediscovery must
        // reactivate them in place rather than treating them as orphaned.
        //
        // `plugin_link.software_item_id` is guaranteed to resolve here: the
        // `host_software_item_plugins` table has a composite FK to
        // `host_software_items(host_id, software_item_id)` (`ON DELETE
        // CASCADE`), which itself has a direct FK to `software_items.id`
        // (also `ON DELETE CASCADE`). Deleting a `software_item` therefore
        // cascades through `host_software_items` and removes any
        // `host_software_item_plugins` row that referenced it, so a plugin
        // link can never outlive its software item. The one hard-delete
        // path in application code (`reset_data::reset_tenant_data`) also
        // deletes `host_software_item_plugins`/`host_software_items` before
        // `software_items`, so it never relies on the cascade either. There
        // is thus no code path that produces the "orphaned link" state, so
        // no fallback branch is needed here.
        let linked_item = SoftwareItem::find()
            .filter(software_item::Column::Id.eq(plugin_link.software_item_id))
            .one(db)
            .await
            .context_to()?
            .ok_or_else(|| {
                report!(AutodiscoveryError::Db(sea_orm::DbErr::RecordNotFound(
                    format!(
                        "software_item {} referenced by host_software_item_plugin {} not found",
                        plugin_link.software_item_id, plugin_link.id
                    )
                )))
            })?;
        let linked_item_id = linked_item.id;

        // Collision rule 2: cascade reactivation of a deactivated item must
        // not collide with an ACTIVE same-name item. If one exists, re-point
        // the link to it and leave the deactivated item dormant.
        let effective_item = if linked_item.deactivated_at.is_some() {
            let active_same_name = SoftwareItem::find()
                .filter(software_item::Column::TenantId.eq(tenant_id))
                .filter(software_item::Column::Name.eq(linked_item.name.as_str()))
                .filter(software_item::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?;
            if let Some(active_item) = active_same_name {
                tracing::debug!(
                    deactivated_item_id = %linked_item_id,
                    active_item_id = %active_item.id,
                    name = %linked_item.name,
                    "cascade reactivation collides with active same-name item; re-pointing link"
                );
                active_item
            } else {
                let mut active: software_item::ActiveModel = linked_item.into();
                active.deactivated_at = Set(None);
                let reactivated = active.update(db).await.context_to()?;
                emit_reactivation_event(
                    ctx.audit,
                    tenant_id,
                    AuditActionType::SOFTWARE_ITEM_REACTIVATE,
                    &SoftwareItemView::from(&reactivated),
                );
                reactivated
            }
        } else {
            linked_item
        };

        // Refresh discovery provenance for the matched link on every pass, but
        // preserve a non-NULL `installed_version` on links that were already
        // active: for registered items the `DetectVersion` scheduled task is
        // the sole version writer, so discovery must not clobber a value it
        // may disagree with. Version fields are still written on fresh
        // inserts, on link-level reactivation (`was_deactivated`), and when
        // the stored version is NULL. `featured` is only a presentation flag
        // (individual list entry vs aggregated per-host summary) and never
        // gates any of these writes.
        // See ADR-0037 (discovery version preservation).
        //
        // Collision rule 1: prefer an ACTIVE link row for the *target*
        // (host, effective_item, qualifier) key over repointing/reactivating
        // a deactivated duplicate. This also covers the cascade-repoint case:
        // if the active item already has an active link, prefer it and leave
        // the originally deactivated link (still keyed to `linked_item_id`)
        // untouched.
        let mut target_hsi_query = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(effective_item.id))
            .filter(host_software_item::Column::DeactivatedAt.is_null());
        target_hsi_query = match item.qualifier {
            Some(q) => target_hsi_query.filter(host_software_item::Column::Qualifier.eq(q)),
            None => target_hsi_query.filter(host_software_item::Column::Qualifier.is_null()),
        };
        let active_target_hsi = target_hsi_query.one(db).await.context_to()?;

        let hsi_to_update = if let Some(active_hsi) = active_target_hsi {
            Some(active_hsi)
        } else {
            // No active row at the target key yet: reactivate/repoint the
            // link row currently keyed to `linked_item_id` (the row Phase 1
            // actually matched).
            let mut orig_hsi_query = HostSoftwareItem::find()
                .filter(host_software_item::Column::HostId.eq(host_id))
                .filter(host_software_item::Column::SoftwareItemId.eq(linked_item_id));
            orig_hsi_query = match item.qualifier {
                Some(q) => orig_hsi_query.filter(host_software_item::Column::Qualifier.eq(q)),
                None => orig_hsi_query.filter(host_software_item::Column::Qualifier.is_null()),
            };
            orig_hsi_query.one(db).await.context_to()?
        };

        if let Some(hsi) = hsi_to_update {
            let target_hsi_id = hsi.id;
            let was_deactivated = hsi.deactivated_at.is_some();
            let preserve_version = !was_deactivated && hsi.installed_version.is_some();
            let mut active: host_software_item::ActiveModel = hsi.into();
            active.software_item_id = Set(effective_item.id);
            if !preserve_version {
                active.installed_version = Set(Some(installed_version.to_string()));
                active.installed_version_detected_at = Set(Some(now));
                active.installed_display_version =
                    Set(item.installed_display_version.map(str::to_string));
            }
            active.last_discovered_at = Set(Some(now));
            active.discovery_source = Set(Some(discovery_source.to_string()));
            active.missing_since = Set(None);
            active.deactivated_at = Set(None);
            let updated_hsi = active.update(db).await.context_to()?;
            if was_deactivated {
                emit_reactivation_event(
                    ctx.audit,
                    tenant_id,
                    AuditActionType::HOST_SOFTWARE_ITEM_REACTIVATE,
                    &HostSoftwareItemLinkView::from(&updated_hsi),
                );
            }

            // Cascade repoint (collision rule 2) moved the live identity
            // from `linked_item_id` to `effective_item.id`. The matched
            // `plugin_link` -- and any sibling role rows for the same
            // `(host, linked_item)` identity, since Phase 1 only matched
            // one role -- still reference the pre-repoint identity via
            // their own `software_item_id`/`host_software_item_id`
            // columns (set once at insert time, never updated above).
            // Bring every such row in line with the live target so role
            // dispatch never desyncs from the reactivated item.
            if effective_item.id != linked_item_id {
                reconcile_stale_plugin_links(
                    db,
                    host_id,
                    linked_item_id,
                    effective_item.id,
                    target_hsi_id,
                )
                .await?;
            }
        }
        return Ok(None);
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
        // Phase 3: no active item found via Phase 2's cross-host lookup. Check
        // for a DEACTIVATED same-name item in this tenant and reactivate it in
        // place instead of creating a duplicate (a fresh `software_items` row
        // would collide with none here since the existing one is deactivated,
        // but reusing it preserves history/identity across the deactivate ->
        // rediscover cycle).
        let deactivated_same_name = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .filter(software_item::Column::Name.eq(name))
            .filter(software_item::Column::DeactivatedAt.is_not_null())
            .one(db)
            .await
            .context_to()?;

        if let Some(dormant) = deactivated_same_name {
            tracing::debug!(
                software_item_id = %dormant.id,
                name = %name,
                "reactivating deactivated software_item found by name"
            );
            let mut active: software_item::ActiveModel = dormant.into();
            active.deactivated_at = Set(None);
            let reactivated = active.update(db).await.context_to()?;
            emit_reactivation_event(
                ctx.audit,
                tenant_id,
                AuditActionType::SOFTWARE_ITEM_REACTIVATE,
                &SoftwareItemView::from(&reactivated),
            );
            reactivated.id
        } else {
            // Create a new pending software item.
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
                awaiting_restart_timeout: Set(None),
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
                                format!(
                                    "software_item with name '{name}' not found after collision"
                                )
                            )))
                        })?
                        .id
                }
                Err(e) => return Err(report!(AutodiscoveryError::Db(e))),
            }
        }
    };

    // Before inserting, check whether a host_software_item row already exists
    // for this (host_id, software_item_id, qualifier) key -- active or
    // deactivated. The partial unique index only guards active rows, so a
    // stale deactivated row would otherwise not block a fresh insert and would
    // leave a duplicate. Reactivate the existing row in place instead.
    let mut existing_hsi_query = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id));
    existing_hsi_query = match item.qualifier {
        Some(q) => existing_hsi_query.filter(host_software_item::Column::Qualifier.eq(q)),
        None => existing_hsi_query.filter(host_software_item::Column::Qualifier.is_null()),
    };
    let existing_hsi_rows = existing_hsi_query.all(db).await.context_to()?;
    let preferred_existing_hsi = existing_hsi_rows
        .iter()
        .find(|h| h.deactivated_at.is_none())
        .or_else(|| existing_hsi_rows.first())
        .cloned();

    if let Some(hsi) = preferred_existing_hsi {
        let existing_id = hsi.id;
        let was_deactivated = hsi.deactivated_at.is_some();
        let preserve_version = !was_deactivated && hsi.installed_version.is_some();
        let mut active: host_software_item::ActiveModel = hsi.into();
        if !preserve_version {
            active.installed_version = Set(Some(installed_version.to_string()));
            active.installed_version_detected_at = Set(Some(now));
            active.installed_display_version =
                Set(item.installed_display_version.map(str::to_string));
        }
        active.last_discovered_at = Set(Some(now));
        active.discovery_source = Set(Some(discovery_source.to_string()));
        active.missing_since = Set(None);
        active.deactivated_at = Set(None);
        let updated_hsi = active.update(db).await.context_to()?;
        if was_deactivated {
            emit_reactivation_event(
                ctx.audit,
                tenant_id,
                AuditActionType::HOST_SOFTWARE_ITEM_REACTIVATE,
                &HostSoftwareItemLinkView::from(&updated_hsi),
            );
        }
        return Ok(Some((software_item_id, existing_id)));
    }

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
        installed_display_version: Set(item.installed_display_version.map(str::to_string)),
        latest_version: Set(None),
        latest_version_fetched_at: Set(None),
        latest_release_metadata: Set(None),
        last_updated_at: Set(None),
        linked_at: Set(now),
        update_category: Set("unknown".to_string()),
        deactivated_at: Set(None),
        last_discovered_at: Set(Some(now)),
        discovery_source: Set(Some(discovery_source.to_string())),
        missing_since: Set(None),
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

/// Repoint every `host_software_item_plugin` role row still keyed to
/// `(host_id, stale_item_id)` onto the live `(target_item_id, target_hsi_id)`
/// identity after a cascade reactivation (collision rule 2).
///
/// Phase 1 only matches a single role row via `.one(db)`, but a
/// `(host, software_item)` identity carries one row per role
/// (`detect_version` / `fetch_releases` / `execute_update`), so every
/// sibling row for the stale identity must move too -- otherwise role
/// dispatch stays desynced from the reactivated item for the roles Phase 1
/// didn't happen to match.
///
/// `target_hsi_id` may already own equivalent role rows (Case A: an active
/// target `host_software_item` pre-existed with its own plugin links). Moving
/// a stale row onto a `(host_software_item_id, role, ordinal)` combination
/// that's already taken would violate `uq_hsip_hsi_role_ordinal`, so such
/// stale rows are dropped instead (the target already has a live, correct
/// assignment for that role); rows with no conflicting role on the target are
/// updated in place via a single batched `update_many`.
async fn reconcile_stale_plugin_links(
    db: &sea_orm::DatabaseConnection,
    host_id: Uuid,
    stale_item_id: Uuid,
    target_item_id: Uuid,
    target_hsi_id: Uuid,
) -> Result<()> {
    let stale_links = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(stale_item_id))
        .all(db)
        .await
        .context_to()?;
    if stale_links.is_empty() {
        return Ok(());
    }

    // Roles already owned by the target HSI *other than* the stale rows
    // themselves. In Case B (`target_hsi_id` is the very row being repointed
    // in place, e.g. the stale rows' own `host_software_item_id`), the stale
    // rows are excluded here so they aren't mistaken for pre-existing
    // conflicts with themselves.
    let stale_ids: HashSet<Uuid> = stale_links.iter().map(|l| l.id).collect();
    let target_roles: HashSet<(String, i32)> = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostSoftwareItemId.eq(target_hsi_id))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .filter(|l| !stale_ids.contains(&l.id))
        .map(|l| (l.role, l.ordinal))
        .collect();

    let (conflicting, movable): (Vec<_>, Vec<_>) = stale_links
        .into_iter()
        .partition(|l| target_roles.contains(&(l.role.clone(), l.ordinal)));

    if !conflicting.is_empty() {
        let conflicting_ids: Vec<Uuid> = conflicting.iter().map(|l| l.id).collect();
        tracing::debug!(
            %host_id,
            %stale_item_id,
            %target_hsi_id,
            count = conflicting_ids.len(),
            "dropping stale plugin-link role rows superseded by the active target's own links"
        );
        HostSoftwareItemPlugin::delete_many()
            .filter(host_software_item_plugin::Column::Id.is_in(conflicting_ids))
            .exec(db)
            .await
            .context_to()?;
    }

    if !movable.is_empty() {
        let movable_ids: Vec<Uuid> = movable.iter().map(|l| l.id).collect();
        HostSoftwareItemPlugin::update_many()
            .col_expr(
                host_software_item_plugin::Column::SoftwareItemId,
                Expr::value(target_item_id),
            )
            .col_expr(
                host_software_item_plugin::Column::HostSoftwareItemId,
                Expr::value(target_hsi_id),
            )
            .filter(host_software_item_plugin::Column::Id.is_in(movable_ids))
            .exec(db)
            .await
            .context_to()?;
    }

    Ok(())
}

/// Process a single discovered software item using the config-ID path.
///
/// Used when items have no targets and the enclosing result has a pre-existing
/// `plugin_config_id`. Creates all three standard role assignments.
async fn process_one_discovery(
    db: &sea_orm::DatabaseConnection,
    ctx: &DiscoveryContext<'_>,
    plugin_config_id: Uuid,
    plugin_type_str: &str,
    args: DiscoveredItemInfo<'_>,
) -> Result<()> {
    let Some((software_item_id, hsi_id)) =
        find_or_create_software_item(db, ctx, Some(plugin_config_id), plugin_type_str, &args)
            .await?
    else {
        return Ok(());
    };

    // Create role plugin assignments for all three standard roles.
    for role in ["detect_version", "fetch_releases", "execute_update"] {
        let plugin_link = host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(ctx.host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(hsi_id),
            plugin_config_id: Set(Some(plugin_config_id)),
            plugin_type: Set(plugin_type_str.to_string()),
            role: Set(role.to_string()),
            ordinal: Set(0),
            package_identifier: Set(args.effective_plugin_pkg_id().to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(ctx.now),
            updated_at: Set(ctx.now),
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
    #![expect(
        clippy::expect_used,
        reason = "test code: panics on failure are acceptable"
    )]
    #![expect(clippy::panic, reason = "test code: panics on failure are acceptable")]

    use super::*;
    use crate::queries::autodiscovery::tests_common::{
        all_roles, insert_host, insert_host_link, insert_plugin_config, insert_software_item,
        insert_tenant, phs_result_no_targets, phs_result_with_apt_target,
        phs_result_with_github_target, phs_result_with_two_targets, setup_db, test_emitter,
    };
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
    use std::sync::Arc;
    use uptrakit_audit_log::{AuditLogDispatcher, DatabaseBackend};
    use uptrakit_shared_db::entity::plugin_config;
    use uptrakit_wire::{
        DiscoveredSoftware as WireDiscoveredSoftware, DiscoveryPluginResult, DiscoveryTarget,
        plugin_ids,
    };

    /// Real (non-Noop) audit emitter for observing `emit_event` writes: the
    /// dispatcher's background loop writes directly to `db` via
    /// `DatabaseBackend`, mirroring `reconcile.rs`'s `real_emitter` helper.
    fn real_emitter(db: &sea_orm::DatabaseConnection) -> AuditEmitter {
        AuditEmitter::new(AuditLogDispatcher::new(Arc::new(DatabaseBackend::new(
            db.clone(),
        ))))
    }

    /// Polls for the most recent tenant audit row with the given action,
    /// retrying briefly to allow the fire-and-forget `emit_event` dispatch
    /// loop to land the row (mirrors
    /// `web-api::routes::discovery_allowlist::latest_tenant_audit_row_for_action`).
    async fn latest_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> uptrakit_shared_db::entity::audit_log::Model {
        use uptrakit_shared_db::entity::audit_log;
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected audit row for action {action_type:?}");
    }

    /// When `find_or_create_software_item` encounters a host link pointing to a
    /// deactivated software item (with no active same-name collision), rediscovery
    /// must reactivate the item in place -- not delete the link and create a
    /// fresh pending item (spec §3: reactivation is update-in-place).
    /// The link itself was never deactivated, so its non-NULL version is
    /// preserved (link-level rule): only the item-level flag flips back.
    #[tokio::test]
    async fn process_one_discovery_deactivated_item_reactivates_in_place() {
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
            installed_display_version: None,
        };

        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
            .await
            .expect("process_one_discovery");

        // The link row must be reused (not deleted/recreated).
        let link_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(old_item_id))
            .count(&db)
            .await
            .expect("link count");
        assert_eq!(link_count, 1, "the existing host link must be reused");

        // The originally deactivated item must be reactivated in place -- no
        // new software_item should be created.
        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("items");
        assert_eq!(items.len(), 1, "no new software_item should be created");
        assert_eq!(items[0].id, old_item_id, "the original item must be reused");
        assert!(
            items[0].deactivated_at.is_none(),
            "the deactivated item must be reactivated in place"
        );
        assert!(
            !items[0].featured,
            "reactivated items should preserve plugin-provided featured=false"
        );

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::SoftwareItemId.eq(old_item_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert_eq!(
            link.installed_version.as_deref(),
            Some("1.0.0"),
            "an always-active link keeps its version through item-level reactivation"
        );

        let plugin_link_count = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PluginConfigId.eq(pc_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("curl"))
            .count(&db)
            .await
            .expect("plugin link count");
        assert_eq!(
            plugin_link_count, 3,
            "expected the pre-existing plugin link rows for all three roles (no duplicates)"
        );

        // Reactivating a deactivated software_item must emit a
        // `SOFTWARE_ITEM_REACTIVATE` audit event (finding C1). The emit is
        // fire-and-forget (`emit_event`), so poll for the row to land.
        let row = latest_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_REACTIVATE,
        )
        .await;
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(old_item_id.to_string().as_str())
        );
    }

    /// Reactivating a deactivated `host_software_item` link in place (Phase 1,
    /// no cascade repoint) must emit a `HOST_SOFTWARE_ITEM_REACTIVATE` audit
    /// event (finding C1). The emit is fire-and-forget (`emit_event`), so this
    /// polls for the row rather than asserting immediately after the await.
    #[tokio::test]
    async fn process_one_discovery_deactivated_link_emits_reactivate_audit() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, item_id, tenant_id, "jq", None).await;
        let hsi_id = crate::queries::autodiscovery::tests_common::insert_discovered_host_link(
            &db,
            host_id,
            item_id,
            pc_id,
            "jq",
            None,
            "package-manager.homebrew",
            Some(now),
            None,
            Some(now),
        )
        .await;

        let args = DiscoveredItemInfo {
            package_identifier: "jq",
            name: "jq",
            installed_version: "1.7.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };

        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
            .await
            .expect("process_one_discovery");

        let link = HostSoftwareItem::find_by_id(hsi_id)
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.deactivated_at.is_none(),
            "the link must be reactivated in place"
        );

        let row = latest_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::HOST_SOFTWARE_ITEM_REACTIVATE,
        )
        .await;
        assert_eq!(row.tenant_id, tenant_id);
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.target_type.as_deref(), Some("host_software_item"));
        assert_eq!(row.target_id.as_deref(), Some(hsi_id.to_string().as_str()));
    }

    /// `process_one_discovery` must preserve a non-NULL `installed_version` when
    /// the existing host link is already active: for registered items the
    /// `DetectVersion` scheduled task is the sole version writer.
    #[tokio::test]
    async fn process_one_discovery_active_link_preserves_version() {
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
            installed_display_version: None,
        };

        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
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
            Some("1.0.0"),
            "installed_version must be preserved on an active link"
        );
    }

    /// An active link whose `installed_version` is NULL is filled by discovery:
    /// writing into NULL overrides nothing.
    #[tokio::test]
    async fn process_one_discovery_fills_null_installed_version() {
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

        // Clear the fixture's seeded version so the link starts NULL.
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        let mut clear: host_software_item::ActiveModel = hsi.into();
        clear.installed_version = Set(None);
        clear.installed_version_detected_at = Set(None);
        clear.update(&db).await.expect("clear installed_version");

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };
        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
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
            Some("2.0.0"),
            "a NULL installed_version must be filled by discovery"
        );
        assert!(
            link.installed_version_detected_at.is_some(),
            "installed_version_detected_at must be stamped on fill"
        );
    }

    /// An active link with a non-NULL version keeps all three version fields
    /// while discovery still stamps presence/provenance on every pass.
    #[tokio::test]
    async fn process_one_discovery_preserves_version_but_stamps_presence() {
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
        // Seeds installed_version="1.0.0", NULL provenance, active.
        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        // Seed missing_since so the clear-on-presence assertion below proves a
        // real transition instead of restating the fixture default.
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        let mut mark: host_software_item::ActiveModel = hsi.into();
        mark.missing_since = Set(Some(now));
        mark.update(&db).await.expect("seed missing_since");

        // DB-roundtripped pre-discovery value, for the unchanged assertion below.
        let detected_before = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist")
            .installed_version_detected_at;

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: Some("v2.0.0-display"),
        };
        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
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
            "non-NULL installed_version must be preserved"
        );
        assert!(
            link.installed_display_version.is_none(),
            "installed_display_version must not be written when the version is preserved"
        );
        assert_eq!(
            link.installed_version_detected_at, detected_before,
            "installed_version_detected_at must not be bumped when the version is preserved"
        );
        assert_eq!(
            link.discovery_source.as_deref(),
            Some("package-manager.homebrew"),
            "discovery_source must still be stamped"
        );
        assert!(
            link.last_discovered_at.is_some(),
            "last_discovered_at must still be stamped"
        );
        assert!(
            link.missing_since.is_none(),
            "the seeded missing_since must be cleared on re-presence"
        );
    }

    /// `featured` is only a presentation flag: re-discovery of a featured item
    /// preserves its non-NULL `installed_version` (like any active link) while
    /// still stamping discovery provenance
    /// (`last_discovered_at`/`discovery_source`) so reconciliation can track it.
    #[tokio::test]
    async fn process_one_discovery_featured_item_preserves_version_stamps_provenance() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        // Insert a featured software item.
        let model = software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("wget".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        };
        SoftwareItem::insert(model)
            .exec(&db)
            .await
            .expect("insert featured software_item");

        // insert_host_link seeds installed_version="1.0.0" and NULL provenance.
        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };

        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
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
            "installed_version must be preserved for active featured items"
        );
        assert!(
            link.last_discovered_at.is_some(),
            "last_discovered_at must be stamped for featured items"
        );
        assert_eq!(
            link.discovery_source.as_deref(),
            Some("package-manager.homebrew"),
            "discovery_source must be stamped for featured items"
        );
    }

    /// A manually-created featured item likewise keeps its non-NULL version on
    /// re-discovery; provenance is still stamped.
    #[tokio::test]
    async fn process_one_discovery_manual_featured_item_preserves_version_stamps_provenance() {
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
            awaiting_restart_timeout: Set(None),
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
            installed_display_version: None,
        };

        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
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
            "installed_version must be preserved for active manually-created items"
        );
        assert!(
            link.last_discovered_at.is_some(),
            "last_discovered_at must be stamped for manual featured items"
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

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
        .await
        .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("releases.github"))
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
            plugin_type: plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![WireDiscoveredSoftware {
                package_identifier: "booklore".to_string(),
                name: "BookLore".to_string(),
                installed_version: "1.18.5".to_string(),
                targets: vec![DiscoveryTarget {
                    plugin_type: plugin_ids::RELEASES_GITHUB.clone(),
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
                installed_display_version: None,
            }],
        };

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
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

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
        .await
        .expect("process_plugin_result");

        // No plugin_config should be created for package manager types.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package-manager.apt"))
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
                link.plugin_type, "package-manager.apt",
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

    #[tokio::test]
    async fn target_based_npm_creates_hsip_without_plugin_config() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let mut result = phs_result_with_apt_target("pm2", "PM2", "5.4.2");
        result.discoveries[0].targets[0].plugin_type = plugin_ids::PACKAGE_MANAGER_NPM.clone();

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
        .await
        .expect("process_plugin_result");

        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("package-manager.npm"))
            .all(&db)
            .await
            .expect("query configs");
        assert!(
            configs.is_empty(),
            "package managers no longer create plugin_configs"
        );

        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .filter(host_software_item_plugin::Column::PackageIdentifier.eq("pm2"))
            .all(&db)
            .await
            .expect("query plugin links");
        assert!(
            !plugin_links.is_empty(),
            "expected plugin links for npm target"
        );
        for link in &plugin_links {
            assert!(
                link.plugin_config_id.is_none(),
                "package manager HSIP rows must have plugin_config_id = NULL"
            );
            assert_eq!(link.plugin_type, "package-manager.npm");
        }
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

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
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

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host1,
            now,
            &result1,
            &HashSet::new(),
        )
        .await
        .expect("host1");
        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host2,
            now,
            &result2,
            &HashSet::new(),
        )
        .await
        .expect("host2");

        let config_count = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq("releases.github"))
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
            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
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
                installed_display_version: None,
            }],
        };

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
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
            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
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
                installed_display_version: None,
            }],
        };

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
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
            awaiting_restart_timeout: Set(None),
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
            plugin_type: plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
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
                installed_display_version: None,
            }],
        };

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
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
        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result1,
            &ignore_set,
        )
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
        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result2,
            &ignore_set,
        )
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

        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
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

        // Exactly two plugin_configs: releases.github + generic.shell.
        let configs = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("query plugin_configs");
        assert_eq!(configs.len(), 2, "expected two plugin_configs");
        let config_types: std::collections::HashSet<String> =
            configs.iter().map(|c| c.plugin_type.clone()).collect();
        assert!(
            config_types.contains("releases.github"),
            "expected a releases.github config"
        );
        assert!(
            config_types.contains("generic.shell"),
            "expected a generic.shell config"
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
            .find(|c| c.plugin_type == "releases.github")
            .unwrap()
            .id;
        let shell_config_id = configs
            .iter()
            .find(|c| c.plugin_type == "generic.shell")
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

    /// Every discovery create/match must stamp provenance columns:
    /// `last_discovered_at`, `discovery_source` (the *discovering* plugin
    /// type -- for PHS targets this is `discovery.proxmox-helper-scripts`,
    /// not the target's `releases.github`), and clear `missing_since`.
    ///
    /// Runs `process_plugin_result` twice: the first pass exercises Phase 3
    /// (create), the second exercises Phase 1 (in-place update / re-match).
    #[tokio::test]
    async fn discovery_create_and_rematch_stamp_provenance() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;

        let result =
            phs_result_with_github_target("booklore", "BookLore", "1.18.5", "BookLore", "BookLore");
        let expected_plugin_type_str = plugin_ids::DISCOVERY_PROXMOX_HELPER_SCRIPTS.to_string();

        // First pass: create.
        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
        .await
        .expect("first process_plugin_result");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("q")
            .expect("row");
        assert!(
            link.last_discovered_at.is_some(),
            "last_discovered_at must be stamped on create"
        );
        assert_eq!(
            link.discovery_source.as_deref(),
            Some(expected_plugin_type_str.as_str()),
            "discovery_source must be the discovering plugin type, not the target's"
        );
        assert!(
            link.missing_since.is_none(),
            "missing_since must be clear on create"
        );

        // Second pass: re-match (Phase 1 in-place update).
        process_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &HashSet::new(),
        )
        .await
        .expect("second process_plugin_result");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("q")
            .expect("row");
        assert!(
            link.last_discovered_at.is_some(),
            "last_discovered_at must be stamped on rematch"
        );
        assert_eq!(
            link.discovery_source.as_deref(),
            Some(expected_plugin_type_str.as_str()),
            "discovery_source must remain the discovering plugin type after rematch"
        );
        assert!(
            link.missing_since.is_none(),
            "missing_since must be clear after rematch"
        );
    }

    // -- Reactivation tests (Task 3) --

    /// Rediscovery of a previously deactivated link + item must reactivate both
    /// in place: clear `deactivated_at` on the link and the item, reset
    /// `missing_since = NULL`, and stamp fresh provenance -- without creating
    /// any new rows.
    #[tokio::test]
    async fn rediscovery_reactivates_deactivated_link_and_item_in_place() {
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

        // Simulate reconciliation having deactivated both the link and the item.
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("query hsi")
            .expect("hsi must exist");
        let hsi_id = hsi.id;
        let mut hsi_active: host_software_item::ActiveModel = hsi.into();
        hsi_active.deactivated_at = Set(Some(now));
        hsi_active.missing_since = Set(Some(now));
        hsi_active.update(&db).await.expect("deactivate hsi");

        let item = SoftwareItem::find_by_id(item_id)
            .one(&db)
            .await
            .expect("query item")
            .expect("item must exist");
        let mut item_active: software_item::ActiveModel = item.into();
        item_active.deactivated_at = Set(Some(now));
        item_active.update(&db).await.expect("deactivate item");

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };
        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
            .await
            .expect("process_one_discovery");

        // No new rows: same item id, same link id.
        let items = SoftwareItem::find()
            .filter(software_item::Column::TenantId.eq(tenant_id))
            .all(&db)
            .await
            .expect("items");
        assert_eq!(items.len(), 1, "no new software_item should be created");
        assert_eq!(items[0].id, item_id, "the original item must be reused");
        assert!(
            items[0].deactivated_at.is_none(),
            "item.deactivated_at must be cleared on reactivation"
        );

        let links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("links");
        assert_eq!(
            links.len(),
            1,
            "no new host_software_item should be created"
        );
        assert_eq!(links[0].id, hsi_id, "the original link must be reused");
        assert!(
            links[0].deactivated_at.is_none(),
            "link.deactivated_at must be cleared on reactivation"
        );
        assert!(
            links[0].missing_since.is_none(),
            "missing_since must be reset to NULL on reactivation"
        );
        assert_eq!(
            links[0].installed_version.as_deref(),
            Some("2.0.0"),
            "installed_version must be refreshed on reactivation"
        );
    }

    /// Collision rule 1: if an ACTIVE link already exists for the same
    /// `(host, software_item, qualifier)` key, rediscovery must prefer and
    /// update the active row, leaving a co-existing deactivated duplicate
    /// untouched (row count unchanged).
    #[tokio::test]
    async fn reactivation_prefers_existing_active_link_row() {
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
        // insert_host_link creates one active link + 3 plugin-link rows.
        insert_host_link(&db, host_id, item_id, pc_id, "wget").await;

        let active_hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .one(&db)
            .await
            .expect("query active hsi")
            .expect("active hsi must exist");
        let active_hsi_id = active_hsi.id;

        // Insert a second, deactivated link row directly for the same key.
        // The partial unique index excludes deactivated rows, so this is legal.
        let deactivated_hsi_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(deactivated_hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(pc_id)),
            package_identifier: Set(Some("wget".to_string())),
            installed_version: Set(Some("0.9.0".to_string())),
            installed_version_detected_at: Set(Some(now)),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(Some(now)),
            last_discovered_at: Set(None),
            discovery_source: Set(None),
            missing_since: Set(Some(now)),
        }
        .insert(&db)
        .await
        .expect("insert deactivated duplicate link");

        let args = DiscoveredItemInfo {
            package_identifier: "wget",
            name: "wget",
            installed_version: "2.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };
        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
            .await
            .expect("process_one_discovery");

        // Row count unchanged: still exactly two host_software_item rows for this item.
        let links = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
            .all(&db)
            .await
            .expect("links");
        assert_eq!(
            links.len(),
            2,
            "row count must be unchanged: no new link created"
        );

        let active_after = links
            .iter()
            .find(|l| l.id == active_hsi_id)
            .expect("active row must still exist");
        assert!(
            active_after.deactivated_at.is_none(),
            "the active row must remain active"
        );
        assert_eq!(
            active_after.installed_version.as_deref(),
            Some("1.0.0"),
            "the preferred active row keeps its non-NULL version"
        );

        let deactivated_after = links
            .iter()
            .find(|l| l.id == deactivated_hsi_id)
            .expect("deactivated duplicate must still exist");
        assert!(
            deactivated_after.deactivated_at.is_some(),
            "the deactivated duplicate must be left untouched (still deactivated)"
        );
        assert_eq!(
            deactivated_after.installed_version.as_deref(),
            Some("0.9.0"),
            "the deactivated duplicate must not be modified"
        );
    }

    /// Collision rule 2: cascade-reactivation of a deactivated `software_item`
    /// must not violate `uq_software_items_active_name`. If an ACTIVE item with
    /// the same name already exists, the link must be re-pointed to the active
    /// item and the originally deactivated item left dormant.
    #[tokio::test]
    async fn cascade_reactivation_repoints_link_when_active_same_name_item_exists() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let deactivated_item_id = Uuid::now_v7();
        let active_item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        // The deactivated item + its (also deactivated) host link.
        insert_software_item(&db, deactivated_item_id, tenant_id, "Foo", None).await;
        insert_host_link(&db, host_id, deactivated_item_id, pc_id, "foo").await;
        let hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(deactivated_item_id))
            .one(&db)
            .await
            .expect("query hsi")
            .expect("hsi must exist");
        let mut hsi_active: host_software_item::ActiveModel = hsi.into();
        hsi_active.deactivated_at = Set(Some(now));
        hsi_active.missing_since = Set(Some(now));
        hsi_active.update(&db).await.expect("deactivate hsi");

        let item = SoftwareItem::find_by_id(deactivated_item_id)
            .one(&db)
            .await
            .expect("query item")
            .expect("item must exist");
        let mut item_active: software_item::ActiveModel = item.into();
        item_active.deactivated_at = Set(Some(now));
        item_active.update(&db).await.expect("deactivate item");

        // A separate, ACTIVE software_item with the same name (same tenant).
        insert_software_item(&db, active_item_id, tenant_id, "Foo", None).await;

        let args = DiscoveredItemInfo {
            package_identifier: "foo",
            name: "Foo",
            installed_version: "3.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };
        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
            .await
            .expect("process_one_discovery");

        // The originally deactivated item must remain dormant (untouched).
        let deactivated_item_after = SoftwareItem::find_by_id(deactivated_item_id)
            .one(&db)
            .await
            .expect("query deactivated item")
            .expect("deactivated item must still exist");
        assert!(
            deactivated_item_after.deactivated_at.is_some(),
            "the originally deactivated item must remain dormant"
        );

        // The link must now point at the active item.
        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(active_item_id))
            .one(&db)
            .await
            .expect("query repointed link")
            .expect("link on the active item must exist");
        assert!(
            link.deactivated_at.is_none(),
            "the repointed link must be active"
        );
        assert_eq!(
            link.installed_version.as_deref(),
            Some("3.0.0"),
            "the repointed link must carry the fresh discovery data"
        );

        // No lingering active link on the deactivated item.
        let stale_link_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(deactivated_item_id))
            .filter(host_software_item::Column::DeactivatedAt.is_null())
            .count(&db)
            .await
            .expect("count stale active links");
        assert_eq!(
            stale_link_count, 0,
            "no active link should remain on the dormant item"
        );

        // Case B: no active target HSI pre-existed, so the orig HSI row was
        // repointed in place -- its plugin-link rows (all three roles) must
        // now reference the live item and HSI, not the deactivated ones.
        let plugin_links = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query plugin links");
        assert_eq!(
            plugin_links.len(),
            3,
            "all three role rows must exist (no duplicates, none dropped)"
        );
        for pl in &plugin_links {
            assert_eq!(
                pl.software_item_id, active_item_id,
                "plugin_link.software_item_id must reference the live active item, role={}",
                pl.role
            );
            assert_eq!(
                pl.host_software_item_id, link.id,
                "plugin_link.host_software_item_id must reference the repointed HSI, role={}",
                pl.role
            );
        }
    }

    /// Collision rule 2, Case A: an ACTIVE target `host_software_item` already
    /// exists (with its own live plugin-link rows) for `(host, effective_item,
    /// qualifier)` when the cascade repoint happens. The matched `plugin_link`
    /// (and its sibling role rows, still keyed to the deactivated orig item/HSI)
    /// must be reconciled onto the pre-existing active target: repointed where
    /// the target has no equivalent role row yet, and dropped (deduped) where
    /// the target already owns that role -- never producing a
    /// `uq_hsip_hsi_role_ordinal` conflict or a dangling stale reference.
    #[tokio::test]
    async fn cascade_reactivation_case_a_reconciles_plugin_links_onto_active_target_hsi() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        let deactivated_item_id = Uuid::now_v7();
        let active_item_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;

        // The deactivated item + its (also deactivated) host link and plugin
        // links -- these are the "stale" rows Phase 1 will match one of.
        insert_software_item(&db, deactivated_item_id, tenant_id, "Foo", None).await;
        insert_host_link(&db, host_id, deactivated_item_id, pc_id, "foo").await;
        let stale_hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(deactivated_item_id))
            .one(&db)
            .await
            .expect("query stale hsi")
            .expect("stale hsi must exist");
        let mut hsi_active: host_software_item::ActiveModel = stale_hsi.into();
        hsi_active.deactivated_at = Set(Some(now));
        hsi_active.missing_since = Set(Some(now));
        hsi_active.update(&db).await.expect("deactivate stale hsi");

        let item = SoftwareItem::find_by_id(deactivated_item_id)
            .one(&db)
            .await
            .expect("query item")
            .expect("item must exist");
        let mut item_active: software_item::ActiveModel = item.into();
        item_active.deactivated_at = Set(Some(now));
        item_active.update(&db).await.expect("deactivate item");

        // A separate, ACTIVE software_item with the same name, ALREADY carrying
        // its own active host link + plugin links (Case A precondition).
        insert_software_item(&db, active_item_id, tenant_id, "Foo", None).await;
        insert_host_link(&db, host_id, active_item_id, pc_id, "foo").await;
        let active_hsi = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .filter(host_software_item::Column::SoftwareItemId.eq(active_item_id))
            .one(&db)
            .await
            .expect("query active hsi")
            .expect("active hsi must exist");
        let active_hsi_id = active_hsi.id;

        // Sanity: two HSIs, six plugin-link rows (three stale + three live) before reconciliation.
        let plugin_links_before = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .count(&db)
            .await
            .expect("count plugin links before");
        assert_eq!(plugin_links_before, 6, "precondition: six plugin links");

        let args = DiscoveredItemInfo {
            package_identifier: "foo",
            name: "Foo",
            installed_version: "3.0.0",
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        };
        let audit = real_emitter(&db);
        let ctx = DiscoveryContext {
            tenant_id,
            host_id,
            now,
            discovery_source: "package-manager.homebrew",
            audit: &audit,
        };
        process_one_discovery(&db, &ctx, pc_id, "package-manager.homebrew", args)
            .await
            .expect("process_one_discovery");

        // The originally deactivated item must remain dormant (untouched).
        let deactivated_item_after = SoftwareItem::find_by_id(deactivated_item_id)
            .one(&db)
            .await
            .expect("query deactivated item")
            .expect("deactivated item must still exist");
        assert!(
            deactivated_item_after.deactivated_at.is_some(),
            "the originally deactivated item must remain dormant"
        );

        // The pre-existing active HSI must be the one updated (no new HSI created).
        let hsi_count = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .count(&db)
            .await
            .expect("count hsis");
        assert_eq!(
            hsi_count, 2,
            "no new host_software_item should be created; still two (stale + active)"
        );

        let active_hsi_after = HostSoftwareItem::find_by_id(active_hsi_id)
            .one(&db)
            .await
            .expect("query active hsi after")
            .expect("active hsi must still exist");
        assert_eq!(
            active_hsi_after.installed_version.as_deref(),
            Some("1.0.0"),
            "the pre-existing active hsi keeps its version through the cascade repoint"
        );

        // Every plugin_link row for this host must now reference the live
        // (active_item_id, active_hsi_id) identity -- no row may still point
        // at the deactivated item or its stale HSI, and no duplicate role rows
        // may exist (the unique index would reject that).
        let plugin_links_after = HostSoftwareItemPlugin::find()
            .filter(host_software_item_plugin::Column::HostId.eq(host_id))
            .all(&db)
            .await
            .expect("query plugin links after");
        assert_eq!(
            plugin_links_after.len(),
            3,
            "stale duplicate role rows must be deduped away; exactly three remain"
        );
        for pl in &plugin_links_after {
            assert_eq!(
                pl.software_item_id, active_item_id,
                "plugin_link.software_item_id must reference the live active item, role={}",
                pl.role
            );
            assert_eq!(
                pl.host_software_item_id, active_hsi_id,
                "plugin_link.host_software_item_id must reference the active target hsi, role={}",
                pl.role
            );
        }
        let roles: std::collections::HashSet<&str> =
            plugin_links_after.iter().map(|l| l.role.as_str()).collect();
        assert!(roles.contains("detect_version"));
        assert!(roles.contains("fetch_releases"));
        assert!(roles.contains("execute_update"));
    }
}
