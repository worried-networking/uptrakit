//! Database operations for the Proxmox plugin's agent-local tables.

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};

use super::entity::proxmox_host_state;
use super::entity::proxmox_pending_match;
use crate::{ProxmoxError, Result};

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
    pve_plugin_config_id: Option<String>,
    pve_node_name: Option<String>,
) -> Result<()> {
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    let existing = find_host_state(db, host_id).await?;

    if let Some(row) = existing {
        let mut model: proxmox_host_state::ActiveModel = row.into();
        model.is_pve_node = Set(is_pve_node);
        // Preserve-on-None: a caller with no config id / node name to report
        // (e.g. a re-bootstrap hook that runs before the credential flow)
        // must not wipe an already-migrated row's operative config id or
        // drop the row out of the cluster set on a transient detection miss.
        // Targeted setters (`set_new_plugin_config_id`, `promote_cluster_rows`)
        // own explicit clearing.
        if let Some(config_id) = pve_plugin_config_id {
            model.pve_plugin_config_id = Set(Some(config_id));
        }
        if let Some(node_name) = pve_node_name {
            model.pve_node_name = Set(Some(node_name));
        }
        model.updated_at = Set(now);
        model.update(db).await.context_to::<ProxmoxError>()?;
    } else {
        let model = proxmox_host_state::ActiveModel {
            host_id: Set(host_id.to_string()),
            is_pve_node: Set(is_pve_node),
            pve_plugin_config_id: Set(pve_plugin_config_id),
            pve_node_name: Set(pve_node_name),
            created_at: Set(now.clone()),
            updated_at: Set(now),
            legacy_pve_user: Set(None),
            new_pve_plugin_config_id: Set(None),
            migration_attempts: Set(0),
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

/// Find a PVE host that already has a `pve_plugin_config_id` set.
pub async fn find_pve_host_with_config(
    db: &DatabaseConnection,
) -> Result<Option<proxmox_host_state::Model>> {
    proxmox_host_state::Entity::find()
        .filter(proxmox_host_state::Column::IsPveNode.eq(true))
        .filter(proxmox_host_state::Column::PvePluginConfigId.is_not_null())
        .one(db)
        .await
        .context_to::<ProxmoxError>()
}

/// Stamp (or clear) the legacy PVE user marker on a set of cluster rows.
pub async fn set_legacy_pve_user(
    db: &DatabaseConnection,
    host_ids: &[String],
    legacy: Option<String>,
) -> Result<()> {
    proxmox_host_state::Entity::update_many()
        .col_expr(
            proxmox_host_state::Column::LegacyPveUser,
            sea_orm::sea_query::Expr::value(legacy),
        )
        .filter(proxmox_host_state::Column::HostId.is_in(host_ids.iter().map(String::as_str)))
        .exec(db)
        .await
        .context_to::<ProxmoxError>()?;
    Ok(())
}

/// Stamp the ack-confirmed new plugin config id on a host, and promote it to
/// the operative `pve_plugin_config_id` only when the latter is currently
/// NULL (a fresh bootstrap gets its operative id immediately; a
/// migration-window row keeps its legacy operative id until phase-2
/// promotion). The ack marker itself is never cleared elsewhere.
pub async fn set_new_plugin_config_id(
    db: &DatabaseConnection,
    host_id: &str,
    config_id: &str,
) -> Result<()> {
    use sea_orm::sea_query::{Expr, Func};

    proxmox_host_state::Entity::update_many()
        .col_expr(
            proxmox_host_state::Column::NewPvePluginConfigId,
            Expr::value(config_id),
        )
        .col_expr(
            proxmox_host_state::Column::PvePluginConfigId,
            Expr::expr(Func::coalesce([
                Expr::col(proxmox_host_state::Column::PvePluginConfigId),
                Expr::value(config_id),
            ])),
        )
        .filter(proxmox_host_state::Column::HostId.eq(host_id))
        .exec(db)
        .await
        .context_to::<ProxmoxError>()?;
    Ok(())
}

/// Phase-2 promotion: point the given cluster rows at the new operative
/// config id, clear the legacy-user marker, and reset the attempt counter.
/// Never touches `new_pve_plugin_config_id` — the ack marker is never
/// cleared once set.
pub async fn promote_cluster_rows(
    db: &DatabaseConnection,
    host_ids: &[String],
    new_config_id: &str,
) -> Result<()> {
    use sea_orm::sea_query::Expr;

    proxmox_host_state::Entity::update_many()
        .col_expr(
            proxmox_host_state::Column::PvePluginConfigId,
            Expr::value(new_config_id),
        )
        .col_expr(
            proxmox_host_state::Column::LegacyPveUser,
            Expr::value(Option::<String>::None),
        )
        .col_expr(
            proxmox_host_state::Column::MigrationAttempts,
            Expr::value(0),
        )
        .filter(proxmox_host_state::Column::HostId.is_in(host_ids.iter().map(String::as_str)))
        .exec(db)
        .await
        .context_to::<ProxmoxError>()?;
    Ok(())
}

/// Increment the phase-2 migration attempt counter on a set of cluster rows.
pub async fn increment_migration_attempts(
    db: &DatabaseConnection,
    host_ids: &[String],
) -> Result<()> {
    use sea_orm::{ExprTrait, sea_query::Expr};
    proxmox_host_state::Entity::update_many()
        .col_expr(
            proxmox_host_state::Column::MigrationAttempts,
            Expr::col(proxmox_host_state::Column::MigrationAttempts).add(1),
        )
        .filter(proxmox_host_state::Column::HostId.is_in(host_ids.iter().map(String::as_str)))
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
    async fn migration_setters_roundtrip_and_promotion_retains_ack_marker() {
        let db = setup_agent_db().await;
        upsert_host_state(
            &db,
            "h1",
            true,
            Some("legacy-cfg".to_string()),
            Some("node1".to_string()),
        )
        .await
        .expect("seed h1");
        upsert_host_state(
            &db,
            "h2",
            true,
            Some("legacy-cfg".to_string()),
            Some("node2".to_string()),
        )
        .await
        .expect("seed h2");
        let ids = vec!["h1".to_string(), "h2".to_string()];

        set_legacy_pve_user(&db, &ids, Some("uptrakit-t@pve".to_string()))
            .await
            .expect("set legacy");
        set_new_plugin_config_id(&db, "h1", "new-cfg")
            .await
            .expect("set new id");
        // Legacy operative id survives the ack marker (promotion is phase 2's job) —
        // red-checkable: unconditional overwrite fails this.
        let h1_pre = find_host_state(&db, "h1")
            .await
            .expect("query")
            .expect("h1 exists");
        assert_eq!(h1_pre.pve_plugin_config_id.as_deref(), Some("legacy-cfg"));

        // Fresh row (NULL operative id) is promoted immediately on ack — nothing else
        // writes pve_plugin_config_id on a clean cluster; red-checkable: writing only
        // the marker breaks guest bootstrap on fresh clusters.
        upsert_host_state(&db, "h3", true, None, Some("node3".to_string()))
            .await
            .expect("seed h3");
        set_new_plugin_config_id(&db, "h3", "new-cfg")
            .await
            .expect("set h3");
        let h3 = find_host_state(&db, "h3")
            .await
            .expect("query")
            .expect("h3 exists");
        assert_eq!(h3.pve_plugin_config_id.as_deref(), Some("new-cfg"));
        assert_eq!(h3.new_pve_plugin_config_id.as_deref(), Some("new-cfg"));

        increment_migration_attempts(&db, &ids)
            .await
            .expect("bump attempts");

        promote_cluster_rows(&db, &ids, "new-cfg")
            .await
            .expect("promote");

        let h1 = find_host_state(&db, "h1")
            .await
            .expect("query")
            .expect("h1 exists");
        let h2 = find_host_state(&db, "h2")
            .await
            .expect("query")
            .expect("h2 exists");
        for row in [&h1, &h2] {
            assert_eq!(row.pve_plugin_config_id.as_deref(), Some("new-cfg"));
            assert_eq!(
                row.legacy_pve_user, None,
                "promotion clears the legacy marker"
            );
            assert_eq!(row.migration_attempts, 0, "promotion resets attempts");
        }
        // The ack marker is NEVER cleared (spec § 4) — red-checkable: clearing it in
        // promote_cluster_rows fails this.
        assert_eq!(h1.new_pve_plugin_config_id.as_deref(), Some("new-cfg"));

        // Update-arm preserve-on-None: re-upserting with no config id / node name
        // (the hooks-before-flow shape from a later task) must not wipe either column.
        upsert_host_state(&db, "h1", true, None, None)
            .await
            .expect("re-upsert h1");
        let h1_post = find_host_state(&db, "h1")
            .await
            .expect("query")
            .expect("h1 exists");
        assert_eq!(h1_post.pve_plugin_config_id.as_deref(), Some("new-cfg"));
        assert_eq!(h1_post.pve_node_name.as_deref(), Some("node1"));
    }
}
