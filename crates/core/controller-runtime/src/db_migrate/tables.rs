//! Database data migration — orchestrator over core tables (in
//! `shared-db::migrate_core_tables`) and plugin tables (registered via
//! `PluginDescriptor::db_migrate_tables`).

use rootcause::prelude::*;
use sea_orm::DatabaseConnection;

use super::error::Result;

#[cfg(test)]
pub(crate) use uptrakit_shared_db::migrate_core_tables::core_tables;

pub(crate) async fn copy_all(
    src: &DatabaseConnection,
    dst: &DatabaseConnection,
    batch_size: u64,
) -> Result<u64> {
    let mut total = uptrakit_shared_db::migrate_core_tables::copy(src, dst, batch_size)
        .await
        .context_to()?;
    total += uptrakit_plugin_infrastructure_registry::copy_plugin_tables(src, dst, batch_size)
        .await
        .context_to()?;
    Ok(total)
}

pub(crate) async fn clean_all(dst: &DatabaseConnection) -> Result<()> {
    // Plugin tables first (FK leaves of the core graph).
    uptrakit_plugin_infrastructure_registry::clean_plugin_tables(dst)
        .await
        .context_to()?;
    uptrakit_shared_db::migrate_core_tables::clean(dst)
        .await
        .context_to()?;
    Ok(())
}

pub(crate) async fn verify_all(src: &DatabaseConnection, dst: &DatabaseConnection) -> Result<u64> {
    let mut total = uptrakit_shared_db::migrate_core_tables::verify(src, dst)
        .await
        .context_to()?;
    total += uptrakit_plugin_infrastructure_registry::verify_plugin_tables(src, dst)
        .await
        .context_to()?;
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Standing drift gate: runs on every `cargo test` (no longer ignored) — the
    /// coverage list cannot silently drift on any path that runs tests.
    ///
    /// Every live application table (after running migrations) must be
    /// covered by either `core_tables()` (core tables), a registered plugin's
    /// `db_migrate_tables` entry, or the explicit `AGENT_ONLY_TABLES` exclusion
    /// list below.
    ///
    /// `AGENT_ONLY_TABLES` are created by controller-side migrations so agents
    /// can use them without running their own schema setup, but the tables hold
    /// transient agent-local state that is NOT copied during `db-migrate`
    /// (they are re-populated by agents after each migration).
    ///
    /// Failure modes caught:
    /// - New entity migration without registering the table for db-migrate.
    /// - Stale entry in `core_tables()` or a plugin descriptor pointing at a
    ///   dropped table.
    #[tokio::test]
    async fn migration_coverage_complete() {
        use sea_orm::sea_query::{Alias, Expr, ExprTrait, Query};
        use sea_orm::{ConnectOptions, ConnectionTrait, Database, TryGetable as _};
        use std::collections::HashSet;

        /// Tables created by migrations but intentionally excluded from db-migrate.
        /// These hold transient agent-local state that agents re-populate after migration.
        ///
        /// Two families, both agent-local: the Proxmox plugin's embedded-agent tables
        /// (`proxmox_host_state`, `proxmox_pending_matches`) and the agent-ssh-runtime
        /// embedded-agent tables (`ssh_hosts` and its FK children `pending_proxmox_matches`
        /// and `routeros_host_config`). Note `proxmox_pending_matches` (Proxmox plugin) and
        /// `pending_proxmox_matches` (agent-ssh-runtime) are distinct tables despite the
        /// near-identical names.
        const AGENT_ONLY_TABLES: &[&str] = &[
            "ssh_hosts",
            "proxmox_host_state",
            "proxmox_pending_matches",
            "pending_proxmox_matches",
            "routeros_host_config",
        ];

        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("source db");
        crate::migration::run_migrations(&db)
            .await
            .expect("source migrations");

        // `sqlite_master` has no SeaORM entity to build against, so it is
        // addressed via `Alias` rather than `Entity::find`.
        let select = Query::select()
            .column(Alias::new("name"))
            .from(Alias::new("sqlite_master"))
            .and_where(Expr::col(Alias::new("type")).eq("table"))
            .and_where(Expr::col(Alias::new("name")).not_like("sqlite_%"))
            .and_where(Expr::col(Alias::new("name")).ne("seaql_migrations"))
            .to_owned();
        let live: HashSet<String> = db
            .query_all(&select)
            .await
            .expect("query live tables")
            .into_iter()
            .map(|row| String::try_get(&row, "", "name").expect("name"))
            .filter(|name| !AGENT_ONLY_TABLES.contains(&name.as_str()))
            .collect();

        let mut covered: HashSet<String> =
            core_tables().iter().map(|d| d.name.to_owned()).collect();
        for descriptor in uptrakit_plugin_infrastructure_registry::all_descriptors() {
            if let Some(tables_fn) = descriptor.db_migrate_tables {
                for td in tables_fn() {
                    covered.insert(td.name.to_owned());
                }
            }
        }

        let missing: Vec<_> = live.difference(&covered).cloned().collect();
        let extra: Vec<_> = covered.difference(&live).cloned().collect();

        assert!(
            missing.is_empty() && extra.is_empty(),
            "schema drift between migrations and db-migrate coverage:\n  \
             missing from coverage: {missing:?}\n  \
             extra in lists: {extra:?}"
        );
    }
}
