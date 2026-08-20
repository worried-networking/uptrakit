use sea_orm_migration::prelude::*;

/// Snapshot `redirect_uri` and `return_origin` on `pending_oidc_flows`.
///
/// Plan B pins the OIDC `redirect_uri` to the canonical host and snapshots
/// it (plus the originating `return_origin`) on the pending-flow row so the
/// callback replays the values recorded at authorize time instead of
/// re-deriving them. Both columns are added `NOT NULL` with no sensible
/// default for existing rows, so `up()` first purges the table: pending
/// flows live at most `TTL_SECONDS` (600s, `oidc_state.rs`) before they
/// expire, so any row present at migration time is either already stale or
/// about to become so — and it cannot satisfy the new NOT NULL columns
/// regardless. Purging is strictly kinder than leaving it to serve a
/// broken callback later. The empty-string column default is kept anyway
/// (SQLite requires one for a `NOT NULL` add on a non-empty table, and it
/// keeps behavior aligned with Postgres, which allows the add without a
/// default only because the table is provably empty at this point).
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(PendingOidcFlows::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(PendingOidcFlows::Table)
                    .add_column(
                        ColumnDef::new(PendingOidcFlows::RedirectUri)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(PendingOidcFlows::Table)
                    .add_column(
                        ColumnDef::new(PendingOidcFlows::ReturnOrigin)
                            .text()
                            .not_null()
                            .default(""),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(PendingOidcFlows::Table)
                    .drop_column(PendingOidcFlows::RedirectUri)
                    .to_owned(),
            )
            .await?;

        manager
            .alter_table(
                Table::alter()
                    .table(PendingOidcFlows::Table)
                    .drop_column(PendingOidcFlows::ReturnOrigin)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum PendingOidcFlows {
    Table,
    RedirectUri,
    ReturnOrigin,
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;
    use uuid::Uuid;

    use super::Migration;
    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    /// Insert a row shaped like a pre-migration `pending_oidc_flows` row
    /// (no `redirect_uri`/`return_origin` columns yet).
    async fn seed_legacy_flow(db: &DatabaseConnection, csrf_state: &str) {
        db.execute(
            &Query::insert()
                .into_table(Alias::new("pending_oidc_flows"))
                .columns([
                    Alias::new("csrf_state"),
                    Alias::new("provider_id"),
                    Alias::new("pkce_verifier"),
                    Alias::new("nonce"),
                    Alias::new("created_at"),
                    Alias::new("expires_at"),
                ])
                .values_panic([
                    csrf_state.into(),
                    Uuid::nil().to_string().into(),
                    "verifier".into(),
                    "nonce".into(),
                    "2026-01-01T00:00:00Z".into(),
                    "2026-01-01T00:00:00Z".into(),
                ])
                .to_owned(),
        )
        .await
        .expect("seed legacy pending_oidc_flows row");
    }

    async fn row_count(db: &DatabaseConnection) -> i64 {
        db.query_one(
            &Query::select()
                .expr_as(
                    Expr::col(Alias::new("csrf_state")).count(),
                    Alias::new("cnt"),
                )
                .from(Alias::new("pending_oidc_flows"))
                .to_owned(),
        )
        .await
        .expect("count query")
        .expect("count row")
        .try_get::<i64>("", "cnt")
        .expect("cnt column")
    }

    /// `(column name, notnull flag)` pairs from SQLite's `pragma_table_info`.
    async fn column_info(db: &DatabaseConnection) -> Vec<(String, i32)> {
        let stmt = Query::select()
            .columns([Alias::new("name"), Alias::new("notnull")])
            .from_function(
                Func::cust(Alias::new("pragma_table_info")).arg(Expr::val("pending_oidc_flows")),
                Alias::new("table_info"),
            )
            .to_owned();
        db.query_all(&stmt)
            .await
            .expect("pragma table_info")
            .into_iter()
            .map(|row| {
                (
                    row.try_get::<String>("", "name").expect("name"),
                    row.try_get::<i32>("", "notnull").expect("notnull"),
                )
            })
            .collect()
    }

    #[tokio::test]
    async fn up_purges_existing_rows_and_adds_not_null_columns() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("run full migrator");

        // Roll this migration back to the pre-migration schema, then seed a
        // row shaped like one that predates the migration — mirroring a
        // pending flow that existed when a deploy picked this migration up.
        let schema_manager = SchemaManager::new(&db);
        Migration
            .down(&schema_manager)
            .await
            .expect("revert to legacy schema");
        seed_legacy_flow(&db, "csrf-legacy").await;
        assert_eq!(
            row_count(&db).await,
            1,
            "row must be seeded before this migration reruns"
        );

        Migration.up(&schema_manager).await.expect("up");

        assert_eq!(
            row_count(&db).await,
            0,
            "pre-existing rows must be purged by up()"
        );

        let columns = column_info(&db).await;
        for name in ["redirect_uri", "return_origin"] {
            let (_, notnull) = columns
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("column {name} must exist; got {columns:?}"));
            assert_eq!(*notnull, 1, "{name} must be NOT NULL; got {columns:?}");
        }
    }

    #[tokio::test]
    async fn down_drops_both_columns() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("run full migrator");

        let schema_manager = SchemaManager::new(&db);
        Migration.down(&schema_manager).await.expect("down");

        let columns = column_info(&db).await;
        assert!(
            !columns
                .iter()
                .any(|(n, _)| n == "redirect_uri" || n == "return_origin"),
            "down must drop both columns; got {columns:?}"
        );
    }
}
