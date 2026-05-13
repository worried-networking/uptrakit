use sea_orm_migration::prelude::*;

use crate::migration::helpers::{timestamp, timestamp_null};

/// Create the `user_totp`, `user_recovery_codes`, and `mfa_challenges` tables.
///
/// These three tables back the 2FA backend:
///
/// - `user_totp`: one active TOTP secret per user (one-to-one; enforced by
///   the `UNIQUE` index on `user_id`).
/// - `user_recovery_codes`: up to 8 single-use recovery codes per user.
/// - `mfa_challenges`: short-lived bridge token issued after password verification
///   succeeds but before the second factor is verified; consumed when a session
///   is created.
///
/// All three tables cascade-delete when the parent `users` row is removed.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // user_totp: one active TOTP secret per user
        manager
            .create_table(
                Table::create()
                    .table(UserTotp::Table)
                    .col(ColumnDef::new(UserTotp::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(UserTotp::UserId).uuid().not_null())
                    .col(ColumnDef::new(UserTotp::Secret).text().not_null())
                    .col(
                        ColumnDef::new(UserTotp::IsActive)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(timestamp_null(UserTotp::EnrolledAt))
                    .col(ColumnDef::new(UserTotp::LastUsedStep).big_integer().null())
                    .col(timestamp(UserTotp::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserTotp::Table, UserTotp::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(UserTotp::Table)
                    .name("idx_user_totp_user_id")
                    .col(UserTotp::UserId)
                    .unique()
                    .to_owned(),
            )
            .await?;

        // user_recovery_codes: up to 8 codes per user, single-use
        manager
            .create_table(
                Table::create()
                    .table(UserRecoveryCodes::Table)
                    .col(
                        ColumnDef::new(UserRecoveryCodes::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(UserRecoveryCodes::UserId).uuid().not_null())
                    .col(
                        ColumnDef::new(UserRecoveryCodes::CodeHash)
                            .string()
                            .not_null(),
                    )
                    .col(timestamp(UserRecoveryCodes::CreatedAt))
                    .col(timestamp_null(UserRecoveryCodes::UsedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserRecoveryCodes::Table, UserRecoveryCodes::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(UserRecoveryCodes::Table)
                    .name("idx_user_recovery_codes_user_id_used_at")
                    .col(UserRecoveryCodes::UserId)
                    .col(UserRecoveryCodes::UsedAt)
                    .to_owned(),
            )
            .await?;

        // mfa_challenges: bridge token between password-verify and session creation
        manager
            .create_table(
                Table::create()
                    .table(MfaChallenges::Table)
                    .col(
                        ColumnDef::new(MfaChallenges::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(MfaChallenges::UserId).uuid().not_null())
                    .col(ColumnDef::new(MfaChallenges::TokenHash).string().not_null())
                    .col(ColumnDef::new(MfaChallenges::EmailCodeHash).string().null())
                    .col(
                        ColumnDef::new(MfaChallenges::AttemptCount)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(timestamp(MfaChallenges::ExpiresAt))
                    .col(timestamp_null(MfaChallenges::ConsumedAt))
                    .col(timestamp(MfaChallenges::CreatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .from(MfaChallenges::Table, MfaChallenges::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(MfaChallenges::Table)
                    .name("idx_mfa_challenges_token_hash")
                    .col(MfaChallenges::TokenHash)
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(MfaChallenges::Table)
                    .name("idx_mfa_challenges_expires_at")
                    .col(MfaChallenges::ExpiresAt)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(MfaChallenges::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(
                Table::drop()
                    .table(UserRecoveryCodes::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;
        manager
            .drop_table(Table::drop().table(UserTotp::Table).if_exists().to_owned())
            .await
    }
}

/// Target table reference for FK declarations pointing at `users`.
#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum UserTotp {
    Table,
    Id,
    UserId,
    Secret,
    IsActive,
    EnrolledAt,
    LastUsedStep,
    CreatedAt,
}

#[derive(DeriveIden)]
enum UserRecoveryCodes {
    Table,
    Id,
    UserId,
    CodeHash,
    CreatedAt,
    UsedAt,
}

#[derive(DeriveIden)]
enum MfaChallenges {
    Table,
    Id,
    UserId,
    TokenHash,
    EmailCodeHash,
    AttemptCount,
    ExpiresAt,
    ConsumedAt,
    CreatedAt,
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
            .position(|m| m.name() == "m20260516_000001_2fa")
            .expect("2fa migration must be registered") as u32
    }

    /// Apply all previous migrations plus this one, roll it back, and
    /// re-apply it — exercises both `up` and `down` paths.
    #[tokio::test]
    async fn migration_2fa_round_trips() {
        let db = test_db().await;
        let index = migration_index();

        Migrator::up(&db, Some(index + 1))
            .await
            .expect("migrations through 2fa must apply");

        db.execute_unprepared("SELECT COUNT(*) FROM user_totp")
            .await
            .expect("user_totp should be queryable after up");
        db.execute_unprepared("SELECT COUNT(*) FROM user_recovery_codes")
            .await
            .expect("user_recovery_codes should be queryable after up");
        db.execute_unprepared("SELECT COUNT(*) FROM mfa_challenges")
            .await
            .expect("mfa_challenges should be queryable after up");

        Migrator::down(&db, Some(1))
            .await
            .expect("2fa migration must roll back cleanly");

        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM mfa_challenges")
            .await;
        assert!(
            res.is_err(),
            "mfa_challenges table should be dropped by down"
        );
        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM user_recovery_codes")
            .await;
        assert!(
            res.is_err(),
            "user_recovery_codes table should be dropped by down"
        );
        let res = db
            .execute_unprepared("SELECT COUNT(*) FROM user_totp")
            .await;
        assert!(res.is_err(), "user_totp table should be dropped by down");

        Migrator::up(&db, None)
            .await
            .expect("migrations must re-apply after rollback");
    }

    /// Confirm the unique index on `user_totp.user_id` prevents duplicate rows.
    #[tokio::test]
    async fn user_totp_unique_user_id_enforced() {
        let db = test_db().await;
        Migrator::up(&db, Some(migration_index() + 1))
            .await
            .expect("migrations should apply");

        // Insert a user row to satisfy the FK constraint on user_totp.user_id.
        // The `users` table has no `tenant_id` column.
        let user_id = uuid::Uuid::now_v7();
        db.execute_unprepared(&format!(
            "INSERT INTO users \
             (id, email, first_name, last_name, is_active, created_at, updated_at) \
             VALUES ('{user_id}', 'u@example.com', 'A', 'B', 1, \
             CURRENT_TIMESTAMP, CURRENT_TIMESTAMP)"
        ))
        .await
        .expect("user insert");

        let totp_id1 = uuid::Uuid::now_v7();
        let totp_id2 = uuid::Uuid::now_v7();

        db.execute_unprepared(&format!(
            "INSERT INTO user_totp (id, user_id, secret, is_active, created_at) \
             VALUES ('{totp_id1}', '{user_id}', 'secret1', 0, CURRENT_TIMESTAMP)"
        ))
        .await
        .expect("first user_totp insert must succeed");

        let res = db
            .execute_unprepared(&format!(
                "INSERT INTO user_totp (id, user_id, secret, is_active, created_at) \
                 VALUES ('{totp_id2}', '{user_id}', 'secret2', 0, CURRENT_TIMESTAMP)"
            ))
            .await;
        assert!(
            res.is_err(),
            "unique index must reject second user_totp row for same user_id"
        );
    }

    /// Confirm the unique index on `mfa_challenges.token_hash` is present.
    #[tokio::test]
    async fn mfa_challenges_token_hash_index_is_unique() {
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
                    .and_where(Expr::col(Alias::new("name")).eq("idx_mfa_challenges_token_hash"))
                    .to_owned(),
            )
            .await
            .expect("index lookup should succeed")
            .expect("idx_mfa_challenges_token_hash must exist");

        let sql: String = row
            .try_get::<String>("", "sql")
            .expect("index row should contain SQL");
        assert!(
            sql.to_uppercase().contains("UNIQUE"),
            "idx_mfa_challenges_token_hash must be UNIQUE; got: {sql}"
        );
    }

    /// Exercise the standalone `down` impl.
    #[tokio::test]
    async fn down_drops_all_three_tables() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        let schema_manager = SchemaManager::new(&db);
        Migration.down(&schema_manager).await.expect("down");

        for table in ["user_totp", "user_recovery_codes", "mfa_challenges"] {
            let res = db
                .execute_unprepared(&format!("SELECT COUNT(*) FROM {table}"))
                .await;
            assert!(res.is_err(), "{table} should not exist after down");
        }
    }
}
