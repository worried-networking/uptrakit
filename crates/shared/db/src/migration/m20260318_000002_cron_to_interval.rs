use sea_orm_migration::prelude::*;

use super::helpers::{self, CrashRecoveryState};

/// Replace `cron_expression TEXT` with `interval_seconds INTEGER` + `jitter_seconds INTEGER`
/// in the `scheduled_tasks` table.
///
/// SQLite does not support arbitrary `ALTER TABLE DROP COLUMN`, so this migration uses
/// the standard table-recreation pattern (create new → copy → drop old → rename).
///
/// PostgreSQL uses `ALTER TABLE ADD/DROP COLUMN` directly.
///
/// ## Interval mapping from previous cron expressions
///
/// | task_type              | old cron       | interval_seconds | jitter_seconds |
/// |------------------------|----------------|-----------------|----------------|
/// | auth_cleanup           | `*/5 * * * *`  | 300             | 30             |
/// | stale_lease_cleanup    | `*/5 * * * *`  | 300             | 30             |
/// | ca_rotation_check      | `0 3 * * *`    | 86400           | 300            |
/// | fetch_releases         | `0 */6 * * *`  | 21600           | 300            |
/// | detect_version         | `0 0 * * *`    | 86400           | 300            |
/// | service_cert_check     | `0 */12 * * *` | 43200           | 300            |
/// | crl_renewal            | `0 */4 * * *`  | 14400           | 120            |
/// | audit_log_cleanup      | `0 3 * * *`    | 86400           | 300            |
/// | discover_host_packages | `0 */6 * * *`  | 21600           | 300            |
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Non-generated data columns of the new `scheduled_tasks` schema (without `cron_expression`).
#[derive(Copy, Clone, DeriveIden)]
enum Col {
    Id,
    TenantId,
    TaskType,
    IntervalSeconds,
    JitterSeconds,
    Enabled,
    TaskConfig,
    LastRunAt,
    NextRunAt,
    LockedBy,
    LockedAt,
    LastError,
    RunCount,
    CreatedAt,
    UpdatedAt,
}

#[derive(DeriveIden)]
enum ScheduledTasks {
    Table,
}

#[derive(Clone, DeriveIden)]
enum ScheduledTasksNew {
    Table,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

fn build_new_schema(table_name: impl IntoTableRef + Clone) -> TableCreateStatement {
    Table::create()
        .table(table_name.clone())
        .col(ColumnDef::new(Col::Id).uuid().not_null().primary_key())
        .col(ColumnDef::new(Col::TenantId).uuid().not_null())
        .col(ColumnDef::new(Col::TaskType).text().not_null())
        .col(
            ColumnDef::new(Col::IntervalSeconds)
                .integer()
                .not_null()
                .default(300),
        )
        .col(
            ColumnDef::new(Col::JitterSeconds)
                .integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(Col::Enabled)
                .boolean()
                .not_null()
                .default(true),
        )
        .col(ColumnDef::new(Col::TaskConfig).json_binary().null())
        .col(
            ColumnDef::new(Col::LastRunAt)
                .timestamp_with_time_zone()
                .null(),
        )
        .col(
            ColumnDef::new(Col::NextRunAt)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(ColumnDef::new(Col::LockedBy).uuid())
        .col(
            ColumnDef::new(Col::LockedAt)
                .timestamp_with_time_zone()
                .null(),
        )
        .col(ColumnDef::new(Col::LastError).text())
        .col(
            ColumnDef::new(Col::RunCount)
                .big_integer()
                .not_null()
                .default(0),
        )
        .col(
            ColumnDef::new(Col::CreatedAt)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .col(
            ColumnDef::new(Col::UpdatedAt)
                .timestamp_with_time_zone()
                .not_null(),
        )
        .foreign_key(
            ForeignKey::create()
                .name("fk_scheduled_tasks_tenant")
                .from(table_name, Col::TenantId)
                .to(Tenants::Table, Tenants::Id)
                .on_delete(ForeignKeyAction::Cascade),
        )
        .to_owned()
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        if helpers::is_sqlite(manager) {
            self.up_sqlite(manager).await
        } else {
            self.up_alter(manager).await
        }
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Irreversible: cron expressions cannot be reconstructed from intervals.
        Err(DbErr::Custom(
            "cannot reverse cron-to-interval migration".to_string(),
        ))
    }
}

impl Migration {
    /// SQLite path: table recreation (create new → copy with CASE mapping → drop old → rename).
    async fn up_sqlite(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        helpers::set_foreign_keys(manager, false).await?;

        let state =
            helpers::check_crash_recovery(manager, "scheduled_tasks", "scheduled_tasks_new")
                .await?;

        if state == CrashRecoveryState::Normal {
            // Create the replacement table with the new schema.
            manager
                .create_table(build_new_schema(ScheduledTasksNew::Table))
                .await?;

            // Copy data, mapping cron_expression → interval_seconds/jitter_seconds via CASE.
            //
            // This uses execute_unprepared because sea_query's INSERT…SELECT builder does not
            // support CASE expressions in the SELECT column list. This is a migration-only
            // statement and is the accepted pattern for complex data transformations.
            manager
                .get_connection()
                .execute_unprepared(
                    "INSERT INTO scheduled_tasks_new \
                     (id, tenant_id, task_type, interval_seconds, jitter_seconds, \
                      enabled, task_config, last_run_at, next_run_at, locked_by, \
                      locked_at, last_error, run_count, created_at, updated_at) \
                     SELECT id, tenant_id, task_type, \
                       CASE task_type \
                         WHEN 'auth_cleanup' THEN 300 \
                         WHEN 'stale_lease_cleanup' THEN 300 \
                         WHEN 'ca_rotation_check' THEN 86400 \
                         WHEN 'fetch_releases' THEN 21600 \
                         WHEN 'detect_version' THEN 86400 \
                         WHEN 'service_cert_check' THEN 43200 \
                         WHEN 'crl_renewal' THEN 14400 \
                         WHEN 'audit_log_cleanup' THEN 86400 \
                         WHEN 'discover_host_packages' THEN 21600 \
                         ELSE 300 \
                       END, \
                       CASE task_type \
                         WHEN 'auth_cleanup' THEN 30 \
                         WHEN 'stale_lease_cleanup' THEN 30 \
                         WHEN 'ca_rotation_check' THEN 300 \
                         WHEN 'fetch_releases' THEN 300 \
                         WHEN 'detect_version' THEN 300 \
                         WHEN 'service_cert_check' THEN 300 \
                         WHEN 'crl_renewal' THEN 120 \
                         WHEN 'audit_log_cleanup' THEN 300 \
                         WHEN 'discover_host_packages' THEN 300 \
                         ELSE 30 \
                       END, \
                       enabled, task_config, last_run_at, next_run_at, locked_by, \
                       locked_at, last_error, run_count, created_at, updated_at \
                     FROM scheduled_tasks",
                )
                .await?;

            helpers::drop_original(manager, "scheduled_tasks").await?;
        }

        helpers::rename_temp(manager, "scheduled_tasks_new", "scheduled_tasks").await?;

        self.create_indexes(manager).await?;

        helpers::set_foreign_keys(manager, true).await?;
        Ok(())
    }

    /// PostgreSQL path: ALTER TABLE ADD + UPDATE + DROP.
    async fn up_alter(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        // Add new columns.
        manager
            .alter_table(
                Table::alter()
                    .table(ScheduledTasks::Table)
                    .add_column(
                        ColumnDef::new(Col::IntervalSeconds)
                            .integer()
                            .not_null()
                            .default(300),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(ScheduledTasks::Table)
                    .add_column(
                        ColumnDef::new(Col::JitterSeconds)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .to_owned(),
            )
            .await?;

        // Populate from cron_expression.
        let conn = manager.get_connection();
        let task_mappings: &[(&str, i32, i32)] = &[
            ("auth_cleanup", 300, 30),
            ("stale_lease_cleanup", 300, 30),
            ("ca_rotation_check", 86400, 300),
            ("fetch_releases", 21600, 300),
            ("detect_version", 86400, 300),
            ("service_cert_check", 43200, 300),
            ("crl_renewal", 14400, 120),
            ("audit_log_cleanup", 86400, 300),
            ("discover_host_packages", 21600, 300),
        ];

        for &(task_type, interval, jitter) in task_mappings {
            conn.execute_unprepared(&format!(
                "UPDATE scheduled_tasks SET interval_seconds = {interval}, jitter_seconds = {jitter} \
                 WHERE task_type = '{task_type}'"
            ))
            .await?;
        }

        // Drop old column.
        #[derive(DeriveIden)]
        enum CronCol {
            CronExpression,
        }

        manager
            .alter_table(
                Table::alter()
                    .table(ScheduledTasks::Table)
                    .drop_column(CronCol::CronExpression)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    /// Recreate indexes that existed on the original table.
    async fn create_indexes(&self, manager: &SchemaManager<'_>) -> Result<(), DbErr> {
        manager
            .create_index(
                Index::create()
                    .name("idx_scheduled_tasks_next_run")
                    .table(ScheduledTasks::Table)
                    .col(Col::NextRunAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_scheduled_tasks_tenant_id")
                    .table(ScheduledTasks::Table)
                    .col(Col::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_scheduled_tasks_tenant_task_type")
                    .table(ScheduledTasks::Table)
                    .col(Col::TenantId)
                    .col(Col::TaskType)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
