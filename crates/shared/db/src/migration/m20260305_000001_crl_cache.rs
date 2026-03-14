use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();

        // ── crl_cache ────────────────────────────────────────────────────────
        // Stores the most recently signed CRL for each CA, keyed by fingerprint.
        // No tenant_id — CRL is global PKI state.
        // No FK constraint — ca_fingerprint is an opaque string identifier,
        // not a DB-level FK, to allow the cache to survive CA record deletion.
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("crl_cache"))
                    .col(
                        ColumnDef::new(Alias::new("ca_fingerprint"))
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Alias::new("crl_pem")).text().not_null())
                    .col(
                        ColumnDef::new(Alias::new("crl_number"))
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("this_update"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("next_update"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("updated_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        // ── Seed crl_renewal scheduled task for all existing tenants ─────────
        // Same pattern as the initial migration task seeding.
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
                            "crl_renewal".into(),
                            "0 */4 * * *".into(),
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
        // Remove seeded crl_renewal tasks
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("scheduled_tasks"))
                    .and_where(Expr::col(Alias::new("task_type")).eq("crl_renewal"))
                    .to_owned(),
            )
            .await?;

        // Drop the crl_cache table
        manager
            .drop_table(Table::drop().table(Alias::new("crl_cache")).to_owned())
            .await?;

        Ok(())
    }
}
