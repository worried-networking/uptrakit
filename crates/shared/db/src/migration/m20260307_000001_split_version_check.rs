use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── 1. Rename existing version_check tasks to fetch_releases ─────────
        manager
            .exec_stmt(
                Query::update()
                    .table(Alias::new("scheduled_tasks"))
                    .value(Alias::new("task_type"), "fetch_releases")
                    .and_where(Expr::col(Alias::new("task_type")).eq("version_check"))
                    .to_owned(),
            )
            .await?;

        // ── 2. Seed detect_version task for all existing tenants ─────────────
        // Same per-tenant pattern as m20260305_000001_crl_cache.rs.
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
                            "detect_version".into(),
                            "0 0 * * *".into(),
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
        // Remove seeded detect_version tasks.
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("scheduled_tasks"))
                    .and_where(Expr::col(Alias::new("task_type")).eq("detect_version"))
                    .to_owned(),
            )
            .await?;

        // Rename fetch_releases tasks back to version_check.
        manager
            .exec_stmt(
                Query::update()
                    .table(Alias::new("scheduled_tasks"))
                    .value(Alias::new("task_type"), "version_check")
                    .and_where(Expr::col(Alias::new("task_type")).eq("fetch_releases"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
