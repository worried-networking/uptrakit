use sea_orm_migration::prelude::*;

use crate::migration::helpers::{timestamp, timestamp_null};

/// Create the `oauth_clients` table.
///
/// Backs the MCP OAuth Authorization Server (Plan A foundation) per
/// `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §11.1.
///
/// `id` is `TEXT` (not `UUID`) because Client Identifier Metadata Document
/// (CIMD) `client_id` values are HTTPS URLs, while Dynamic Client Registration
/// (DCR) clients use UUID-as-text. The Sea-ORM entity models this column as
/// `String`.
///
/// This table is dormant until the `oauth.mcp_enabled` master switch is set
/// to `true` in a later phase; the schema is established here so subsequent
/// tasks (consents, authorization codes, refresh tokens, etc.) can reference
/// it via foreign keys.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(OauthClients::Table)
                    .col(
                        ColumnDef::new(OauthClients::Id)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(OauthClients::ClientName).text().not_null())
                    .col(ColumnDef::new(OauthClients::ClientUri).text().null())
                    .col(ColumnDef::new(OauthClients::LogoUri).text().null())
                    .col(ColumnDef::new(OauthClients::RedirectUris).text().not_null())
                    .col(ColumnDef::new(OauthClients::DefaultScope).text().not_null())
                    .col(ColumnDef::new(OauthClients::GrantTypes).text().not_null())
                    .col(
                        ColumnDef::new(OauthClients::ResponseTypes)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(OauthClients::TokenEndpointAuthMethod)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(OauthClients::ClientSecretHash).text().null())
                    .col(
                        ColumnDef::new(OauthClients::RegistrationAccessTokenHash)
                            .text()
                            .null(),
                    )
                    .col(ColumnDef::new(OauthClients::CreatedVia).text().not_null())
                    .col(timestamp(OauthClients::CreatedAt))
                    .col(timestamp_null(OauthClients::LastUsedAt))
                    .col(timestamp_null(OauthClients::RevokedAt))
                    .col(timestamp_null(OauthClients::MetadataCachedAt))
                    .col(ColumnDef::new(OauthClients::MetadataEtag).text().null())
                    .col(
                        ColumnDef::new(OauthClients::MetadataContentHash)
                            .text()
                            .null(),
                    )
                    .col(ColumnDef::new(OauthClients::MetadataRaw).text().null())
                    .col(
                        ColumnDef::new(OauthClients::MetadataParseError)
                            .text()
                            .null(),
                    )
                    .col(timestamp_null(OauthClients::MetadataParseErrorAt))
                    .col(timestamp_null(OauthClients::TrustedAt))
                    .to_owned(),
            )
            .await?;

        // Partial index on active (non-revoked) clients. SQLite and PostgreSQL
        // both support partial indexes with identical `WHERE column IS NULL`
        // syntax, so a single statement covers both backends.
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_oauth_clients_active \
                 ON oauth_clients (revoked_at) \
                 WHERE revoked_at IS NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(OauthClients::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum OauthClients {
    Table,
    Id,
    ClientName,
    ClientUri,
    LogoUri,
    RedirectUris,
    DefaultScope,
    GrantTypes,
    ResponseTypes,
    TokenEndpointAuthMethod,
    ClientSecretHash,
    RegistrationAccessTokenHash,
    CreatedVia,
    CreatedAt,
    LastUsedAt,
    RevokedAt,
    MetadataCachedAt,
    MetadataEtag,
    MetadataContentHash,
    MetadataRaw,
    MetadataParseError,
    MetadataParseErrorAt,
    TrustedAt,
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
            .position(|m| m.name() == "m20260513_000001_oauth_clients")
            .expect("oauth_clients migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, then roll this one back
    /// and re-apply it. Exercises both `up` and `down` paths.
    #[tokio::test]
    async fn oauth_clients_migration_round_trips() {
        let db = test_db().await;
        let index = migration_index();
        // Bring the schema up to and including the oauth_clients migration.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through oauth_clients must apply");

        // Verify the table is present and queryable.
        db.execute(
            &Query::select()
                .expr(Expr::col(Asterisk).count())
                .from(Alias::new("oauth_clients"))
                .to_owned(),
        )
        .await
        .expect("oauth_clients should be queryable after up");

        // Roll back just this migration.
        Migrator::down(&db, Some(1))
            .await
            .expect("oauth_clients migration must roll back cleanly");

        // After down, the table must be gone.
        let res = db
            .execute(
                &Query::select()
                    .expr(Expr::col(Asterisk).count())
                    .from(Alias::new("oauth_clients"))
                    .to_owned(),
            )
            .await;
        assert!(
            res.is_err(),
            "oauth_clients table should be dropped by down"
        );

        // Re-apply to confirm `up` is idempotent across drop/recreate cycles.
        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm the partial index exists after migration.
    #[tokio::test]
    async fn oauth_clients_active_index_is_partial() {
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
                    .and_where(Expr::col(Alias::new("name")).eq("idx_oauth_clients_active"))
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_oauth_clients_active index should exist");

        let sql: String = row
            .try_get::<String>("", "sql")
            .expect("index row should contain SQL");
        assert!(
            sql.contains("WHERE revoked_at IS NULL"),
            "index must be partial on revoked_at IS NULL; got: {sql}"
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
                    .from(Alias::new("oauth_clients"))
                    .to_owned(),
            )
            .await;
        assert!(
            res.is_err(),
            "oauth_clients table should not exist after down"
        );
    }
}
