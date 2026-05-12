use sea_orm_migration::prelude::*;

use crate::migration::helpers::{timestamp, timestamp_null};

/// Create the `oauth_consents` table.
///
/// Backs per-user consent grants for the MCP OAuth Authorization Server
/// (Plan A foundation) per
/// `docs/superpowers/specs/2026-05-12-mcp-oauth-authorization-design.md` §12.3.
///
/// Each row represents a user's grant of a specific scope set to an OAuth
/// Client. A partial UNIQUE index enforces that at most one *active*
/// (non-revoked) consent exists per `(user_id, client_id)` — revoked rows
/// remain in the table for audit but do not block re-grants.
///
/// `cimd_content_hash_at_grant` snapshots the client's
/// `oauth_clients.metadata_content_hash` at consent time, so the consent
/// template can render a diff between the metadata the user previously
/// approved and the current metadata when `revalidation_required_at` is
/// set by CIMD material-change detection (§11.3 step 7).
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
                    .table(OauthConsents::Table)
                    .col(
                        ColumnDef::new(OauthConsents::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(OauthConsents::UserId).uuid().not_null())
                    .col(ColumnDef::new(OauthConsents::ClientId).text().not_null())
                    .col(ColumnDef::new(OauthConsents::Scopes).text().not_null())
                    .col(
                        ColumnDef::new(OauthConsents::CimdContentHashAtGrant)
                            .text()
                            .null(),
                    )
                    .col(timestamp_null(OauthConsents::RevalidationRequiredAt))
                    .col(timestamp(OauthConsents::GrantedAt))
                    .col(timestamp_null(OauthConsents::RevokedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_consents_user")
                            .from(OauthConsents::Table, OauthConsents::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_oauth_consents_client")
                            .from(OauthConsents::Table, OauthConsents::ClientId)
                            .to(OauthClients::Table, OauthClients::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Partial UNIQUE index on active (non-revoked) consents.
        //
        // sea_query does not support `WHERE` clauses on `CREATE INDEX`, so use
        // raw SQL. SQLite and PostgreSQL both support partial unique indexes
        // with identical `WHERE column IS NULL` syntax, so a single statement
        // covers both backends.
        manager
            .get_connection()
            .execute_unprepared(
                "CREATE UNIQUE INDEX idx_oauth_consents_active_user_client_unique \
                 ON oauth_consents (user_id, client_id) \
                 WHERE revoked_at IS NULL",
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(OauthConsents::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum OauthConsents {
    Table,
    Id,
    UserId,
    ClientId,
    Scopes,
    CimdContentHashAtGrant,
    RevalidationRequiredAt,
    GrantedAt,
    RevokedAt,
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
            .position(|m| m.name() == "m20260512_000002_oauth_consents")
            .expect("oauth_consents migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, then roll this one back
    /// and re-apply it. Exercises both `up` and `down` paths.
    #[tokio::test]
    async fn oauth_consents_migration_round_trips() {
        let db = test_db().await;
        let index = migration_index();
        // Bring the schema up to and including the oauth_consents migration.
        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through oauth_consents must apply");

        // Verify the table is present and queryable.
        db.execute_unprepared("SELECT COUNT(*) FROM oauth_consents")
            .await
            .expect("oauth_consents should be queryable after up");

        // Roll back just this migration.
        Migrator::down(&db, Some(1))
            .await
            .expect("oauth_consents migration must roll back cleanly");

        // After down, the table must be gone.
        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM oauth_consents")
            .await;
        assert!(
            res.is_err(),
            "oauth_consents table should be dropped by down"
        );

        // Re-apply to confirm `up` is idempotent across drop/recreate cycles.
        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm the partial UNIQUE index exists after migration and contains
    /// both the `UNIQUE` qualifier and the `WHERE revoked_at IS NULL` clause.
    #[tokio::test]
    async fn oauth_consents_active_index_is_partial_unique() {
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
                            .eq("idx_oauth_consents_active_user_client_unique"),
                    )
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_oauth_consents_active_user_client_unique index should exist");

        let sql: String = row
            .try_get::<String>("", "sql")
            .expect("index row should contain SQL");
        assert!(sql.contains("UNIQUE"), "index must be UNIQUE; got: {sql}");
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
            .execute_unprepared("SELECT COUNT(*) FROM oauth_consents")
            .await;
        assert!(
            res.is_err(),
            "oauth_consents table should not exist after down"
        );
    }
}
