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
        model.pve_plugin_config_id = Set(pve_plugin_config_id);
        model.pve_node_name = Set(pve_node_name);
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
}
