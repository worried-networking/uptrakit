use sea_orm_migration::prelude::*;

use crate::migration::helpers;
use crate::migration::helpers::{timestamp, timestamp_null};

/// Add execution-owner columns to `update_history`.
///
/// This migration is schema-only. Rollout cleanup of legacy `InProgress`
/// rows is intentionally handled in controller startup code, not here.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if helpers::is_sqlite(manager) {
            self.up_sqlite(manager).await
        } else {
            self.up_alter(manager).await
        }
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if helpers::is_sqlite(manager) {
            self.down_sqlite(manager).await
        } else {
            manager
                .get_connection()
                .execute_unprepared(
                    "ALTER TABLE update_history DROP CONSTRAINT IF EXISTS ck_update_history_owner_pair",
                )
                .await?;

            manager
                .alter_table(
                    Table::alter()
                        .table(UpdateHistory::Table)
                        .drop_column(UpdateHistory::ExecutionOwnerInstanceId)
                        .drop_column(UpdateHistory::ExecutionOwnerServiceId)
                        .to_owned(),
                )
                .await
        }
    }
}

impl Migration {
    async fn up_alter(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(UpdateHistory::Table)
                    .add_column(
                        ColumnDef::new(UpdateHistory::ExecutionOwnerServiceId)
                            .uuid()
                            .null(),
                    )
                    .add_column(
                        ColumnDef::new(UpdateHistory::ExecutionOwnerInstanceId)
                            .uuid()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .get_connection()
            .execute_unprepared(
                r#"
                ALTER TABLE update_history
                ADD CONSTRAINT ck_update_history_owner_pair
                CHECK (
                    execution_owner_instance_id IS NULL
                    OR execution_owner_service_id IS NOT NULL
                )
                "#,
            )
            .await?;

        Ok(())
    }

    async fn up_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;
        let state =
            helpers::check_crash_recovery(manager, "update_history", "update_history_new").await?;

        if state == helpers::CrashRecoveryState::Normal {
            manager
                .create_table(build_update_history_new_table())
                .await?;
            copy_update_history_into_new_table(manager).await?;
            helpers::drop_original(manager, "update_history").await?;
        }

        helpers::rename_temp(manager, "update_history_new", "update_history").await?;
        recreate_update_history_indexes(manager).await?;
        helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }

    async fn down_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;
        let state =
            helpers::check_crash_recovery(manager, "update_history", "update_history_old").await?;

        if state == helpers::CrashRecoveryState::Normal {
            manager
                .create_table(build_update_history_old_table())
                .await?;
            copy_update_history_into_old_table(manager).await?;
            helpers::drop_original(manager, "update_history").await?;
        }

        helpers::rename_temp(manager, "update_history_old", "update_history").await?;
        recreate_update_history_indexes(manager).await?;
        helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }
}

fn build_update_history_new_table() -> TableCreateStatement {
    let mut table = build_update_history_table("update_history_new", true);
    table.check(
        Expr::col(UpdateHistory::ExecutionOwnerInstanceId)
            .is_null()
            .or(Expr::col(UpdateHistory::ExecutionOwnerServiceId).is_not_null()),
    );
    table
}

fn build_update_history_old_table() -> TableCreateStatement {
    build_update_history_table("update_history_old", false)
}

fn build_update_history_table(
    table_name: &'static str,
    with_execution_owner: bool,
) -> TableCreateStatement {
    let mut table = Table::create();
    table
        .table(Alias::new(table_name))
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
        .col(ColumnDef::new(UpdateHistory::FromVersion).string().null())
        .col(ColumnDef::new(UpdateHistory::ToVersion).string().null())
        .col(ColumnDef::new(UpdateHistory::Status).string().not_null())
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
        .col(timestamp_null(UpdateHistory::StartedAt))
        .col(timestamp_null(UpdateHistory::CompletedAt))
        .col(timestamp(UpdateHistory::CreatedAt))
        .col(
            ColumnDef::new(UpdateHistory::UpdateCategory)
                .text()
                .not_null()
                .default("unknown"),
        )
        .col(ColumnDef::new(UpdateHistory::BatchId).uuid().null())
        .col(
            ColumnDef::new(UpdateHistory::Interactive)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(
            ColumnDef::new(UpdateHistory::OutputTruncated)
                .boolean()
                .not_null()
                .default(false),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_tenant")
                .from(Alias::new(table_name), UpdateHistory::TenantId)
                .to(Tenants::Table, Tenants::Id)
                .on_delete(ForeignKeyAction::Restrict),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_host")
                .from(Alias::new(table_name), UpdateHistory::HostId)
                .to(Hosts::Table, Hosts::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_software_item")
                .from(Alias::new(table_name), UpdateHistory::SoftwareItemId)
                .to(SoftwareItems::Table, SoftwareItems::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_host_software_item")
                .from(Alias::new(table_name), UpdateHistory::HostSoftwareItemId)
                .to(HostSoftwareItems::Table, HostSoftwareItems::Id)
                .on_delete(ForeignKeyAction::SetNull),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_update_history_batch_id")
                .from(Alias::new(table_name), UpdateHistory::BatchId)
                .to(UpdateBatches::Table, UpdateBatches::Id)
                .on_delete(ForeignKeyAction::SetNull),
        );

    if with_execution_owner {
        table
            .col(
                ColumnDef::new(UpdateHistory::ExecutionOwnerServiceId)
                    .uuid()
                    .null(),
            )
            .col(
                ColumnDef::new(UpdateHistory::ExecutionOwnerInstanceId)
                    .uuid()
                    .null(),
            );
    }

    table.to_owned()
}

async fn copy_update_history_into_new_table(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .get_connection()
        .execute_unprepared(
            r#"
            INSERT INTO update_history_new (
                id,
                tenant_id,
                host_id,
                software_item_id,
                host_software_item_id,
                from_version,
                to_version,
                status,
                output,
                output_bytes,
                actor_type,
                actor_id,
                started_at,
                completed_at,
                created_at,
                update_category,
                batch_id,
                interactive,
                output_truncated,
                execution_owner_service_id,
                execution_owner_instance_id
            )
            SELECT
                id,
                tenant_id,
                host_id,
                software_item_id,
                host_software_item_id,
                from_version,
                to_version,
                status,
                output,
                output_bytes,
                actor_type,
                actor_id,
                started_at,
                completed_at,
                created_at,
                update_category,
                batch_id,
                interactive,
                output_truncated,
                NULL,
                NULL
            FROM update_history
            "#,
        )
        .await?;
    Ok(())
}

async fn copy_update_history_into_old_table(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .get_connection()
        .execute_unprepared(
            r#"
            INSERT INTO update_history_old (
                id,
                tenant_id,
                host_id,
                software_item_id,
                host_software_item_id,
                from_version,
                to_version,
                status,
                output,
                output_bytes,
                actor_type,
                actor_id,
                started_at,
                completed_at,
                created_at,
                update_category,
                batch_id,
                interactive,
                output_truncated
            )
            SELECT
                id,
                tenant_id,
                host_id,
                software_item_id,
                host_software_item_id,
                from_version,
                to_version,
                status,
                output,
                output_bytes,
                actor_type,
                actor_id,
                started_at,
                completed_at,
                created_at,
                update_category,
                batch_id,
                interactive,
                output_truncated
            FROM update_history
            "#,
        )
        .await?;
    Ok(())
}

async fn recreate_update_history_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
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
        (
            "idx_update_history_host_item_status",
            vec![
                UpdateHistory::HostId,
                UpdateHistory::SoftwareItemId,
                UpdateHistory::Status,
            ],
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

    manager
        .create_index(
            Index::create()
                .name("idx_update_history_host_queued")
                .table(UpdateHistory::Table)
                .col(UpdateHistory::HostId)
                .col(UpdateHistory::Id)
                .and_where(Expr::col(UpdateHistory::Status).eq("queued"))
                .to_owned(),
        )
        .await?;

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
    ExecutionOwnerServiceId,
    ExecutionOwnerInstanceId,
    StartedAt,
    CompletedAt,
    CreatedAt,
    UpdateCategory,
    BatchId,
    Interactive,
    OutputTruncated,
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
enum SoftwareItems {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum HostSoftwareItems {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum UpdateBatches {
    Table,
    Id,
}
