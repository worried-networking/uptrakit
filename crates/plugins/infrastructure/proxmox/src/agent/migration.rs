//! Agent-local database migrations for the Proxmox infrastructure plugin.
//!
//! Contributed to the SSH agent's migration set via the plugin descriptor's
//! `agent_migrations` field (`declare_plugin!` → collected by
//! `uptrakit-agent-ssh-runtime`'s `Migrator::migrations()`).

use sea_orm_migration::prelude::*;

// ── Migration: create proxmox_host_state ─────────────────────────────────────

/// Creates the `proxmox_host_state` table and migrates data from the legacy
/// PVE columns on `ssh_hosts` (if they exist).
pub struct CreateProxmoxHostState;

impl MigrationName for CreateProxmoxHostState {
    fn name(&self) -> &str {
        "m20260308_000001_create_proxmox_host_state"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxHostState {
    async fn up(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        // Create the new table.
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxHostState::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProxmoxHostState::HostId)
                            .string()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::IsPveNode)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::PveNodeName)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::PvePluginConfigId)
                            .string()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::CreatedAt)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxHostState::UpdatedAt)
                            .string()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Migrate data from legacy columns if they exist.
        let db = manager.get_connection();

        // SQLite-specific: check if the old column exists via pragma.
        // query_one_raw with a Statement is the approved exception for raw SQL.
        let has_legacy = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM pragma_table_info('ssh_hosts') WHERE name = 'is_pve_node'",
            ))
            .await?;

        let col_exists = has_legacy
            .as_ref()
            .and_then(|r| r.try_get_by_index::<i32>(0).ok())
            .unwrap_or(0)
            > 0;

        if col_exists {
            let now = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

            // Copy PVE hosts into the new table.
            // SQLite limitation: INSERT...SELECT with sea_query is awkward,
            // so we use a raw parameterised statement.
            db.execute_raw(sea_orm::Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "INSERT OR IGNORE INTO proxmox_host_state \
                 (host_id, is_pve_node, pve_node_name, pve_plugin_config_id, created_at, updated_at) \
                 SELECT id, is_pve_node, pve_node_name, pve_plugin_config_id, $1, $1 \
                 FROM ssh_hosts WHERE is_pve_node = 1",
                [now.into()],
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ProxmoxHostState::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ProxmoxHostState {
    Table,
    HostId,
    IsPveNode,
    PveNodeName,
    PvePluginConfigId,
    CreatedAt,
    UpdatedAt,
    LegacyPveUser,
    NewPvePluginConfigId,
    MigrationAttempts,
}

// ── Migration: create proxmox_pending_matches ────────────────────────────────

/// Creates the `proxmox_pending_matches` table, migrating data from the
/// legacy `pending_proxmox_matches` table (if it exists).
pub struct CreateProxmoxPendingMatches;

impl MigrationName for CreateProxmoxPendingMatches {
    fn name(&self) -> &str {
        "m20260308_000002_create_proxmox_pending_matches"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for CreateProxmoxPendingMatches {
    async fn up(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(ProxmoxPendingMatches::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::Id)
                            .integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::HostId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::MappingId)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(ProxmoxPendingMatches::CreatedAt)
                            .string()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // Migrate data from the legacy table if it exists.
        let db = manager.get_connection();
        let has_legacy = db
            .query_one_raw(sea_orm::Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT COUNT(*) AS cnt FROM sqlite_master WHERE type='table' AND name='pending_proxmox_matches'",
            ))
            .await?;

        let table_exists = has_legacy
            .as_ref()
            .and_then(|r| r.try_get_by_index::<i32>(0).ok())
            .unwrap_or(0)
            > 0;

        if table_exists {
            db.execute_unprepared(
                "INSERT OR IGNORE INTO proxmox_pending_matches (host_id, mapping_id, created_at) \
                 SELECT host_id, mapping_id, created_at FROM pending_proxmox_matches",
            )
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ProxmoxPendingMatches::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ProxmoxPendingMatches {
    Table,
    Id,
    HostId,
    MappingId,
    CreatedAt,
    Attempts,
}

// ── Migration: add attempts to proxmox_pending_matches ──────────────────────

/// Adds the `attempts` retry counter used by the drain's poison-row guard.
pub struct AddPendingMatchAttempts;

impl MigrationName for AddPendingMatchAttempts {
    fn name(&self) -> &str {
        "m20260716_000001_pending_match_attempts"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddPendingMatchAttempts {
    async fn up(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxPendingMatches::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxPendingMatches::Attempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxPendingMatches::Table)
                    .drop_column(ProxmoxPendingMatches::Attempts)
                    .to_owned(),
            )
            .await
    }
}

// ── Migration: PVE identity-migration bookkeeping columns ───────────────────

/// Adds the legacy-user marker, the never-cleared ack-confirmed new-config
/// marker, and the phase-2 attempt counter used by the identity migration.
pub struct AddPveMigrationColumns;

impl MigrationName for AddPveMigrationColumns {
    fn name(&self) -> &str {
        "m20260816_000001_pve_migration_columns"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for AddPveMigrationColumns {
    async fn up(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostState::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxHostState::LegacyPveUser)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostState::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxHostState::NewPvePluginConfigId)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostState::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxHostState::MigrationAttempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        for col in [
            ProxmoxHostState::MigrationAttempts,
            ProxmoxHostState::NewPvePluginConfigId,
            ProxmoxHostState::LegacyPveUser,
        ] {
            manager
                .alter_table(
                    Table::alter()
                        .table(ProxmoxHostState::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }
}

// ── Migration: drop PVE identity-migration bookkeeping columns ──────────────

/// Removes the completed legacy-user migration bookkeeping
/// (ADR-0044 § Migration, executed to completion): folds the
/// `new_pve_plugin_config_id` ack marker into the operative
/// `pve_plugin_config_id`, then drops the three bookkeeping columns.
///
/// The fold is unconditional — no `WHERE`, no `COALESCE` — so it clears
/// `pve_plugin_config_id` on every row where `new_pve_plugin_config_id` is
/// NULL, regardless of why the row got there. Two distinct row shapes reach
/// that state:
///
/// - A `m20260308_000001`-backfilled, never-acked row: a pre-ADR-0044 legacy
///   plugin-config id with no post-ADR-0044 confirmation at all. Clearing it
///   is the fold's actual purpose — the credential flow falls through to
///   create/regenerate, the safe direction.
/// - A promoted-but-never-self-acked peer row: the removed `promote_cluster_rows`
///   helper wrote a shared, controller-acknowledged operative id to every row
///   in a cluster's `host_ids` in one `UPDATE`, but never touched
///   `new_pve_plugin_config_id` on any of them — only the removed
///   `set_new_plugin_config_id`, called for the flow host's own row, ever set
///   that column. So a peer node that never itself completed a
///   post-ADR-0044 credential flow can hold a *legitimate*
///   controller-shared-user `pve_plugin_config_id` with
///   `new_pve_plugin_config_id` NULL, and the fold clears that value too.
///
/// The second case is transient, not lossy: `credential_flow.rs`'s Branch 4
/// evidence scan reads `pve_plugin_config_id` across the whole cluster row
/// set, so as long as one cluster row still carries the id (e.g. an
/// already-acked node), the peer's own next sync re-resolves it and
/// re-persists it onto its own row via `set_plugin_config_id` (Branch 4
/// reuse). Until that next sync, `surface_actions.rs`'s `(node, config_id) ->
/// host_id` map has no entry for the peer, so guest-bootstrap surface
/// actions targeting it are unavailable for one cycle.
pub struct DropPveMigrationColumns;

impl MigrationName for DropPveMigrationColumns {
    fn name(&self) -> &str {
        "m20260817_000001_drop_pve_migration_columns"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for DropPveMigrationColumns {
    async fn up(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        // Ledger/schema-skew guard, per column: a store that skipped
        // m20260816_000001, or a partially applied run of THIS migration
        // (the three ALTER TABLEs are separate statements — an interrupt
        // between them leaves a subset dropped), must no-op per column
        // instead of failing the whole agent migration run on re-entry.
        if manager
            .has_column("proxmox_host_state", "new_pve_plugin_config_id")
            .await?
        {
            manager
                .exec_stmt(
                    Query::update()
                        .table(ProxmoxHostState::Table)
                        .value(
                            ProxmoxHostState::PvePluginConfigId,
                            Expr::col(ProxmoxHostState::NewPvePluginConfigId),
                        )
                        .to_owned(),
                )
                .await?;
        }
        // SQLite allows one alteration per ALTER TABLE.
        for (col, name) in [
            (ProxmoxHostState::MigrationAttempts, "migration_attempts"),
            (
                ProxmoxHostState::NewPvePluginConfigId,
                "new_pve_plugin_config_id",
            ),
            (ProxmoxHostState::LegacyPveUser, "legacy_pve_user"),
        ] {
            if !manager.has_column("proxmox_host_state", name).await? {
                continue;
            }
            manager
                .alter_table(
                    Table::alter()
                        .table(ProxmoxHostState::Table)
                        .drop_column(col)
                        .to_owned(),
                )
                .await?;
        }
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> std::result::Result<(), DbErr> {
        // Columns re-added empty — data not restored, same forward-only
        // posture as the other agent-local down() bodies. Note for anyone
        // adding a step-counted down()/up() round-trip test against this
        // migration: re-adding `new_pve_plugin_config_id` empty here means a
        // subsequent up() folds NULL over every row unconditionally, clearing
        // every `pve_plugin_config_id` in the table — down() then up() is NOT
        // a safe no-op pair for this migration.
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostState::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxHostState::LegacyPveUser)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostState::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxHostState::NewPvePluginConfigId)
                            .string()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;
        manager
            .alter_table(
                Table::alter()
                    .table(ProxmoxHostState::Table)
                    .add_column(
                        ColumnDef::new(ProxmoxHostState::MigrationAttempts)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await
    }
}

/// All agent-local migrations for this plugin, in application order.
pub fn agent_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
    vec![
        Box::new(CreateProxmoxHostState),
        Box::new(CreateProxmoxPendingMatches),
        Box::new(AddPendingMatchAttempts),
        Box::new(AddPveMigrationColumns),
        Box::new(DropPveMigrationColumns),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::entity::proxmox_host_state;
    use sea_orm::{Database, EntityTrait};

    /// One `proxmox_host_state` row shape for the fold-behaviour fixture
    /// below: initial column values plus the expected post-fold
    /// `pve_plugin_config_id`.
    struct FixtureRow {
        host_id: &'static str,
        pve_plugin_config_id: Option<&'static str>,
        new_pve_plugin_config_id: Option<&'static str>,
        legacy_pve_user: Option<&'static str>,
        expected_pve_plugin_config_id: Option<&'static str>,
    }

    /// Drives `AddPveMigrationColumns::up` then `DropPveMigrationColumns::up`
    /// manually against `sqlite::memory:` with a three-row fixture and
    /// asserts the fold's real per-row behaviour — pins Finding 1 of the Task
    /// 4 fix round: only the DROP was previously covered, not the FOLD.
    ///
    /// Each row's initial `pve_plugin_config_id` differs from its asserted
    /// post-fold value, so this goes RED under either a no-op fold (skips
    /// the UPDATE entirely — every row would keep its initial value) or an
    /// "always clear" fold (every row would end up `None`, failing the
    /// "host-acked" assertion).
    #[tokio::test]
    async fn drop_pve_migration_columns_folds_ack_and_clears_unacked_rows() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        let manager = SchemaManager::new(&db);

        CreateProxmoxHostState
            .up(&manager)
            .await
            .expect("create proxmox_host_state");
        AddPveMigrationColumns
            .up(&manager)
            .await
            .expect("add migration bookkeeping columns");

        let now = "2026-08-19T00:00:00Z";

        let fixture = [
            // Acked, but with a stale operative value: `set_new_plugin_config_id`
            // (removed by an earlier round) only backfilled
            // `pve_plugin_config_id` via `COALESCE` when it was NULL, so an
            // already-populated stale value could survive next to a fresh ack
            // marker. The fold must overwrite the stale value with the
            // ack-derived one.
            FixtureRow {
                host_id: "host-acked",
                pve_plugin_config_id: Some("cfg-stale"),
                new_pve_plugin_config_id: Some("cfg-acked"),
                legacy_pve_user: None,
                expected_pve_plugin_config_id: Some("cfg-acked"),
            },
            // Promoted peer: the removed `promote_cluster_rows` helper wrote
            // the shared operative id to every row in a cluster's `host_ids`,
            // including peers that never themselves ran the ack-writing
            // flow — it never touched `new_pve_plugin_config_id`. The fold
            // must clear it (Finding 1's real behaviour).
            FixtureRow {
                host_id: "host-promoted-peer",
                pve_plugin_config_id: Some("cfg-shared"),
                new_pve_plugin_config_id: None,
                legacy_pve_user: None,
                expected_pve_plugin_config_id: None,
            },
            // Backfill-only: `m20260308_000001` copied a pre-ADR-0044 legacy
            // value straight from `ssh_hosts`, never acked. The fold must
            // clear it — this is the fold's actual purpose.
            FixtureRow {
                host_id: "host-backfill-only",
                pve_plugin_config_id: Some("cfg-legacy"),
                new_pve_plugin_config_id: None,
                legacy_pve_user: Some("legacy-user"),
                expected_pve_plugin_config_id: None,
            },
        ];

        for row in &fixture {
            db.execute(
                &Query::insert()
                    .into_table(ProxmoxHostState::Table)
                    .columns([
                        ProxmoxHostState::HostId,
                        ProxmoxHostState::IsPveNode,
                        ProxmoxHostState::PvePluginConfigId,
                        ProxmoxHostState::NewPvePluginConfigId,
                        ProxmoxHostState::LegacyPveUser,
                        ProxmoxHostState::MigrationAttempts,
                        ProxmoxHostState::CreatedAt,
                        ProxmoxHostState::UpdatedAt,
                    ])
                    .values_panic([
                        row.host_id.into(),
                        true.into(),
                        row.pve_plugin_config_id.into(),
                        row.new_pve_plugin_config_id.into(),
                        row.legacy_pve_user.into(),
                        0i32.into(),
                        now.into(),
                        now.into(),
                    ])
                    .to_owned(),
            )
            .await
            .expect("insert host_state fixture row");
        }

        DropPveMigrationColumns
            .up(&manager)
            .await
            .expect("fold and drop migration bookkeeping columns");

        let rows = proxmox_host_state::Entity::find()
            .all(&db)
            .await
            .expect("entity read must decode post-fold rows");

        for row in &fixture {
            let found = rows
                .iter()
                .find(|r| r.host_id == row.host_id)
                .expect("fixture row must survive the migration");
            assert_eq!(
                found.pve_plugin_config_id.as_deref(),
                row.expected_pve_plugin_config_id,
                "unexpected post-fold pve_plugin_config_id for {}",
                row.host_id
            );
        }
    }
}
