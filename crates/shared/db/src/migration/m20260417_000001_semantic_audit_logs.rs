use sea_orm_migration::prelude::*;

use crate::migration::helpers::{self, CrashRecoveryState};

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        recreate_table(
            manager,
            "audit_logs",
            "audit_logs_new",
            build_semantic_audit_logs_table("audit_logs_new"),
        )
        .await?;
        recreate_table(
            manager,
            "system_audit_logs",
            "system_audit_logs_new",
            build_semantic_system_audit_logs_table("system_audit_logs_new"),
        )
        .await?;

        create_indexes(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("system_audit_logs"))
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("audit_logs"))
                    .if_exists()
                    .to_owned(),
            )
            .await
    }
}

fn build_semantic_audit_logs_table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(
            ColumnDef::new(Alias::new("id"))
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(Alias::new("tenant_id")).uuid().not_null())
        .col(
            ColumnDef::new(Alias::new("actor_type"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("actor_id")).uuid())
        .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("action_type"))
                .string_len(128)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
        .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("outcome"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("details_json")).json_binary())
        .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("occurred_at"))
                .timestamp_with_time_zone()
                .not_null(),
        )
        .to_owned()
}

fn build_semantic_system_audit_logs_table(name: &str) -> TableCreateStatement {
    Table::create()
        .table(Alias::new(name))
        .col(
            ColumnDef::new(Alias::new("id"))
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(
            ColumnDef::new(Alias::new("actor_type"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("actor_id")).uuid())
        .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("action_type"))
                .string_len(128)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
        .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
        .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("outcome"))
                .string_len(32)
                .not_null(),
        )
        .col(ColumnDef::new(Alias::new("details_json")).json_binary())
        .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
        .col(
            ColumnDef::new(Alias::new("occurred_at"))
                .timestamp_with_time_zone()
                .not_null(),
        )
        .to_owned()
}

async fn recreate_table(
    manager: &SchemaManager<'_>,
    current_table: &str,
    temp_table: &str,
    create_stmt: TableCreateStatement,
) -> Result<(), DbErr> {
    let state = helpers::check_crash_recovery(manager, current_table, temp_table).await?;
    if state != CrashRecoveryState::RenameOnly {
        manager.create_table(create_stmt).await?;
        helpers::drop_original(manager, current_table).await?;
    }
    helpers::rename_temp(manager, temp_table, current_table).await
}

async fn create_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    for (table, specs) in [
        (
            "audit_logs",
            vec![
                (
                    "idx_audit_logs_tenant_occurred_at",
                    vec!["tenant_id", "occurred_at"],
                ),
                (
                    "idx_audit_logs_tenant_action_type_occurred_at",
                    vec!["tenant_id", "action_type", "occurred_at"],
                ),
                (
                    "idx_audit_logs_tenant_actor_type_occurred_at",
                    vec!["tenant_id", "actor_type", "occurred_at"],
                ),
                (
                    "idx_audit_logs_tenant_actor_id_occurred_at",
                    vec!["tenant_id", "actor_id", "occurred_at"],
                ),
                (
                    "idx_audit_logs_tenant_target_type_occurred_at",
                    vec!["tenant_id", "target_type", "occurred_at"],
                ),
                (
                    "idx_audit_logs_tenant_target_id_occurred_at",
                    vec!["tenant_id", "target_id", "occurred_at"],
                ),
                (
                    "idx_audit_logs_tenant_outcome_occurred_at",
                    vec!["tenant_id", "outcome", "occurred_at"],
                ),
            ],
        ),
        (
            "system_audit_logs",
            vec![
                ("idx_system_audit_logs_occurred_at", vec!["occurred_at"]),
                (
                    "idx_system_audit_logs_action_type_occurred_at",
                    vec!["action_type", "occurred_at"],
                ),
                (
                    "idx_system_audit_logs_actor_type_occurred_at",
                    vec!["actor_type", "occurred_at"],
                ),
                (
                    "idx_system_audit_logs_actor_id_occurred_at",
                    vec!["actor_id", "occurred_at"],
                ),
                (
                    "idx_system_audit_logs_target_type_occurred_at",
                    vec!["target_type", "occurred_at"],
                ),
                (
                    "idx_system_audit_logs_target_id_occurred_at",
                    vec!["target_id", "occurred_at"],
                ),
                (
                    "idx_system_audit_logs_outcome_occurred_at",
                    vec!["outcome", "occurred_at"],
                ),
            ],
        ),
    ] {
        for (name, columns) in specs {
            let mut idx = Index::create();
            idx.table(Alias::new(table)).name(name);
            for column in columns {
                if column == "occurred_at" {
                    idx.col((Alias::new(column), IndexOrder::Desc));
                } else {
                    idx.col(Alias::new(column));
                }
            }
            manager.create_index(idx.to_owned()).await?;
        }
    }
    Ok(())
}
