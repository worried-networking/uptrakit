use sea_orm_migration::prelude::*;

use crate::migration::helpers::{timestamp, timestamp_null};

/// Create the `oauth_authorization_codes` table.
///
/// Backs single-use authorization codes for the MCP OAuth Authorization
/// Server (Plan A foundation) per
/// `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §16.
///
/// Each row represents an opaque, hashed authorization code minted at consent
/// time and redeemed once at `/oauth/token` for an access + refresh token
/// pair. Codes have a default 30-second TTL (matches OAuth 2.1 §4.1 ≤ 60 s
/// recommendation) and the `consumed_at` timestamp enforces single-use.
///
/// A partial index on `code_hash WHERE consumed_at IS NULL` keeps the
/// hot-path redemption lookup tight.
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
                    .table(OauthAuthorizationCodes::Table)
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::CodeHash)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::RequestId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::ClientId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::UserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::RedirectUri)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::Scope)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::CodeChallenge)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::CodeChallengeMethod)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationCodes::Resource)
                            .text()
                            .not_null(),
                    )
                    .col(timestamp(OauthAuthorizationCodes::IssuedAt))
                    .col(timestamp(OauthAuthorizationCodes::ExpiresAt))
                    .col(timestamp_null(OauthAuthorizationCodes::ConsumedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_authorization_codes_request")
                            .from(
                                OauthAuthorizationCodes::Table,
                                OauthAuthorizationCodes::RequestId,
                            )
                            .to(
                                OauthAuthorizationRequests::Table,
                                OauthAuthorizationRequests::RequestId,
                            )
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_authorization_codes_client")
                            .from(
                                OauthAuthorizationCodes::Table,
                                OauthAuthorizationCodes::ClientId,
                            )
                            .to(OauthClients::Table, OauthClients::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_authorization_codes_user")
                            .from(
                                OauthAuthorizationCodes::Table,
                                OauthAuthorizationCodes::UserId,
                            )
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Partial index on un-consumed codes for the hot-path redemption lookup.
        //
        // sea_query does not support `WHERE` clauses on `CREATE INDEX`, so use
        // raw SQL. SQLite and PostgreSQL both support partial indexes with
        // identical `WHERE column IS NULL` syntax, so a single statement covers
        // both backends.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_oauth_authorization_codes_unconsumed \
                 ON oauth_authorization_codes (code_hash) \
                 WHERE consumed_at IS NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(OauthAuthorizationCodes::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum OauthAuthorizationCodes {
    Table,
    Id,
    CodeHash,
    RequestId,
    ClientId,
    UserId,
    RedirectUri,
    Scope,
    CodeChallenge,
    CodeChallengeMethod,
    Resource,
    IssuedAt,
    ExpiresAt,
    ConsumedAt,
}

#[derive(DeriveIden)]
enum OauthAuthorizationRequests {
    Table,
    RequestId,
}

#[derive(DeriveIden)]
enum OauthClients {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
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
            .position(|m| m.name() == "m20260512_000004_oauth_authorization_codes")
            .expect("oauth_authorization_codes migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, then roll this one back
    /// and re-apply it. Exercises both `up` and `down` paths.
    #[tokio::test]
    async fn oauth_authorization_codes_migration_round_trips() {
        let db = test_db().await;
        let index = migration_index();
        // Bring the schema up to and including this migration.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through oauth_authorization_codes must apply");

        // Verify the table is present and queryable.
        db.execute_unprepared("SELECT COUNT(*) FROM oauth_authorization_codes")
            .await
            .expect("oauth_authorization_codes should be queryable after up");

        // Roll back just this migration.
        Migrator::down(&db, Some(1))
            .await
            .expect("oauth_authorization_codes migration must roll back cleanly");

        // After down, the table must be gone.
        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM oauth_authorization_codes")
            .await;
        assert!(
            res.is_err(),
            "oauth_authorization_codes table should be dropped by down"
        );

        // Re-apply to confirm `up` is idempotent across drop/recreate cycles.
        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm the partial index exists after migration and contains the
    /// `WHERE consumed_at IS NULL` clause.
    #[tokio::test]
    async fn oauth_authorization_codes_unconsumed_index_is_partial() {
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
                            .eq("idx_oauth_authorization_codes_unconsumed"),
                    )
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_oauth_authorization_codes_unconsumed index should exist");

        let sql: String = row
            .try_get::<String>("", "sql")
            .expect("index row should contain SQL");
        assert!(
            sql.contains("WHERE consumed_at IS NULL"),
            "index must be partial on consumed_at IS NULL; got: {sql}"
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
            .execute_unprepared("SELECT COUNT(*) FROM oauth_authorization_codes")
            .await;
        assert!(
            res.is_err(),
            "oauth_authorization_codes table should not exist after down"
        );
    }
}
