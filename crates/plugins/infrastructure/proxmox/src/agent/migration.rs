//! Agent-local database migrations for the Proxmox infrastructure plugin.
//!
//! These migrations are contributed to the SSH agent's migration set via
//! [`PluginBase::service_migrations()`](uptrakit_plugin_infrastructure_core::PluginBase::service_migrations).

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
}
