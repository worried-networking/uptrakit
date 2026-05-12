use sea_orm_migration::prelude::*;

use crate::migration::helpers::{timestamp, timestamp_null};

/// Create the `oauth_authorization_requests` table.
///
/// Backs the in-flight server-side state for the MCP OAuth Authorization
/// Server consent flow (Plan A foundation) per
/// `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §12.2.
///
/// Each row represents a pending authorization request whose parameters
/// (redirect_uri, scope, state, PKCE challenge, resource) are stored
/// server-side so the consent screen only needs to carry `request_id`. This
/// avoids re-passing OAuth parameters through the frontend (which would risk
/// Referer leakage) and lets the AS validate the authorize request exactly
/// once.
///
/// A partial index on `consumed_at IS NULL` keeps lookups for active
/// (un-consumed) requests fast.
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
                    .table(OauthAuthorizationRequests::Table)
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::RequestId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::ClientId)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::UserId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::RedirectUri)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::Scope)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::State)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::CodeChallenge)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::CodeChallengeMethod)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthAuthorizationRequests::Resource)
                            .text()
                            .not_null(),
                    )
                    .col(timestamp(OauthAuthorizationRequests::CreatedAt))
                    .col(timestamp(OauthAuthorizationRequests::ExpiresAt))
                    .col(timestamp_null(OauthAuthorizationRequests::ConsumedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_authorization_requests_client")
                            .from(
                                OauthAuthorizationRequests::Table,
                                OauthAuthorizationRequests::ClientId,
                            )
                            .to(OauthClients::Table, OauthClients::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_authorization_requests_user")
                            .from(
                                OauthAuthorizationRequests::Table,
                                OauthAuthorizationRequests::UserId,
                            )
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Partial index on pending (un-consumed) requests.
        //
        // sea_query does not support `WHERE` clauses on `CREATE INDEX`, so use
        // raw SQL. SQLite and PostgreSQL both support partial indexes with
        // identical `WHERE column IS NULL` syntax, so a single statement covers
        // both backends.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_oauth_authorization_requests_pending \
                 ON oauth_authorization_requests (request_id) \
                 WHERE consumed_at IS NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(OauthAuthorizationRequests::Table)
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum OauthAuthorizationRequests {
    Table,
    RequestId,
    ClientId,
    UserId,
    RedirectUri,
    Scope,
    State,
    CodeChallenge,
    CodeChallengeMethod,
    Resource,
    CreatedAt,
    ExpiresAt,
    ConsumedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum OauthClients {
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
            .position(|m| m.name() == "m20260512_000003_oauth_authorization_requests")
            .expect("oauth_authorization_requests migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, then roll this one back
    /// and re-apply it. Exercises both `up` and `down` paths.
    #[tokio::test]
    async fn oauth_authorization_requests_migration_round_trips() {
        let db = test_db().await;
        let index = migration_index();
        // Bring the schema up to and including this migration.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through oauth_authorization_requests must apply");

        // Verify the table is present and queryable.
        db.execute_unprepared("SELECT COUNT(*) FROM oauth_authorization_requests")
            .await
            .expect("oauth_authorization_requests should be queryable after up");

        // Roll back just this migration.
        Migrator::down(&db, Some(1))
            .await
            .expect("oauth_authorization_requests migration must roll back cleanly");

        // After down, the table must be gone.
        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM oauth_authorization_requests")
            .await;
        assert!(
            res.is_err(),
            "oauth_authorization_requests table should be dropped by down"
        );

        // Re-apply to confirm `up` is idempotent across drop/recreate cycles.
        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm the partial index exists after migration and contains the
    /// `WHERE consumed_at IS NULL` clause.
    #[tokio::test]
    async fn oauth_authorization_requests_pending_index_is_partial() {
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
                            .eq("idx_oauth_authorization_requests_pending"),
                    )
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_oauth_authorization_requests_pending index should exist");

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
            .execute_unprepared("SELECT COUNT(*) FROM oauth_authorization_requests")
            .await;
        assert!(
            res.is_err(),
            "oauth_authorization_requests table should not exist after down"
        );
    }
}
