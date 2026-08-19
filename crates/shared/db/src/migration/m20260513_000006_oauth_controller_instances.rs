use sea_orm_migration::prelude::*;

use crate::migration::helpers::timestamp;

/// Create the `oauth_controller_instances` table.
///
/// Backs the multi-controller boot guard for the MCP OAuth Authorization
/// Server (Plan A foundation) per
/// `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §24.
///
/// At controller startup (when `oauth.mcp_enabled = true`), the controller
/// writes a row to this table with its `instance_id` (UUID, per-process),
/// `jwt_secret_fingerprint` (HMAC-SHA256 of the signing secret with a static
/// salt — verifies key equality without revealing the secret), `started_at`,
/// and `last_seen_at` (refreshed every 30 s). On boot, the controller scans
/// for any other active rows (`last_seen_at > now - 90s`) and hard-fails if a
/// peer is found with a different fingerprint (token-validation flapping is
/// not acceptable), or warns/fails-by-policy if the fingerprint matches.
///
/// The single index on `last_seen_at` keeps the boot scan cheap regardless
/// of historical row count.
///
/// No foreign keys: rows are self-contained per-process records.
///
/// This table is dormant until the `oauth.mcp_enabled` master switch is set
/// to `true` in a later phase.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(OauthControllerInstances::Table)
                    .col(
                        ColumnDef::new(OauthControllerInstances::InstanceId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OauthControllerInstances::JwtSecretFingerprint)
                            .text()
                            .not_null(),
                    )
                    .col(timestamp(OauthControllerInstances::StartedAt))
                    .col(timestamp(OauthControllerInstances::LastSeenAt))
                    .to_owned(),
            )
            .await?;

        // Single index on last_seen_at supports the boot scan for active peers.
        manager
            .create_index(
                Index::create()
                    .name("idx_oauth_controller_instances_last_seen")
                    .table(OauthControllerInstances::Table)
                    .col(OauthControllerInstances::LastSeenAt)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(OauthControllerInstances::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum OauthControllerInstances {
    Table,
    InstanceId,
    JwtSecretFingerprint,
    StartedAt,
    LastSeenAt,
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;

    use super::Migration;
    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    fn migration_index() -> u32 {
        Migrator::migrations()
            .iter()
            .position(|m| m.name() == "m20260513_000006_oauth_controller_instances")
            .expect("oauth_controller_instances migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, then roll this one back
    /// and re-apply it. Exercises both `up` and `down` paths.
    #[tokio::test]
    async fn oauth_controller_instances_migration_round_trips() {
        let db = test_db().await;
        let index = migration_index();
        // Bring the schema up to and including this migration.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through oauth_controller_instances must apply");

        // Verify the table is present and queryable.
        db.execute(
            &Query::select()
                .expr(Expr::col(Asterisk).count())
                .from(Alias::new("oauth_controller_instances"))
                .to_owned(),
        )
        .await
        .expect("oauth_controller_instances should be queryable after up");

        // Roll back just this migration.
        Migrator::down(&db, Some(1))
            .await
            .expect("oauth_controller_instances migration must roll back cleanly");

        // After down, the table must be gone.
        let res = db
            .execute(
                &Query::select()
                    .expr(Expr::col(Asterisk).count())
                    .from(Alias::new("oauth_controller_instances"))
                    .to_owned(),
            )
            .await;
        assert!(
            res.is_err(),
            "oauth_controller_instances table should be dropped by down"
        );

        // Re-apply to confirm `up` is idempotent across drop/recreate cycles.
        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm the `last_seen_at` index exists after migration.
    #[tokio::test]
    async fn oauth_controller_instances_last_seen_index_present() {
        let db = test_db().await;
        Migrator::up(&db, Some(migration_index() + 1))
            .await
            .expect("migrations should apply");

        let row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("sql"))
                    .from(Alias::new("sqlite_master"))
                    .and_where(Expr::col(Alias::new("type")).eq("index"))
                    .and_where(
                        Expr::col(Alias::new("name"))
                            .eq("idx_oauth_controller_instances_last_seen"),
                    )
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_oauth_controller_instances_last_seen index should exist");

        let sql: String = row
            .try_get::<String>("", "sql")
            .expect("index row should contain SQL");
        assert!(
            sql.contains("last_seen_at"),
            "index must cover last_seen_at; got: {sql}"
        );
    }

    /// Exercise the standalone `down` impl by invoking it on a `Migration`
    /// instance after running everything up. This protects the down path even
    /// when the full Migrator never rolls back in production.
    #[tokio::test]
    async fn down_drops_table() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        let schema_manager = SchemaManager::new(&db);
        Migration.down(&schema_manager).await.expect("down");

        let res = db
            .execute(
                &Query::select()
                    .expr(Expr::col(Asterisk).count())
                    .from(Alias::new("oauth_controller_instances"))
                    .to_owned(),
            )
            .await;
        assert!(
            res.is_err(),
            "oauth_controller_instances table should not exist after down"
        );
    }
}
