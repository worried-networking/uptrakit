use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();

        // --- scheduled_tasks ---
        manager
            .create_table(
                Table::create()
                    .table(ScheduledTasks::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(ScheduledTasks::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(ScheduledTasks::TenantId).uuid().not_null())
                    .col(ColumnDef::new(ScheduledTasks::TaskType).text().not_null())
                    .col(string(ScheduledTasks::CronExpression))
                    .col(boolean(ScheduledTasks::Enabled).default(true))
                    .col(json_null(ScheduledTasks::TaskConfig))
                    .col(timestamp_null(ScheduledTasks::LastRunAt))
                    .col(timestamp(ScheduledTasks::NextRunAt))
                    .col(ColumnDef::new(ScheduledTasks::LockedBy).uuid())
                    .col(timestamp_null(ScheduledTasks::LockedAt))
                    .col(ColumnDef::new(ScheduledTasks::LastError).text())
                    .col(big_integer(ScheduledTasks::RunCount).default(0))
                    .col(timestamp(ScheduledTasks::CreatedAt))
                    .col(timestamp(ScheduledTasks::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_scheduled_tasks_tenant")
                            .from(ScheduledTasks::Table, ScheduledTasks::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_scheduled_tasks_next_run")
                    .table(ScheduledTasks::Table)
                    .col(ScheduledTasks::NextRunAt)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_scheduled_tasks_tenant_id")
                    .table(ScheduledTasks::Table)
                    .col(ScheduledTasks::TenantId)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uq_scheduled_tasks_tenant_task_type")
                    .table(ScheduledTasks::Table)
                    .col(ScheduledTasks::TenantId)
                    .col(ScheduledTasks::TaskType)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // Seed default tasks for all existing tenants
        let tasks = [
            ("auth_cleanup", "*/5 * * * *"),
            ("stale_lease_cleanup", "*/5 * * * *"),
            ("event_cleanup", "0 * * * *"),
            ("ca_rotation_check", "0 3 * * *"),
            ("version_check", "0 */6 * * *"),
            ("service_cert_check", "0 */12 * * *"),
        ];

        let db = manager.get_connection();

        // Get all tenant IDs
        let select = sea_orm::sea_query::Query::select()
            .column(Tenants::Id)
            .from(Tenants::Table)
            .to_owned();
        let tenant_rows: Vec<serde_json::Value> = db
            .query_all(&select)
            .await?
            .iter()
            .filter_map(|row| {
                use sea_orm::TryGetable;
                let id: Option<Uuid> = Uuid::try_get_by(row, "id").ok();
                id.map(|id| serde_json::json!(id))
            })
            .collect();

        for tenant_json in &tenant_rows {
            let tenant_id: Uuid = serde_json::from_value(tenant_json.clone())
                .map_err(|e| DbErr::Custom(format!("failed to parse tenant ID: {e}")))?;

            for (task_type, cron_expr) in &tasks {
                manager
                    .exec_stmt(
                        Query::insert()
                            .into_table(ScheduledTasks::Table)
                            .columns([
                                ScheduledTasks::Id,
                                ScheduledTasks::TenantId,
                                ScheduledTasks::TaskType,
                                ScheduledTasks::CronExpression,
                                ScheduledTasks::Enabled,
                                ScheduledTasks::NextRunAt,
                                ScheduledTasks::RunCount,
                                ScheduledTasks::CreatedAt,
                                ScheduledTasks::UpdatedAt,
                            ])
                            .values_panic([
                                Uuid::now_v7().into(),
                                tenant_id.into(),
                                (*task_type).into(),
                                (*cron_expr).into(),
                                true.into(),
                                now.into(),
                                0i64.into(),
                                now.into(),
                                now.into(),
                            ])
                            .to_owned(),
                    )
                    .await?;
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(ScheduledTasks::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum ScheduledTasks {
    Table,
    Id,
    TenantId,
    TaskType,
    CronExpression,
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
enum Tenants {
    Table,
    Id,
}
