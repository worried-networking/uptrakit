use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;

/// Repair migration for controller deployments that previously ran the
/// monolithic `m20260331_000001_ssh_agent_tables` migration.
///
/// That migration created all SSH agent tables in one shot. The new schema
/// ownership model puts each migration in `agent-ssh-runtime`, contributed
/// via `service_migrations()`. SeaORM would try to re-run the 13 standalone
/// migrations unless their names are already present in `seaql_migrations`.
///
/// This migration:
/// 1. Detects whether the old monolithic row exists.
/// 2. If so, inserts the 13 individual migration names and deletes the old row.
/// 3. If not, no-ops (fresh install or standalone agent-ssh DB).
///
/// Frozen-list constraint: the INSERT list reflects the 13 migrations
/// that existed when this repair was written. No new agent-ssh migrations
/// may land between writing this repair and shipping the release. See ADR-0005.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let conn = manager.get_connection();

        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        let exists = conn
            .query_one_raw(sea_orm::Statement::from_string(
                manager.get_database_backend(),
                "SELECT 1 FROM seaql_migrations \
                 WHERE version = 'm20260331_000001_ssh_agent_tables' LIMIT 1",
            ))
            .await?
            .is_some();

        if !exists {
            return Ok(());
        }

        // Note on transaction safety: SeaORM's migration runner wraps up() in its own
        // outer transaction before calling this method. A nested BEGIN IMMEDIATE here
        // would open a SAVEPOINT (always deferred in SQLite), not a true BEGIN IMMEDIATE.
        // The actual safety guarantee is ON CONFLICT DO NOTHING: duplicates are silently
        // skipped, and the DELETE is a single-row keyed write. No extra locking needed.
        #[expect(
            clippy::disallowed_methods,
            reason = "builder limitation: bulk INSERT ... VALUES rows call the SQLite unixepoch() function, which sea_query's insert builder cannot embed without the banned Expr::cust"
        )]
        conn.execute_unprepared(
            "INSERT INTO seaql_migrations (version, applied_at) VALUES
               ('m20260215_000001_initial',                     unixepoch()),
               ('m20260222_000002_add_machine_id',              unixepoch()),
               ('m20260224_000003_add_sudo_columns',            unixepoch()),
               ('m20260302_000001_convert_ssh_host_timestamps', unixepoch()),
               ('m20260302_000002_ensure_machine_id_nullable',  unixepoch()),
               ('m20260310_000001_data_encryption_keys',        unixepoch()),
               ('m20260306_000001_add_pve_columns',             unixepoch()),
               ('m20260307_000001_add_pve_node_name',           unixepoch()),
               ('m20260307_000002_pending_proxmox_matches',     unixepoch()),
               ('m20260308_000003_ssh_host_uuid_columns',       unixepoch()),
               ('m20260313_000001_drop_ssh_host_is_pve_node',   unixepoch()),
               ('m20260322_000001_ssh_hosts_lower_name_index',  unixepoch()),
               ('m20260507_000001_add_routeros_host_config',    unixepoch())
             ON CONFLICT DO NOTHING",
        )
        .await?;

        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        conn.execute_unprepared(
            "DELETE FROM seaql_migrations \
             WHERE version = 'm20260331_000001_ssh_agent_tables'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
