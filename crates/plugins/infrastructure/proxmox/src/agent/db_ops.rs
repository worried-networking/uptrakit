//! Database operations for the Proxmox plugin's agent-local tables.

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use super::entity::proxmox_host_state;
use super::entity::proxmox_pending_match;
use crate::{ProxmoxError, Result};
use uptrakit_shared_db::begin_immediate;

// ── proxmox_host_state ops ───────────────────────────────────────────────────

/// Find the Proxmox host state for a given SSH host.
pub async fn find_host_state(
    db: &DatabaseConnection,
    host_id: &str,
) -> Result<Option<proxmox_host_state::Model>> {
    proxmox_host_state::Entity::find_by_id(host_id)
        .one(db)
        .await
        .context_to::<ProxmoxError>()
}

/// Upsert PVE state for a host.
pub async fn upsert_host_state(
    db: &DatabaseConnection,
    host_id: &str,
    is_pve_node: bool,
    pve_node_name: Option<String>,
) -> Result<()> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    let existing = find_host_state(db, host_id).await?;

    if let Some(row) = existing {
        let mut model: proxmox_host_state::ActiveModel = row.into();
        model.is_pve_node = Set(is_pve_node);
        // This function never touches `pve_plugin_config_id` — the
        // controller-ack path (`set_plugin_config_id`) is its sole owner, so
        // a detection pass or re-bootstrap hook can never wipe it.
        if let Some(node_name) = pve_node_name {
            model.pve_node_name = Set(Some(node_name));
        }
        model.updated_at = Set(now);
        model.update(db).await.context_to::<ProxmoxError>()?;
    } else {
        let model = proxmox_host_state::ActiveModel {
            host_id: Set(host_id.to_string()),
            is_pve_node: Set(is_pve_node),
            pve_plugin_config_id: Set(None),
            pve_node_name: Set(pve_node_name),
            created_at: Set(now.clone()),
            updated_at: Set(now),
        };
        model.insert(db).await.context_to::<ProxmoxError>()?;
    }

    Ok(())
}

/// Find all hosts that are PVE nodes.
pub async fn find_pve_hosts(db: &DatabaseConnection) -> Result<Vec<proxmox_host_state::Model>> {
    proxmox_host_state::Entity::find()
        .filter(proxmox_host_state::Column::IsPveNode.eq(true))
        .all(db)
        .await
        .context_to::<ProxmoxError>()
}

/// Stamp the controller-acknowledged plugin config id on a host.
///
/// Plain single-column write: the controller-ack path
/// (`on_plugin_config_reported`) and the Branch 4 reuse-persist site are the
/// only writers of `pve_plugin_config_id`, so a stored value always means
/// "controller-acknowledged".
pub async fn set_plugin_config_id(
    db: &DatabaseConnection,
    host_id: &str,
    config_id: &str,
) -> Result<()> {
    use sea_orm::sea_query::Expr;

    proxmox_host_state::Entity::update_many()
        .col_expr(
            proxmox_host_state::Column::PvePluginConfigId,
            Expr::value(config_id),
        )
        .filter(proxmox_host_state::Column::HostId.eq(host_id))
        .exec(db)
        .await
        .context_to::<ProxmoxError>()?;
    Ok(())
}

// ── proxmox_pending_matches ops ──────────────────────────────────────────────

/// A pending Proxmox host-mapping match.
pub struct PendingMatch {
    pub id: i32,
    pub host_id: String,
    pub mapping_id: String,
    pub attempts: i32,
}

/// Insert a pending match record.
pub async fn insert_pending_match(
    db: &DatabaseConnection,
    host_id: &str,
    mapping_id: &str,
) -> Result<()> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    let row = proxmox_pending_match::ActiveModel {
        host_id: Set(host_id.to_string()),
        mapping_id: Set(mapping_id.to_string()),
        created_at: Set(now),
        attempts: Set(0),
        ..Default::default()
    };

    row.insert(db).await.context_to::<ProxmoxError>()?;
    Ok(())
}

/// Return all pending matches, ordered by `id` (insertion order).
pub async fn drain_pending_matches(db: &DatabaseConnection) -> Result<Vec<PendingMatch>> {
    use sea_orm::QueryOrder;

    let rows = proxmox_pending_match::Entity::find()
        .order_by_asc(proxmox_pending_match::Column::Id)
        .all(db)
        .await
        .context_to::<ProxmoxError>()?;

    Ok(rows
        .into_iter()
        .map(|r| PendingMatch {
            id: r.id,
            host_id: r.host_id,
            mapping_id: r.mapping_id,
            attempts: r.attempts,
        })
        .collect())
}

/// Delete a pending match by its row `id`.
pub async fn delete_pending_match(db: &DatabaseConnection, id: i32) -> Result<()> {
    proxmox_pending_match::Entity::delete_many()
        .filter(proxmox_pending_match::Column::Id.eq(id))
        .exec(db)
        .await
        .context_to::<ProxmoxError>()?;
    Ok(())
}

/// Increment the retry counter on a pending match after a failed drain attempt.
pub async fn increment_match_attempts(db: &DatabaseConnection, id: i32) -> Result<()> {
    use sea_orm::{ExprTrait, sea_query::Expr};
    proxmox_pending_match::Entity::update_many()
        .col_expr(
            proxmox_pending_match::Column::Attempts,
            Expr::col(proxmox_pending_match::Column::Attempts).add(1),
        )
        .filter(proxmox_pending_match::Column::Id.eq(id))
        .exec(db)
        .await
        .context_to::<ProxmoxError>()?;
    Ok(())
}

// ── tenant-rebind wipe ───────────────────────────────────────────────────────

/// Wipe all agent-local Proxmox state: every `proxmox_host_state` row and
/// every `proxmox_pending_matches` row. Called when the agent's tenant
/// binding changes — stale rows from the previous tenant (including
/// `pve_plugin_config_id` values pointing at a foreign tenant's plugin
/// config) must not satisfy reuse checks under the new tenant.
///
/// Both deletes run inside a single `begin_immediate()` transaction so a
/// failure partway through never leaves one table wiped and the other
/// holding stale, foreign-tenant rows.
pub async fn wipe_all(db: &DatabaseConnection) -> Result<()> {
    let txn = begin_immediate(db).await.context_to::<ProxmoxError>()?;
    proxmox_host_state::Entity::delete_many()
        .exec(&txn)
        .await
        .context_to::<ProxmoxError>()?;
    proxmox_pending_match::Entity::delete_many()
        .exec(&txn)
        .await
        .context_to::<ProxmoxError>()?;
    txn.commit().await.context_to::<ProxmoxError>()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_agent_db() -> sea_orm::DatabaseConnection {
        let db = sea_orm::Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        let manager = sea_orm_migration::SchemaManager::new(&db);
        for migration in crate::agent::migration::agent_migrations() {
            migration.up(&manager).await.expect("agent migration");
        }
        db
    }

    #[tokio::test]
    async fn increment_match_attempts_increments_counter() {
        let db = setup_agent_db().await;
        insert_pending_match(&db, "host-1", "mapping-1")
            .await
            .expect("insert pending match");

        let pending = drain_pending_matches(&db).await.expect("drain");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 0);
        let id = pending[0].id;

        increment_match_attempts(&db, id)
            .await
            .expect("increment 1");
        increment_match_attempts(&db, id)
            .await
            .expect("increment 2");

        let pending = drain_pending_matches(&db).await.expect("drain again");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 2);
    }

    #[tokio::test]
    async fn set_plugin_config_id_roundtrip_overwrites_existing_value() {
        let db = setup_agent_db().await;
        upsert_host_state(&db, "host-1", true, Some("node1".to_string()))
            .await
            .expect("upsert");
        set_plugin_config_id(&db, "host-1", "cfg-a")
            .await
            .expect("first write");
        // Intermediate state, before the overwrite below — red-checkable: a
        // no-op first write (e.g. an accidental early-return) would leave
        // this None instead of "cfg-a".
        let row_after_first_write = find_host_state(&db, "host-1")
            .await
            .expect("read")
            .expect("row exists");
        assert_eq!(
            row_after_first_write.pve_plugin_config_id.as_deref(),
            Some("cfg-a")
        );
        set_plugin_config_id(&db, "host-1", "cfg-b")
            .await
            .expect("overwrite");
        let row = find_host_state(&db, "host-1")
            .await
            .expect("read")
            .expect("row exists");
        // "cfg-b" differs from both the fixture default (None) and the first
        // write's value ("cfg-a") — a no-op second write would leave this at
        // "cfg-a", not "cfg-b", so this assertion is non-vacuous against a
        // no-op'd setter.
        assert_eq!(row.pve_plugin_config_id.as_deref(), Some("cfg-b"));
    }

    #[tokio::test]
    async fn upsert_with_none_node_name_preserves_existing_node_name() {
        let db = setup_agent_db().await;
        upsert_host_state(&db, "host-1", true, Some("node1".to_string()))
            .await
            .expect("seed with node name");
        // Second upsert passes `None` for `pve_node_name` — as re-detection
        // does when node-name detection fails on a later sync
        // (`plugin.rs`'s `node_name` becomes `None` on a `detect_pve_node_name`
        // error). "node1" differs from the fixture default (no row at all /
        // `None`), so a bug that unconditionally overwrote with `None` would
        // leave this assertion red rather than trivially passing.
        upsert_host_state(&db, "host-1", true, None)
            .await
            .expect("upsert with no node name");
        let row = find_host_state(&db, "host-1")
            .await
            .expect("read")
            .expect("row exists");
        assert_eq!(row.pve_node_name.as_deref(), Some("node1"));
    }

    #[tokio::test]
    async fn upsert_never_clears_plugin_config_id() {
        let db = setup_agent_db().await;
        upsert_host_state(&db, "host-1", true, Some("node1".to_string()))
            .await
            .expect("seed host");
        set_plugin_config_id(&db, "host-1", "cfg-a")
            .await
            .expect("stamp controller-ack config id");
        // Exercise the upsert path again (e.g. a later sync re-detecting PVE
        // node state) — it must never touch `pve_plugin_config_id`. "cfg-a"
        // differs from the fixture default (`None`), so a bug that cleared
        // the column on upsert would leave this assertion red rather than
        // trivially passing.
        upsert_host_state(&db, "host-1", true, Some("node1".to_string()))
            .await
            .expect("re-upsert");
        let row = find_host_state(&db, "host-1")
            .await
            .expect("read")
            .expect("row exists");
        assert_eq!(row.pve_plugin_config_id.as_deref(), Some("cfg-a"));
    }

    #[tokio::test]
    async fn tenant_rebind_wipes_local_state() {
        let db = setup_agent_db().await;
        upsert_host_state(&db, "h1", true, Some("node1".to_string()))
            .await
            .expect("seed h1");
        upsert_host_state(&db, "h2", true, Some("node2".to_string()))
            .await
            .expect("seed h2");
        // Non-PVE row: `upsert_host_state` also writes these (e.g. a host that
        // was probed and found not to be a PVE node), and `find_pve_hosts`
        // filters them out — a wipe scoped to `is_pve_node = true` would leave
        // this row behind undetected without an unfiltered assertion.
        upsert_host_state(&db, "h-non-pve", false, None)
            .await
            .expect("seed non-pve host");
        insert_pending_match(&db, "h1", "mapping-1")
            .await
            .expect("seed pending match");

        // Sanity: both tables are non-empty before the wipe — red-checkable:
        // a wipe_all that's a no-op (or that deletes the wrong table) leaves
        // these non-empty after the call below.
        let unfiltered_host_count = || async {
            proxmox_host_state::Entity::find()
                .all(&db)
                .await
                .expect("query all host state rows")
                .len()
        };
        assert_eq!(unfiltered_host_count().await, 3);
        assert_eq!(
            drain_pending_matches(&db)
                .await
                .expect("query pending")
                .len(),
            1
        );

        wipe_all(&db).await.expect("wipe_all");

        // Unfiltered count, not `find_pve_hosts` (which filters
        // `is_pve_node = true`): a wipe scoped to that filter would still
        // pass a `find_pve_hosts`-based assertion while leaving the
        // non-PVE row (and any is_pve_node=true row it missed) behind.
        assert_eq!(
            unfiltered_host_count().await,
            0,
            "proxmox_host_state must be empty after tenant rebind wipe, including non-PVE rows"
        );
        assert!(
            drain_pending_matches(&db)
                .await
                .expect("query pending")
                .is_empty(),
            "proxmox_pending_matches must be empty after tenant rebind wipe"
        );
    }
}
