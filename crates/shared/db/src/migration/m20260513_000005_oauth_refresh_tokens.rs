use sea_orm_migration::prelude::*;

use crate::migration::helpers::{timestamp, timestamp_null};

/// Create the `oauth_refresh_tokens` table.
///
/// Backs refresh-token rotation for the MCP OAuth Authorization Server
/// (Plan A foundation) per
/// `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §10.1.
///
/// Each row represents an opaque, hashed refresh token. The schema supports
/// **family-replay detection** (every rotation chains `parent_id` into a
/// shared `family_id`; using a previously-rotated token revokes the entire
/// family) and **sliding + absolute TTLs** (`expires_at` slides on rotation,
/// `family_expires_at` is fixed at first issue).
///
/// `parent_id` is an opaque lineage marker, NOT a real foreign key — it
/// references rows in the same table, but we deliberately omit the FK so
/// pruning a rotated row never cascades into its descendants. Replay
/// detection scans by `family_id`, not by `parent_id`.
///
/// Foreign keys:
/// - `client_id` → `oauth_clients(id)` ON DELETE **Restrict** so that an
///   active token blocks deleting the client (revocation is the supported
///   path for retirement; deletion is reserved for FK-clean test data).
/// - `user_id` → `users(id)` ON DELETE **Cascade** so that deleting a user
///   removes their refresh-token rows.
/// - `consent_id` → `oauth_consents(id)` ON DELETE **Cascade** so that
///   deleting a consent row removes its dependent tokens (the runtime path
///   instead flips `revoked_at`; cascade only fires for FK-clean test data).
///
/// Indexes per §10.1:
/// - `idx_oauth_refresh_tokens_token_hash` — full index on `token_hash`.
///   The column is already UNIQUE so a separate index is partially redundant
///   on SQLite, but the spec lists it explicitly for cross-backend parity.
/// - `idx_oauth_refresh_tokens_family` — covering `(family_id, rotated_at)`
///   for replay detection.
/// - `idx_oauth_refresh_tokens_active_user_client` — partial index on
///   `(user_id, client_id) WHERE revoked_at IS NULL` for the active-tokens
///   admin view.
/// - `idx_oauth_refresh_tokens_consent` — covering `(consent_id)` for the
///   per-consent revocation cascade.
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
                    .table(OauthRefreshTokens::Table)
                    .col(
                        ColumnDef::new(OauthRefreshTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(OauthRefreshTokens::FamilyId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(OauthRefreshTokens::ParentId).uuid().null())
                    .col(
                        ColumnDef::new(OauthRefreshTokens::TokenHash)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(OauthRefreshTokens::ClientId)
                            .text()
                            .not_null(),
                    )
                    .col(ColumnDef::new(OauthRefreshTokens::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(OauthRefreshTokens::ConsentId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(OauthRefreshTokens::Scope).text().not_null())
                    .col(
                        ColumnDef::new(OauthRefreshTokens::Resource)
                            .text()
                            .not_null(),
                    )
                    .col(timestamp(OauthRefreshTokens::IssuedAt))
                    .col(timestamp(OauthRefreshTokens::ExpiresAt))
                    .col(timestamp(OauthRefreshTokens::FamilyExpiresAt))
                    .col(timestamp_null(OauthRefreshTokens::RotatedAt))
                    .col(timestamp_null(OauthRefreshTokens::RevokedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_refresh_tokens_client")
                            .from(OauthRefreshTokens::Table, OauthRefreshTokens::ClientId)
                            .to(OauthClients::Table, OauthClients::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_refresh_tokens_user")
                            .from(OauthRefreshTokens::Table, OauthRefreshTokens::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_refresh_tokens_consent")
                            .from(OauthRefreshTokens::Table, OauthRefreshTokens::ConsentId)
                            .to(OauthConsents::Table, OauthConsents::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Full index on token_hash. The column is also UNIQUE, but the spec
        // §10.1 lists this index explicitly for cross-backend parity.
        manager
            .create_index(
                Index::create()
                    .name("idx_oauth_refresh_tokens_token_hash")
                    .table(OauthRefreshTokens::Table)
                    .col(OauthRefreshTokens::TokenHash)
                    .to_owned(),
            )
            .await?;

        // Composite index for replay detection (scan a family by rotation order).
        manager
            .create_index(
                Index::create()
                    .name("idx_oauth_refresh_tokens_family")
                    .table(OauthRefreshTokens::Table)
                    .col(OauthRefreshTokens::FamilyId)
                    .col(OauthRefreshTokens::RotatedAt)
                    .to_owned(),
            )
            .await?;

        // Partial index on active (non-revoked) tokens per (user, client).
        //
        // sea_query does not support `WHERE` clauses on `CREATE INDEX`, so use
        // raw SQL. SQLite and PostgreSQL both support partial indexes with
        // identical `WHERE column IS NULL` syntax.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE INDEX idx_oauth_refresh_tokens_active_user_client \
                 ON oauth_refresh_tokens (user_id, client_id) \
                 WHERE revoked_at IS NULL",
            )
            .await?;

        // Full index on consent_id for the per-consent revocation cascade.
        manager
            .create_index(
                Index::create()
                    .name("idx_oauth_refresh_tokens_consent")
                    .table(OauthRefreshTokens::Table)
                    .col(OauthRefreshTokens::ConsentId)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(OauthRefreshTokens::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum OauthRefreshTokens {
    Table,
    Id,
    FamilyId,
    ParentId,
    TokenHash,
    ClientId,
    UserId,
    ConsentId,
    Scope,
    Resource,
    IssuedAt,
    ExpiresAt,
    FamilyExpiresAt,
    RotatedAt,
    RevokedAt,
}

#[derive(DeriveIden)]
enum OauthClients {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum OauthConsents {
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
            .position(|m| m.name() == "m20260513_000005_oauth_refresh_tokens")
            .expect("oauth_refresh_tokens migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, then roll this one back
    /// and re-apply it. Exercises both `up` and `down` paths.
    #[tokio::test]
    async fn oauth_refresh_tokens_migration_round_trips() {
        let db = test_db().await;
        let index = migration_index();
        // Bring the schema up to and including this migration.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through oauth_refresh_tokens must apply");

        // Verify the table is present and queryable.
        db.execute_unprepared("SELECT COUNT(*) FROM oauth_refresh_tokens")
            .await
            .expect("oauth_refresh_tokens should be queryable after up");

        // Roll back just this migration.
        Migrator::down(&db, Some(1))
            .await
            .expect("oauth_refresh_tokens migration must roll back cleanly");

        // After down, the table must be gone.
        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM oauth_refresh_tokens")
            .await;
        assert!(
            res.is_err(),
            "oauth_refresh_tokens table should be dropped by down"
        );

        // Re-apply to confirm `up` is idempotent across drop/recreate cycles.
        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm all four expected indexes exist after migration: three full
    /// indexes and one partial index with `WHERE revoked_at IS NULL`.
    #[tokio::test]
    async fn oauth_refresh_tokens_indexes_present() {
        let db = test_db().await;
        Migrator::up(&db, Some(migration_index() + 1))
            .await
            .expect("migrations should apply");

        let names: Vec<String> = db
            .query_all(
                &Query::select()
                    .column(Alias::new("name"))
                    .from(Alias::new("sqlite_master"))
                    .and_where(Expr::col(Alias::new("type")).eq("index"))
                    .and_where(Expr::col(Alias::new("tbl_name")).eq("oauth_refresh_tokens"))
                    .to_owned(),
            )
            .await
            .expect("index list query should succeed")
            .into_iter()
            .map(|row| {
                row.try_get::<String>("", "name")
                    .expect("index row should contain name")
            })
            .collect();

        for expected in [
            "idx_oauth_refresh_tokens_token_hash",
            "idx_oauth_refresh_tokens_family",
            "idx_oauth_refresh_tokens_active_user_client",
            "idx_oauth_refresh_tokens_consent",
        ] {
            assert!(
                names.iter().any(|n| n == expected),
                "expected index {expected} to exist; got: {names:?}"
            );
        }

        // Confirm the active-user-client index is partial.
        let row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("sql"))
                    .from(Alias::new("sqlite_master"))
                    .and_where(Expr::col(Alias::new("type")).eq("index"))
                    .and_where(
                        Expr::col(Alias::new("name"))
                            .eq("idx_oauth_refresh_tokens_active_user_client"),
                    )
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_oauth_refresh_tokens_active_user_client index should exist");

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
            .execute_unprepared("SELECT COUNT(*) FROM oauth_refresh_tokens")
            .await;
        assert!(
            res.is_err(),
            "oauth_refresh_tokens table should not exist after down"
        );
    }
}
