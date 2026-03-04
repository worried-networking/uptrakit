use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Seed a `discover_host_packages` scheduled task for every existing tenant.
        // New tenants receive this task via the initial tenant-seeding path.
        // The task runs every 6 hours and triggers a full host-package rediscovery
        // for all active hosts, allowing the system to detect newly installed or
        // removed packages without waiting for a host re-registration event.
        let now = time::OffsetDateTime::now_utc();

        let tenant_id_select = Query::select()
            .column(Alias::new("id"))
            .from(Alias::new("tenants"))
            .to_owned();

        let db = manager.get_connection();
        let tenant_rows = db.query_all(&tenant_id_select).await?;

        for tenant_row in &tenant_rows {
            use sea_orm::TryGetable;
            let tenant_id: Uuid = Uuid::try_get_by(tenant_row, "id")
                .map_err(|e| DbErr::Custom(format!("failed to get tenant ID: {e:?}")))?;

            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Alias::new("scheduled_tasks"))
                        .columns([
                            Alias::new("id"),
                            Alias::new("tenant_id"),
                            Alias::new("task_type"),
                            Alias::new("cron_expression"),
                            Alias::new("enabled"),
                            Alias::new("next_run_at"),
                            Alias::new("run_count"),
                            Alias::new("created_at"),
                            Alias::new("updated_at"),
                        ])
                        .values_panic([
                            Uuid::now_v7().into(),
                            tenant_id.into(),
                            "discover_host_packages".into(),
                            "0 */6 * * *".into(),
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

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("scheduled_tasks"))
                    .and_where(Expr::col(Alias::new("task_type")).eq("discover_host_packages"))
                    .to_owned(),
            )
            .await
    }
}
