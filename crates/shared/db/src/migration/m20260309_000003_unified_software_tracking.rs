use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use crate::migration::helpers;

/// Unify the two parallel software tracking systems (software items + host
/// packages) into a single model.
///
/// ## Changes
///
/// ### `software_items` — table recreation
/// - Remove `enabled` and `discovery_state` columns
/// - Add `featured BOOL NOT NULL DEFAULT false`
///
/// ### `host_software_items` — table recreation
/// - Add `plugin_config_id UUID NULL` (FK → plugin_configs)
/// - Add `package_identifier TEXT NULL`
/// - Add `deactivated_at TIMESTAMP NULL`
///
/// ### `update_history` — table recreation
/// - Add `tenant_id UUID NOT NULL` (FK → tenants)
/// - Add `host_software_item_id UUID NULL` (FK → host_software_items)
/// - Change `to_version` from NOT NULL to NULL
/// - Change `started_at` from NOT NULL to NULL
///
/// ### `update_batches` — table recreation
/// - Add `output TEXT NOT NULL DEFAULT ''`
/// - Add `output_bytes BIGINT NOT NULL DEFAULT 0`
///
/// ### `software_ignores` — new table
/// Replaces both `autodiscovery_ignores` and `host_package_ignores`.
///
/// ### Dropped tables
/// - `host_packages`
/// - `host_package_ignores`
/// - `host_package_update_history`
/// - `autodiscovery_ignores`
///
/// ### Scheduler tasks
/// - Rename `discover_host_packages` → `discover_software`
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// ---------------------------------------------------------------------------
// Iden definitions
// ---------------------------------------------------------------------------

#[derive(DeriveIden)]
enum SoftwareItems {
    Table,
    Id,
    TenantId,
    Name,
    Featured,
    LastCheckedAt,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(Clone, DeriveIden)]
enum SoftwareItemsNew {
    Table,
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    Id,
    HostId,
    SoftwareItemId,
    Qualifier,
    PluginConfigId,
    PackageIdentifier,
    InstalledVersion,
    InstalledVersionDetectedAt,
    LatestVersion,
    LatestVersionFetchedAt,
    LatestReleaseMetadata,
    LastUpdatedAt,
    LinkedAt,
    UpdateCategory,
    DeactivatedAt,
}

#[derive(Clone, DeriveIden)]
enum HostSoftwareItemsNew {
    Table,
}

#[derive(DeriveIden)]
enum UpdateHistory {
    Table,
    Id,
    TenantId,
    HostId,
    SoftwareItemId,
    HostSoftwareItemId,
    FromVersion,
    ToVersion,
    Status,
    Output,
    OutputBytes,
    ActorType,
    ActorId,
    UpdateCategory,
    StartedAt,
    CompletedAt,
    CreatedAt,
    BatchId,
}

#[derive(Clone, DeriveIden)]
enum UpdateHistoryNew {
    Table,
}

#[derive(DeriveIden)]
enum UpdateOutputLines {
    Table,
    UpdateHistoryId,
}

#[derive(DeriveIden)]
enum UpdateBatches {
    Table,
    Id,
    TenantId,
    BatchType,
    Status,
    TotalCount,
    ActorType,
    ActorId,
    Output,
    OutputBytes,
    CreatedAt,
    CompletedAt,
}

#[derive(Clone, DeriveIden)]
enum UpdateBatchesNew {
    Table,
}

#[derive(DeriveIden)]
enum SoftwareIgnores {
    Table,
    Id,
    TenantId,
    HostId,
    Name,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum PluginConfigs {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum ScheduledTasks {
    Table,
    TaskType,
}

// ---------------------------------------------------------------------------
// Migration impl
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        // ── 1. Rebuild software_items ──────────────────────────────────────
        rebuild_software_items(manager).await?;

        // ── 2. Rebuild host_software_items ─────────────────────────────────
        rebuild_host_software_items(manager).await?;

        // ── 3. Rebuild update_history (depends on host_software_items) ─────
        rebuild_update_history(manager).await?;

        // ── 4. Rebuild update_batches ──────────────────────────────────────
        rebuild_update_batches(manager).await?;

        // ── 5. Create software_ignores ─────────────────────────────────────
        create_software_ignores(manager).await?;

        // ── 6. Drop old tables ─────────────────────────────────────────────
        drop_old_tables(manager).await?;

        // ── 7. Rename scheduler task ───────────────────────────────────────
        rename_scheduler_task(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Backwards migration is not supported for this structural change.
        Err(DbErr::Custom(
            "Downward migration for unified software tracking is not supported".into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Step 1: Rebuild software_items
// ---------------------------------------------------------------------------

async fn rebuild_software_items(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let state =
        helpers::check_crash_recovery(manager, "software_items", "software_items_new").await?;

    if state == helpers::CrashRecoveryState::Normal {
        // Create new table with featured column, without enabled/discovery_state.
        manager
            .create_table(
                Table::create()
                    .table(SoftwareItemsNew::Table)
                    .col(
                        ColumnDef::new(SoftwareItems::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(SoftwareItems::TenantId).uuid().not_null())
                    .col(string(SoftwareItems::Name))
                    .col(
                        ColumnDef::new(SoftwareItems::Featured)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(timestamp_null(SoftwareItems::LastCheckedAt))
                    .col(timestamp(SoftwareItems::CreatedAt))
                    .col(timestamp(SoftwareItems::UpdatedAt))
                    .col(timestamp_null(SoftwareItems::DeactivatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_software_items_tenant")
                            .from(SoftwareItemsNew::Table, SoftwareItems::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Copy data: discovery_state=approved or manually created → featured=true,
        // discovery_state=pending → featured=false.
        //
        // SQLite does not support CASE in sea_query INSERT...SELECT well, so we
        // use execute_unprepared for the data copy.
        let copy_sql = if helpers::is_sqlite(manager) {
            "INSERT INTO software_items_new (id, tenant_id, name, featured, last_checked_at, created_at, updated_at, deactivated_at) \
             SELECT id, tenant_id, name, \
                    CASE WHEN discovery_state = 'pending' THEN 0 ELSE 1 END, \
                    last_checked_at, created_at, updated_at, deactivated_at \
             FROM software_items"
        } else {
            "INSERT INTO software_items_new (id, tenant_id, name, featured, last_checked_at, created_at, updated_at, deactivated_at) \
             SELECT id, tenant_id, name, \
                    CASE WHEN discovery_state = 'pending' THEN false ELSE true END, \
                    last_checked_at, created_at, updated_at, deactivated_at \
             FROM software_items"
        };
        manager
            .get_connection()
            .execute_unprepared(copy_sql)
            .await?;

        helpers::drop_original(manager, "software_items").await?;
    }

    helpers::rename_temp(manager, "software_items_new", "software_items").await?;

    // Recreate indexes.
    manager
        .create_index(
            Index::create()
                .name("uq_software_items_active_name")
                .table(SoftwareItems::Table)
                .col(SoftwareItems::TenantId)
                .col(SoftwareItems::Name)
                .unique()
                .and_where(Expr::col(SoftwareItems::DeactivatedAt).is_null())
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_software_items_tenant_id")
                .table(SoftwareItems::Table)
                .col(SoftwareItems::TenantId)
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_software_items_deactivated_at")
                .table(SoftwareItems::Table)
                .col(SoftwareItems::DeactivatedAt)
                .to_owned(),
        )
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Step 2: Rebuild host_software_items
// ---------------------------------------------------------------------------

async fn rebuild_host_software_items(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let state =
        helpers::check_crash_recovery(manager, "host_software_items", "host_software_items_new")
            .await?;

    if state == helpers::CrashRecoveryState::Normal {
        manager
            .create_table(build_hsi_table(HostSoftwareItemsNew::Table))
            .await?;

        // Copy existing data — new columns get defaults (plugin_config_id=NULL,
        // package_identifier=NULL, deactivated_at=NULL).
        let copy_sql = "\
            INSERT INTO host_software_items_new \
                (id, host_id, software_item_id, qualifier, plugin_config_id, package_identifier, \
                 installed_version, installed_version_detected_at, latest_version, \
                 latest_version_fetched_at, latest_release_metadata, last_updated_at, \
                 linked_at, update_category, deactivated_at) \
            SELECT id, host_id, software_item_id, qualifier, NULL, NULL, \
                   installed_version, installed_version_detected_at, latest_version, \
                   latest_version_fetched_at, latest_release_metadata, last_updated_at, \
                   linked_at, update_category, NULL \
            FROM host_software_items";
        manager
            .get_connection()
            .execute_unprepared(copy_sql)
            .await?;

        helpers::drop_original(manager, "host_software_items").await?;
    }

    helpers::rename_temp(manager, "host_software_items_new", "host_software_items").await?;

    // Recreate indexes.
    create_hsi_indexes(manager).await?;

    Ok(())
}

fn build_hsi_table(table_name: impl IntoTableRef + Clone) -> TableCreateStatement {
    Table::create()
        .table(table_name.clone())
        .col(
            ColumnDef::new(HostSoftwareItems::Id)
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(HostSoftwareItems::HostId).uuid().not_null())
        .col(
            ColumnDef::new(HostSoftwareItems::SoftwareItemId)
                .uuid()
                .not_null(),
        )
        .col(ColumnDef::new(HostSoftwareItems::Qualifier).text().null())
        .col(
            ColumnDef::new(HostSoftwareItems::PluginConfigId)
                .uuid()
                .null(),
        )
        .col(
            ColumnDef::new(HostSoftwareItems::PackageIdentifier)
                .text()
                .null(),
        )
        .col(string_null(HostSoftwareItems::InstalledVersion))
        .col(timestamp_null(
            HostSoftwareItems::InstalledVersionDetectedAt,
        ))
        .col(string_null(HostSoftwareItems::LatestVersion))
        .col(timestamp_null(HostSoftwareItems::LatestVersionFetchedAt))
        .col(
            ColumnDef::new(HostSoftwareItems::LatestReleaseMetadata)
                .json_binary()
                .null(),
        )
        .col(timestamp_null(HostSoftwareItems::LastUpdatedAt))
        .col(timestamp(HostSoftwareItems::LinkedAt))
        .col(
            ColumnDef::new(HostSoftwareItems::UpdateCategory)
                .text()
                .not_null()
                .default("unknown"),
        )
        .col(timestamp_null(HostSoftwareItems::DeactivatedAt))
        .foreign_key(
            ForeignKey::create()
                .name("fk_host_software_items_host")
                .from(table_name.clone(), HostSoftwareItems::HostId)
                .to(Hosts::Table, Hosts::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_host_software_items_software_item")
                .from(table_name.clone(), HostSoftwareItems::SoftwareItemId)
                .to(SoftwareItems::Table, SoftwareItems::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_host_software_items_plugin_config")
                .from(table_name, HostSoftwareItems::PluginConfigId)
                .to(PluginConfigs::Table, PluginConfigs::Id)
                .on_delete(ForeignKeyAction::SetNull),
        )
        .to_owned()
}

async fn create_hsi_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // Partial unique: unqualified rows.
    manager
        .create_index(
            Index::create()
                .name("uix_hsi_unqualified")
                .table(HostSoftwareItems::Table)
                .col(HostSoftwareItems::HostId)
                .col(HostSoftwareItems::SoftwareItemId)
                .unique()
                .and_where(Expr::col(HostSoftwareItems::Qualifier).is_null())
                .and_where(Expr::col(HostSoftwareItems::DeactivatedAt).is_null())
                .to_owned(),
        )
        .await?;

    // Partial unique: qualified rows.
    manager
        .create_index(
            Index::create()
                .name("uix_hsi_qualified")
                .table(HostSoftwareItems::Table)
                .col(HostSoftwareItems::HostId)
                .col(HostSoftwareItems::SoftwareItemId)
                .col(HostSoftwareItems::Qualifier)
                .unique()
                .and_where(Expr::col(HostSoftwareItems::Qualifier).is_not_null())
                .and_where(Expr::col(HostSoftwareItems::DeactivatedAt).is_null())
                .to_owned(),
        )
        .await?;

    // Category lookup.
    manager
        .create_index(
            Index::create()
                .name("idx_hsi_host_category")
                .table(HostSoftwareItems::Table)
                .col(HostSoftwareItems::HostId)
                .col(HostSoftwareItems::UpdateCategory)
                .to_owned(),
        )
        .await?;

    // Deactivated_at index for soft-delete queries.
    manager
        .create_index(
            Index::create()
                .name("idx_hsi_deactivated_at")
                .table(HostSoftwareItems::Table)
                .col(HostSoftwareItems::DeactivatedAt)
                .to_owned(),
        )
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Step 3: Rebuild update_history
// ---------------------------------------------------------------------------

async fn rebuild_update_history(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // Drop update_output_lines first (FK dependency).
    if manager.has_table("update_output_lines").await? {
        manager
            .drop_table(Table::drop().table(UpdateOutputLines::Table).to_owned())
            .await?;
    }

    let state =
        helpers::check_crash_recovery(manager, "update_history", "update_history_new").await?;

    if state == helpers::CrashRecoveryState::Normal {
        manager
            .create_table(build_update_history_table(UpdateHistoryNew::Table))
            .await?;

        // Copy data. Derive tenant_id from host → tenant lookup.
        // host_software_item_id is left NULL for all existing rows.
        let copy_sql = "INSERT INTO update_history_new \
                (id, tenant_id, host_id, software_item_id, host_software_item_id, \
                 from_version, to_version, status, output, output_bytes, \
                 actor_type, actor_id, update_category, started_at, completed_at, \
                 created_at, batch_id) \
             SELECT uh.id, h.tenant_id, uh.host_id, uh.software_item_id, NULL, \
                    uh.from_version, uh.to_version, uh.status, uh.output, uh.output_bytes, \
                    uh.actor_type, uh.actor_id, uh.update_category, uh.started_at, \
                    uh.completed_at, uh.created_at, uh.batch_id \
             FROM update_history uh \
             INNER JOIN hosts h ON h.id = uh.host_id";
        manager
            .get_connection()
            .execute_unprepared(copy_sql)
            .await?;

        helpers::drop_original(manager, "update_history").await?;
    }

    helpers::rename_temp(manager, "update_history_new", "update_history").await?;

    // Recreate indexes.
    create_update_history_indexes(manager).await?;

    // Recreate update_output_lines.
    manager
        .create_table(
            Table::create()
                .table(UpdateOutputLines::Table)
                .col(
                    ColumnDef::new(Alias::new("id"))
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(
                    ColumnDef::new(UpdateOutputLines::UpdateHistoryId)
                        .uuid()
                        .not_null(),
                )
                .col(string(Alias::new("stream")))
                .col(ColumnDef::new(Alias::new("output")).text().not_null())
                .col(timestamp(Alias::new("created_at")))
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_update_output_lines_update_history")
                        .from(UpdateOutputLines::Table, UpdateOutputLines::UpdateHistoryId)
                        .to(UpdateHistory::Table, UpdateHistory::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    manager
        .create_index(
            Index::create()
                .name("idx_update_output_lines_update_history")
                .table(UpdateOutputLines::Table)
                .col(UpdateOutputLines::UpdateHistoryId)
                .to_owned(),
        )
        .await?;

    Ok(())
}

fn build_update_history_table(table_name: impl IntoTableRef + Clone) -> TableCreateStatement {
    Table::create()
        .table(table_name.clone())
        .col(
            ColumnDef::new(UpdateHistory::Id)
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(UpdateHistory::TenantId).uuid().not_null())
        .col(ColumnDef::new(UpdateHistory::HostId).uuid().not_null())
        .col(
            ColumnDef::new(UpdateHistory::SoftwareItemId)
                .uuid()
                .not_null(),
        )
        .col(
            ColumnDef::new(UpdateHistory::HostSoftwareItemId)
                .uuid()
                .null(),
        )
        .col(string_null(UpdateHistory::FromVersion))
        .col(string_null(UpdateHistory::ToVersion))
        .col(string(UpdateHistory::Status))
        .col(
            ColumnDef::new(UpdateHistory::Output)
                .text()
                .not_null()
                .default(""),
        )
        .col(
            ColumnDef::new(UpdateHistory::OutputBytes)
                .big_integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(UpdateHistory::ActorType)
                .string()
                .not_null()
                .default("legacy"),
        )
        .col(
            ColumnDef::new(UpdateHistory::ActorId)
                .string()
                .not_null()
                .default(""),
        )
        .col(
            ColumnDef::new(UpdateHistory::UpdateCategory)
                .text()
                .not_null()
                .default("unknown"),
        )
        .col(timestamp_null(UpdateHistory::StartedAt))
        .col(timestamp_null(UpdateHistory::CompletedAt))
        .col(timestamp(UpdateHistory::CreatedAt))
        .col(ColumnDef::new(UpdateHistory::BatchId).uuid().null())
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_tenant")
                .from(table_name.clone(), UpdateHistory::TenantId)
                .to(Tenants::Table, Tenants::Id)
                .on_delete(ForeignKeyAction::Restrict),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_host")
                .from(table_name.clone(), UpdateHistory::HostId)
                .to(Hosts::Table, Hosts::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_software_item")
                .from(table_name.clone(), UpdateHistory::SoftwareItemId)
                .to(SoftwareItems::Table, SoftwareItems::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_host_software_item")
                .from(table_name.clone(), UpdateHistory::HostSoftwareItemId)
                .to(HostSoftwareItems::Table, HostSoftwareItems::Id)
                .on_delete(ForeignKeyAction::SetNull),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_batch_id")
                .from(table_name, UpdateHistory::BatchId)
                .to(UpdateBatches::Table, UpdateBatches::Id)
                .on_delete(ForeignKeyAction::SetNull),
        )
        .to_owned()
}

async fn create_update_history_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for (name, cols) in [
        ("idx_update_history_host_id", vec![UpdateHistory::HostId]),
        (
            "idx_update_history_software_item_id",
            vec![UpdateHistory::SoftwareItemId],
        ),
        ("idx_update_history_status", vec![UpdateHistory::Status]),
        (
            "idx_update_history_host_software_item",
            vec![UpdateHistory::HostId, UpdateHistory::SoftwareItemId],
        ),
        ("idx_uh_batch_id", vec![UpdateHistory::BatchId]),
        (
            "idx_update_history_created_at",
            vec![UpdateHistory::CreatedAt],
        ),
        (
            "idx_update_history_tenant_id",
            vec![UpdateHistory::TenantId],
        ),
    ] {
        let mut idx = Index::create();
        idx.name(name).table(UpdateHistory::Table);
        for c in cols {
            idx.col(c);
        }
        manager.create_index(idx.to_owned()).await?;
    }

    // Partial unique: at most one active update per host.
    manager
        .create_index(
            Index::create()
                .name("uix_update_history_host_active")
                .table(UpdateHistory::Table)
                .col(UpdateHistory::HostId)
                .unique()
                .and_where(Expr::col(UpdateHistory::Status).is_in(["pending", "in_progress"]))
                .to_owned(),
        )
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Step 4: Rebuild update_batches
// ---------------------------------------------------------------------------

async fn rebuild_update_batches(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let state =
        helpers::check_crash_recovery(manager, "update_batches", "update_batches_new").await?;

    if state == helpers::CrashRecoveryState::Normal {
        manager
            .create_table(
                Table::create()
                    .table(UpdateBatchesNew::Table)
                    .col(
                        ColumnDef::new(UpdateBatches::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UpdateBatches::TenantId).uuid().not_null())
                    .col(ColumnDef::new(UpdateBatches::BatchType).text().not_null())
                    .col(
                        ColumnDef::new(UpdateBatches::Status)
                            .text()
                            .not_null()
                            .default("in_progress"),
                    )
                    .col(
                        ColumnDef::new(UpdateBatches::TotalCount)
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(UpdateBatches::ActorType).text().not_null())
                    .col(ColumnDef::new(UpdateBatches::ActorId).text().not_null())
                    .col(
                        ColumnDef::new(UpdateBatches::Output)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .col(
                        ColumnDef::new(UpdateBatches::OutputBytes)
                            .big_integer()
                            .not_null()
                            .default(0),
                    )
                    .col(timestamp(UpdateBatches::CreatedAt))
                    .col(timestamp_null(UpdateBatches::CompletedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_update_batches_tenant_id")
                            .from(UpdateBatchesNew::Table, UpdateBatches::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Copy existing data with empty output defaults.
        let copy_sql = "\
            INSERT INTO update_batches_new \
                (id, tenant_id, batch_type, status, total_count, actor_type, actor_id, \
                 output, output_bytes, created_at, completed_at) \
            SELECT id, tenant_id, batch_type, status, total_count, actor_type, actor_id, \
                   '', 0, created_at, completed_at \
            FROM update_batches";
        manager
            .get_connection()
            .execute_unprepared(copy_sql)
            .await?;

        helpers::drop_original(manager, "update_batches").await?;
    }

    helpers::rename_temp(manager, "update_batches_new", "update_batches").await?;

    // Recreate index.
    manager
        .create_index(
            Index::create()
                .name("idx_ub_tenant_status")
                .table(UpdateBatches::Table)
                .col(UpdateBatches::TenantId)
                .col(UpdateBatches::Status)
                .to_owned(),
        )
        .await?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Step 5: Create software_ignores
// ---------------------------------------------------------------------------

async fn create_software_ignores(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    if manager.has_table("software_ignores").await? {
        return Ok(());
    }

    manager
        .create_table(
            Table::create()
                .table(SoftwareIgnores::Table)
                .col(
                    ColumnDef::new(SoftwareIgnores::Id)
                        .uuid()
                        .not_null()
                        .primary_key(),
                )
                .col(ColumnDef::new(SoftwareIgnores::TenantId).uuid().not_null())
                .col(ColumnDef::new(SoftwareIgnores::HostId).uuid().null())
                .col(string(SoftwareIgnores::Name))
                .col(timestamp(SoftwareIgnores::CreatedAt))
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_software_ignores_tenant")
                        .from(SoftwareIgnores::Table, SoftwareIgnores::TenantId)
                        .to(Tenants::Table, Tenants::Id)
                        .on_delete(ForeignKeyAction::Restrict),
                )
                .foreign_key(
                    ForeignKey::create()
                        .name("fk_software_ignores_host")
                        .from(SoftwareIgnores::Table, SoftwareIgnores::HostId)
                        .to(Hosts::Table, Hosts::Id)
                        .on_delete(ForeignKeyAction::Cascade),
                )
                .to_owned(),
        )
        .await?;

    // Unique: (tenant_id, name) where host_id is NULL (tenant-wide).
    manager
        .create_index(
            Index::create()
                .name("uix_si_tenant_name")
                .table(SoftwareIgnores::Table)
                .col(SoftwareIgnores::TenantId)
                .col(SoftwareIgnores::Name)
                .unique()
                .and_where(Expr::col(SoftwareIgnores::HostId).is_null())
                .to_owned(),
        )
        .await?;

    // Unique: (tenant_id, host_id, name) where host_id is NOT NULL (per-host).
    manager
        .create_index(
            Index::create()
                .name("uix_si_tenant_host_name")
                .table(SoftwareIgnores::Table)
                .col(SoftwareIgnores::TenantId)
                .col(SoftwareIgnores::HostId)
                .col(SoftwareIgnores::Name)
                .unique()
                .and_where(Expr::col(SoftwareIgnores::HostId).is_not_null())
                .to_owned(),
        )
        .await?;

    // Migrate existing autodiscovery_ignores → software_ignores (tenant-wide).
    if manager.has_table("autodiscovery_ignores").await? {
        let migrate_sql = "\
            INSERT INTO software_ignores (id, tenant_id, host_id, name, created_at) \
            SELECT id, tenant_id, NULL, name, created_at \
            FROM autodiscovery_ignores";
        manager
            .get_connection()
            .execute_unprepared(migrate_sql)
            .await?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Step 6: Drop old tables
// ---------------------------------------------------------------------------

async fn drop_old_tables(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // Drop in FK-dependency order.
    for table in [
        "host_package_update_history",
        "host_package_ignores",
        "host_packages",
        "autodiscovery_ignores",
    ] {
        if manager.has_table(table).await? {
            manager
                .drop_table(Table::drop().table(Alias::new(table)).to_owned())
                .await?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Step 7: Rename scheduler task
// ---------------------------------------------------------------------------

async fn rename_scheduler_task(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    let update_stmt = Query::update()
        .table(ScheduledTasks::Table)
        .value(ScheduledTasks::TaskType, "discover_software")
        .and_where(Expr::col(ScheduledTasks::TaskType).eq("discover_host_packages"))
        .to_owned();
    manager.get_connection().execute(&update_stmt).await?;

    Ok(())
}
