use sea_orm_migration::prelude::*;

/// Create `audit_logs` and `system_audit_logs` tables for audit trail, plus
/// seed the `audit_log_cleanup` scheduled task (disabled by default).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── audit_logs (tenant-scoped) ──────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("audit_logs"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("tenant_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("actor_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("actor_type"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("auth_method"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("http_method"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("http_path"))
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("route_pattern")).text())
                    .col(
                        ColumnDef::new(Alias::new("http_status"))
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("client_ip")).string())
                    .col(ColumnDef::new(Alias::new("user_agent")).text())
                    .col(
                        ColumnDef::new(Alias::new("duration_ms"))
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("occurred_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    // No FK constraint on tenant_id — audit records must survive
                    // tenant deletion for compliance.
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("audit_logs"))
                    .name("idx_audit_logs_tenant_occurred_at")
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("occurred_at"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("audit_logs"))
                    .name("idx_audit_logs_actor_id")
                    .col(Alias::new("actor_id"))
                    .to_owned(),
            )
            .await?;

        // ── system_audit_logs (global, no tenant_id) ────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("system_audit_logs"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("actor_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("actor_type"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("auth_method"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("http_method"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("http_path"))
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("route_pattern")).text())
                    .col(
                        ColumnDef::new(Alias::new("http_status"))
                            .integer()
                            .not_null(),
                    )
                    .col(ColumnDef::new(Alias::new("client_ip")).string())
                    .col(ColumnDef::new(Alias::new("user_agent")).text())
                    .col(
                        ColumnDef::new(Alias::new("duration_ms"))
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("occurred_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("system_audit_logs"))
                    .name("idx_system_audit_logs_occurred_at")
                    .col(Alias::new("occurred_at"))
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("system_audit_logs"))
                    .name("idx_system_audit_logs_actor_id")
                    .col(Alias::new("actor_id"))
                    .to_owned(),
            )
            .await?;

        // ── Seed audit_log_cleanup scheduled task (disabled by default) ─────
        // Find the default tenant ID from the `tenants` table.
        let db = manager.get_connection();
        let tenant_rows = db
            .query_all(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("tenants"))
                    .limit(1)
                    .to_owned(),
            )
            .await?;

        if let Some(row) = tenant_rows.first() {
            use sea_orm::TryGetable;
            let tenant_id: uuid::Uuid = uuid::Uuid::try_get_by_index(row, 0)
                .map_err(|e| DbErr::Custom(format!("failed to read tenant_id: {e:?}")))?;

            let task_id = uuid::Uuid::now_v7();
            let now = time::OffsetDateTime::now_utc();
            // Calculate next run: 03:00 UTC tomorrow.
            let tomorrow = now.date() + time::Duration::days(1);
            let next_run = tomorrow
                .with_hms(3, 0, 0)
                .expect("03:00:00 is valid")
                .assume_utc();

            db.execute(
                &Query::insert()
                    .into_table(Alias::new("scheduled_tasks"))
                    .columns([
                        Alias::new("id"),
                        Alias::new("tenant_id"),
                        Alias::new("task_type"),
                        Alias::new("cron_expression"),
                        Alias::new("enabled"),
                        Alias::new("last_run_at"),
                        Alias::new("next_run_at"),
                        Alias::new("locked_by"),
                        Alias::new("locked_at"),
                        Alias::new("last_error"),
                        Alias::new("run_count"),
                        Alias::new("created_at"),
                        Alias::new("updated_at"),
                    ])
                    .values_panic([
                        task_id.into(),
                        tenant_id.into(),
                        "audit_log_cleanup".into(),
                        "0 3 * * *".into(),
                        false.into(),               // disabled by default
                        Option::<String>::None.into(), // last_run_at
                        next_run.into(),
                        Option::<uuid::Uuid>::None.into(), // locked_by
                        Option::<String>::None.into(),     // locked_at
                        Option::<String>::None.into(),     // last_error
                        0i64.into(),                       // run_count
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
        // Remove audit_log_cleanup scheduled task.
        let db = manager.get_connection();
        db.execute(
            &Query::delete()
                .from_table(Alias::new("scheduled_tasks"))
                .and_where(Expr::col(Alias::new("task_type")).eq("audit_log_cleanup"))
                .to_owned(),
        )
        .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("system_audit_logs"))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("audit_logs"))
                    .to_owned(),
            )
            .await
    }
}
