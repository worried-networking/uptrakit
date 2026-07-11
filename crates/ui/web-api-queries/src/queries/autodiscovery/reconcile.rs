//! Discovery-based uninstall reconciliation.
//!
//! For each error-free [`DiscoveryPluginResult`], reconciles the active
//! `host_software_items` links whose `discovery_source` matches the reporting
//! plugin against the identifiers the plugin's snapshot actually reported:
//!
//! - **Present** in the snapshot: clears `missing_since` (if set) and refreshes
//!   `last_discovered_at`. This always runs before any active-update skip logic.
//! - **Absent** from the snapshot: deferred entirely while an update is actively
//!   rewriting the link's state (`InProgress`/`Pending`/`Queued`). Otherwise
//!   applies two-miss hysteresis with a 1-hour age floor: the first miss stamps
//!   `missing_since`; a link is deactivated only once `now - missing_since` has
//!   reached [`RECONCILE_MIN_MISSING_AGE`].
//!
//! Deactivating a link cascades to its `software_item` when no other active
//! link references it, and force-fails any non-terminal `update_history` rows
//! for that link. All state changes for one plugin result happen inside a
//! single `BEGIN IMMEDIATE` transaction (SQLite read-then-write rule); audit
//! events are written in-tx via `emit_stateful`, then the commit hook is
//! flushed after commit.
//!
//! Links with no discovery provenance (`last_discovered_at IS NULL`, i.e.
//! manually-added items) or a provenance from a different plugin
//! (`discovery_source != result.plugin_type`) are never touched here.
//!
//! Audit scope note: Task 3's reactivation path
//! (`discovery_items::find_or_create_software_item`) writes directly against
//! the plain `DatabaseConnection` with no wrapping transaction, so it cannot
//! participate in an in-tx `emit_stateful` write here without changing that
//! path's transaction shape. Reconciliation therefore only emits audit events
//! for the state changes it owns end-to-end via `emit_stateful`: deactivation,
//! the software-item cascade, and in-flight update termination. Reactivation
//! is audited separately, at its own sites in `discovery_items.rs`, via the
//! fire-and-forget `emit_event` path (`HOST_SOFTWARE_ITEM_REACTIVATE` /
//! `SOFTWARE_ITEM_REACTIVATE`, both registered as `AuditActionKind::Event`)
//! since no transaction wraps that path. Presence-clear remains unaudited: it
//! is a routine refresh (clearing `missing_since` on rediscovery), not a state
//! transition worth its own audit entry.

use std::collections::HashSet;

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseTransaction, EntityTrait, PaginatorTrait, QueryFilter,
    Set, SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use time::OffsetDateTime;
use uptrakit_audit_log::{
    AuditActionType, AuditCommitHook, AuditEmitter, AuditEntry, AuditOutcome, AuditView,
};
use uptrakit_shared_db::entity::{host_software_item, prelude::*, software_item, update_history};
use uptrakit_shared_types::UpdateStatus;
use uptrakit_wire::DiscoveryPluginResult;
use uuid::Uuid;

use super::Result;
use crate::queries::software_items::SoftwareItemView;

/// Minimum age `missing_since` must reach before a link is deactivated.
///
/// Guards against transient discovery flakiness: a link only deactivates once
/// it has been observed absent across at least two reconciliation passes
/// spanning this much wall-clock time.
pub(super) const RECONCILE_MIN_MISSING_AGE: time::Duration = time::Duration::hours(1);

/// Message written to `update_history.output` when an in-flight, non-terminal
/// update is force-failed because its underlying link was deactivated.
const UNINSTALLED_UPDATE_OUTPUT: &str = "software no longer installed on host";

/// Secret-safe snapshot of a `host_software_items` row for audit views.
///
/// `pub(super)` so `discovery_items::find_or_create_software_item`'s
/// reactivation sites can reuse it for their `HOST_SOFTWARE_ITEM_REACTIVATE`
/// `emit_event` calls, rather than duplicating an equivalent view type.
#[derive(uptrakit_audit_log::AuditView)]
#[audit(target_type = "host_software_item")]
pub(super) struct HostSoftwareItemLinkView {
    id: Uuid,
    host_id: Uuid,
    software_item_id: Uuid,
    package_identifier: Option<String>,
    discovery_source: Option<String>,
}

impl From<&host_software_item::Model> for HostSoftwareItemLinkView {
    fn from(model: &host_software_item::Model) -> Self {
        Self {
            id: model.id,
            host_id: model.host_id,
            software_item_id: model.software_item_id,
            package_identifier: model.package_identifier.clone(),
            discovery_source: model.discovery_source.clone(),
        }
    }
}

/// Reconcile one error-free plugin result's discovery-owned links against the
/// identifiers it actually reported.
///
/// Opens and commits its own `BEGIN IMMEDIATE` transaction, then flushes the
/// audit commit hook. Caller (`process_discovery_results`) guarantees
/// `result.error.is_none()`.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id, plugin_type = %result.plugin_type))]
pub(super) async fn reconcile_plugin_result(
    db: &sea_orm::DatabaseConnection,
    audit: &AuditEmitter,
    tenant_id: Uuid,
    host_id: Uuid,
    now: OffsetDateTime,
    result: &DiscoveryPluginResult,
    effective_identifiers: &HashSet<(String, Option<String>)>,
) -> Result<()> {
    debug_assert!(
        result.error.is_none(),
        "caller guarantees error-free result"
    );

    let hook = audit.commit_hook();
    let tx = db
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
        .context_to()?;

    let candidates = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .filter(host_software_item::Column::DiscoverySource.eq(result.plugin_type.to_string()))
        .filter(host_software_item::Column::LastDiscoveredAt.is_not_null())
        .all(&tx)
        .await
        .context_to()?;

    for link in candidates {
        let present = link.package_identifier.as_ref().is_some_and(|pid| {
            effective_identifiers.contains(&(pid.clone(), link.qualifier.clone()))
        });

        if present {
            if link.missing_since.is_some() {
                let mut active: host_software_item::ActiveModel = link.into();
                active.missing_since = Set(None);
                active.last_discovered_at = Set(Some(now));
                active.update(&tx).await.context_to()?;
            }
            continue;
        }

        // Absent from this snapshot: defer entirely while an update is
        // actively rewriting this link's state.
        if has_active_update(&tx, link.id).await? {
            continue;
        }

        match link.missing_since {
            None => {
                let mut active: host_software_item::ActiveModel = link.into();
                active.missing_since = Set(Some(now));
                active.update(&tx).await.context_to()?;
            }
            Some(since) if now - since >= RECONCILE_MIN_MISSING_AGE => {
                deactivate_link(&tx, &hook, audit, tenant_id, link, now).await?;
            }
            Some(_) => {
                // Second (or later) miss inside the age floor: wait.
            }
        }
    }

    tx.commit().await.context_to()?;
    hook.flush_after_commit().await;
    Ok(())
}

/// Returns true when the link has an `update_history` row whose status is
/// still actively being rewritten by the update-execution path
/// (`InProgress`/`Pending`/`Queued`).
async fn has_active_update(tx: &DatabaseTransaction, link_id: Uuid) -> Result<bool> {
    let count = UpdateHistory::find()
        .filter(update_history::Column::HostSoftwareItemId.eq(Some(link_id)))
        .filter(update_history::Column::Status.is_in([
            UpdateStatus::InProgress,
            UpdateStatus::Pending,
            UpdateStatus::Queued,
        ]))
        .count(tx)
        .await
        .context_to()?;
    Ok(count > 0)
}

/// Deactivates a link that has been absent past the age floor, cascading to
/// its `software_item` when no other active link remains and force-failing
/// any non-terminal `update_history` rows tied to it. Emits one `Stateful`
/// audit event per state change, in-tx.
async fn deactivate_link(
    tx: &DatabaseTransaction,
    hook: &AuditCommitHook,
    audit: &AuditEmitter,
    tenant_id: Uuid,
    link: host_software_item::Model,
    now: OffsetDateTime,
) -> Result<()> {
    let before_view = HostSoftwareItemLinkView::from(&link);
    let software_item_id = link.software_item_id;

    let mut active: host_software_item::ActiveModel = link.clone().into();
    active.deactivated_at = Set(Some(now));
    let updated_link = active.update(tx).await.context_to()?;
    let after_view = HostSoftwareItemLinkView::from(&updated_link);

    emit_reconcile_stateful(
        tx,
        hook,
        audit,
        tenant_id,
        AuditActionType::HOST_SOFTWARE_ITEM_DEACTIVATE,
        &before_view,
        &after_view,
    )
    .await?;

    // Cascade: deactivate the software_item once no active links remain.
    let remaining_active = HostSoftwareItem::find()
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item::Column::DeactivatedAt.is_null())
        .count(tx)
        .await
        .context_to()?;

    if remaining_active == 0
        && let Some(item) = SoftwareItem::find_by_id(software_item_id)
            .one(tx)
            .await
            .context_to()?
    {
        let item_before = SoftwareItemView::from(&item);
        let mut item_active: software_item::ActiveModel = item.into();
        item_active.deactivated_at = Set(Some(now));
        let updated_item = item_active.update(tx).await.context_to()?;
        let item_after = SoftwareItemView::from(&updated_item);

        emit_reconcile_stateful(
            tx,
            hook,
            audit,
            tenant_id,
            AuditActionType::SOFTWARE_ITEM_DEACTIVATE,
            &item_before,
            &item_after,
        )
        .await?;
    }

    // Force-fail any non-terminal update_history rows tied to this link. The
    // full `unfinished()` set is {Queued, Pending, InProgress, AwaitingRestart},
    // but this function is only reached after `has_active_update` (which
    // already defers {InProgress, Pending, Queued}) returns false, so in
    // practice only `AwaitingRestart` rows can still be here; the broader
    // filter is kept as defense-in-depth rather than narrowed to that one variant.
    let unfinished_rows = UpdateHistory::find()
        .filter(update_history::Column::HostSoftwareItemId.eq(Some(updated_link.id)))
        .filter(update_history::Column::Status.is_in(UpdateStatus::unfinished()))
        .all(tx)
        .await
        .context_to()?;

    if !unfinished_rows.is_empty() {
        let terminate_before = after_view;
        for row in unfinished_rows {
            let mut active: update_history::ActiveModel = row.into();
            active.status = Set(UpdateStatus::Failed);
            active.completed_at = Set(Some(now));
            let output = UNINSTALLED_UPDATE_OUTPUT.to_string();
            active.output_bytes = Set(output.len() as i64);
            active.output = Set(output);
            active.update(tx).await.context_to()?;
        }

        // One event per deactivated link covering the termination sweep,
        // rather than one per terminated update_history row (the link is the
        // audited target; `update_history` rows are not themselves views).
        emit_reconcile_stateful(
            tx,
            hook,
            audit,
            tenant_id,
            AuditActionType::UPDATE_TERMINATE_UNINSTALLED,
            &terminate_before,
            &terminate_before,
        )
        .await?;
    }

    Ok(())
}

/// Builds and emits one `Stateful` audit entry in-tx (system actor, tenant
/// scope, `Success` outcome) for a before/after view pair.
async fn emit_reconcile_stateful<V: AuditView>(
    tx: &DatabaseTransaction,
    hook: &AuditCommitHook,
    audit: &AuditEmitter,
    tenant_id: Uuid,
    action: impl Into<AuditActionType>,
    before: &V,
    after: &V,
) -> Result<()> {
    let entry = AuditEntry::builder_stateful(action)
        .tenant_scope(tenant_id)
        .actor_system()
        .outcome(AuditOutcome::Success)
        .before(before)
        .after(after)
        .build()
        .context_to()?;
    audit.emit_stateful(tx, hook, entry).await.context_to()?;
    Ok(())
}

#[cfg_attr(
    all(test, feature = "db-sqlite"),
    expect(
        clippy::expect_used,
        reason = "test helpers: panics on setup failure are acceptable"
    )
)]
#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use std::collections::HashSet;
    use std::sync::Arc;

    use sea_orm::{
        ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait,
        QueryFilter, Set,
    };
    use time::{Duration, OffsetDateTime};
    use uptrakit_audit_log::{AuditEmitter, AuditLogDispatcher, DatabaseBackend, NoopBackend};
    use uptrakit_shared_db::entity::{host_software_item, prelude::*, update_history};
    use uptrakit_shared_types::UpdateStatus;
    use uptrakit_wire::{DiscoveryPluginResult, plugin_ids};
    use uuid::Uuid;

    use super::super::tests_common::*;
    use super::{UNINSTALLED_UPDATE_OUTPUT, reconcile_plugin_result};

    const SOURCE: &str = "package_manager_apt";

    fn empty_result() -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_type: plugin_ids::PACKAGE_MANAGER_APT.clone(),
            plugin_config_id: None,
            error: None,
            discoveries: vec![],
        }
    }

    fn ids_with(pkg: &str, qualifier: Option<&str>) -> HashSet<(String, Option<String>)> {
        let mut set = HashSet::new();
        set.insert((pkg.to_string(), qualifier.map(str::to_string)));
        set
    }

    /// Real (non-Noop) audit emitter: the in-tx `audit_logs` write is real,
    /// only the post-commit mirror is discarded.
    fn real_emitter(db: &DatabaseConnection) -> AuditEmitter {
        AuditEmitter::with_backends(
            AuditLogDispatcher::new(Arc::new(NoopBackend)),
            Arc::new(DatabaseBackend::new(db.clone())),
            Arc::new(NoopBackend),
        )
    }

    async fn insert_update_history(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        host_software_item_id: Option<Uuid>,
        status: UpdateStatus,
        now: OffsetDateTime,
    ) -> Uuid {
        let id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            from_version: Set(None),
            to_version: Set(Some("2.0.0".to_string())),
            status: Set(status),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(None),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .expect("insert update_history");
        id
    }

    #[tokio::test]
    async fn absent_twice_past_age_floor_deactivates() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.deactivated_at.is_some(),
            "link past the age floor must be deactivated"
        );

        let item = SoftwareItem::find_by_id(software_item_id)
            .one(&db)
            .await
            .expect("query item")
            .expect("item must exist");
        assert!(
            item.deactivated_at.is_some(),
            "sole active link deactivating must cascade to the software_item"
        );
    }

    #[tokio::test]
    async fn absent_once_only_stamps_missing_since() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(1)),
            None,
            None,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.missing_since.is_some(),
            "first miss must stamp missing_since"
        );
        assert!(
            link.deactivated_at.is_none(),
            "a single miss must not deactivate"
        );
    }

    #[tokio::test]
    async fn two_misses_inside_age_floor_do_not_deactivate() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::minutes(20)),
            Some(now - Duration::minutes(10)),
            None,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.deactivated_at.is_none(),
            "a second miss inside the age floor must not deactivate yet"
        );
    }

    #[tokio::test]
    async fn presence_clears_missing_since() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;

        let mut result = empty_result();
        result.discoveries.push(uptrakit_wire::DiscoveredSoftware {
            package_identifier: "curl".to_string(),
            name: "curl".to_string(),
            installed_version: "8.0.0".to_string(),
            targets: vec![],
            extra: None,
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        });

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &ids_with("curl", None),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.missing_since.is_none(),
            "presence in the snapshot must clear missing_since"
        );
        assert_eq!(
            link.last_discovered_at,
            Some(now),
            "presence must refresh last_discovered_at"
        );
    }

    #[tokio::test]
    async fn manual_provenance_null_link_untouched() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        // Manually-added link: no discovery provenance at all.
        insert_host_link(&db, host_id, software_item_id, pc_id, "curl").await;

        let now = OffsetDateTime::now_utc();
        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.missing_since.is_none(),
            "a manual (no-provenance) link must never be touched by reconciliation"
        );
        assert!(link.deactivated_at.is_none());
    }

    #[tokio::test]
    async fn other_discovery_source_untouched() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            "other_plugin",
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_id))
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.deactivated_at.is_none(),
            "a link discovered by a different plugin must not be reconciled here"
        );
    }

    #[tokio::test]
    async fn last_host_cascade_deactivates_item_multi_host_does_not() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_a = Uuid::now_v7();
        let host_b = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_a, tenant_id).await;
        insert_host(&db, host_b, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        insert_discovered_host_link(
            &db,
            host_a,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;
        // A second, unrelated active link to the same software_item from another host.
        insert_host_link(&db, host_b, software_item_id, pc_id, "curl").await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_a,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link_a = HostSoftwareItem::find()
            .filter(host_software_item::Column::HostId.eq(host_a))
            .one(&db)
            .await
            .expect("query link a")
            .expect("link a must exist");
        assert!(
            link_a.deactivated_at.is_some(),
            "host_a's link must deactivate"
        );

        let item = SoftwareItem::find_by_id(software_item_id)
            .one(&db)
            .await
            .expect("query item")
            .expect("item must exist");
        assert!(
            item.deactivated_at.is_none(),
            "software_item must remain active while host_b's link is still active"
        );
    }

    #[tokio::test]
    async fn in_progress_update_defers_absent_candidate() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        let link_id = insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            None,
            None,
        )
        .await;
        insert_update_history(
            &db,
            tenant_id,
            host_id,
            software_item_id,
            Some(link_id),
            UpdateStatus::InProgress,
            now,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find_by_id(link_id)
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.missing_since.is_none(),
            "an absent candidate with an in-progress update must be deferred entirely, not stamped"
        );
        assert!(link.deactivated_at.is_none());
    }

    #[tokio::test]
    async fn in_progress_update_does_not_block_presence_clear() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        let link_id = insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;
        insert_update_history(
            &db,
            tenant_id,
            host_id,
            software_item_id,
            Some(link_id),
            UpdateStatus::InProgress,
            now,
        )
        .await;

        let mut result = empty_result();
        result.discoveries.push(uptrakit_wire::DiscoveredSoftware {
            package_identifier: "curl".to_string(),
            name: "curl".to_string(),
            installed_version: "8.0.0".to_string(),
            targets: vec![],
            extra: None,
            featured: false,
            qualifier: None,
            plugin_package_identifier: None,
            installed_display_version: None,
        });

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &result,
            &ids_with("curl", None),
        )
        .await
        .expect("reconcile must succeed");

        let link = HostSoftwareItem::find_by_id(link_id)
            .one(&db)
            .await
            .expect("query link")
            .expect("link must exist");
        assert!(
            link.missing_since.is_none(),
            "presence-clear must run regardless of an in-progress update"
        );
    }

    #[tokio::test]
    async fn awaiting_restart_terminates_failed_on_deactivation() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        let link_id = insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;
        let update_id = insert_update_history(
            &db,
            tenant_id,
            host_id,
            software_item_id,
            Some(link_id),
            UpdateStatus::AwaitingRestart,
            now,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let row = UpdateHistory::find_by_id(update_id)
            .one(&db)
            .await
            .expect("query update_history")
            .expect("row must exist");
        assert_eq!(row.status, UpdateStatus::Failed);
        assert_eq!(row.output, UNINSTALLED_UPDATE_OUTPUT);
        assert!(row.completed_at.is_some());
    }

    #[tokio::test]
    async fn terminal_update_rows_untouched() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        let link_id = insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;
        let update_id = insert_update_history(
            &db,
            tenant_id,
            host_id,
            software_item_id,
            Some(link_id),
            UpdateStatus::Completed,
            now,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let row = UpdateHistory::find_by_id(update_id)
            .one(&db)
            .await
            .expect("query update_history")
            .expect("row must exist");
        assert_eq!(
            row.status,
            UpdateStatus::Completed,
            "a terminal row must not be touched by deactivation"
        );
    }

    #[tokio::test]
    async fn audit_rows_emitted_for_deactivate_cascade_terminate() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, software_item_id, tenant_id, "curl", None).await;

        let now = OffsetDateTime::now_utc();
        let link_id = insert_discovered_host_link(
            &db,
            host_id,
            software_item_id,
            pc_id,
            "curl",
            None,
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;
        insert_update_history(
            &db,
            tenant_id,
            host_id,
            software_item_id,
            Some(link_id),
            UpdateStatus::AwaitingRestart,
            now,
        )
        .await;

        reconcile_plugin_result(
            &db,
            &real_emitter(&db),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &HashSet::new(),
        )
        .await
        .expect("reconcile must succeed");

        let count = AuditLog::find()
            .filter(uptrakit_shared_db::entity::audit_log::Column::TenantId.eq(tenant_id))
            .filter(
                uptrakit_shared_db::entity::audit_log::Column::ActionType.is_in([
                    uptrakit_audit_log::AuditActionType::HOST_SOFTWARE_ITEM_DEACTIVATE.as_str(),
                    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_DEACTIVATE.as_str(),
                    uptrakit_audit_log::AuditActionType::UPDATE_TERMINATE_UNINSTALLED.as_str(),
                ]),
            )
            .count(&db)
            .await
            .expect("count audit rows");
        assert_eq!(
            count, 3,
            "expected one audit row each for link deactivate, item cascade, and update termination"
        );
    }

    #[tokio::test]
    async fn qualifier_links_reconcile_within_their_qualifier() {
        let db = setup_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let item_a = Uuid::now_v7();
        let item_b = Uuid::now_v7();
        let pc_id = Uuid::now_v7();
        insert_tenant(&db, tenant_id).await;
        insert_host(&db, host_id, tenant_id).await;
        insert_plugin_config(&db, pc_id, tenant_id).await;
        insert_software_item(&db, item_a, tenant_id, "nginx-a", None).await;
        insert_software_item(&db, item_b, tenant_id, "nginx-b", None).await;

        let now = OffsetDateTime::now_utc();
        let link_a = insert_discovered_host_link(
            &db,
            host_id,
            item_a,
            pc_id,
            "nginx",
            Some("a"),
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;
        let link_b = insert_discovered_host_link(
            &db,
            host_id,
            item_b,
            pc_id,
            "nginx",
            Some("b"),
            SOURCE,
            Some(now - Duration::hours(3)),
            Some(now - Duration::hours(2)),
            None,
        )
        .await;

        // Snapshot reports only qualifier "a" as present.
        reconcile_plugin_result(
            &db,
            &test_emitter(),
            tenant_id,
            host_id,
            now,
            &empty_result(),
            &ids_with("nginx", Some("a")),
        )
        .await
        .expect("reconcile must succeed");

        let a = HostSoftwareItem::find_by_id(link_a)
            .one(&db)
            .await
            .expect("query link a")
            .expect("link a must exist");
        let b = HostSoftwareItem::find_by_id(link_b)
            .one(&db)
            .await
            .expect("query link b")
            .expect("link b must exist");
        assert!(
            a.deactivated_at.is_none(),
            "qualifier 'a' is present in the snapshot and must remain active"
        );
        assert!(
            b.deactivated_at.is_some(),
            "qualifier 'b' is absent from the snapshot and must deactivate"
        );
    }
}
