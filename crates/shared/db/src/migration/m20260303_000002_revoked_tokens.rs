use sea_orm_migration::prelude::*;

/// Add tables for persisting JWT token revocations across controller restarts.
///
/// Two tables are created:
///
/// - `revoked_token_jtis`: JTI-level revocations. A row persists until the
///   token's `expires_at` is past; purged by the periodic cleanup task.
/// - `revoked_token_users`: user-level revocations. Rows are purged once
///   `purge_after` is past (ensuring all pre-revocation tokens have expired).
///
/// Both tables are seeded into the in-memory denylist at controller startup.
/// New revocations are written here by the originating controller and
/// broadcast to other instances via a `ControllerMessage::TokenRevoked` NATS
/// event; remote instances update their in-memory caches but do not write to
/// DB (avoiding double-write).
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(RevokedTokenJtis::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RevokedTokenJtis::Jti)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(RevokedTokenJtis::ExpiresAt)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_table(
                Table::create()
                    .table(RevokedTokenUsers::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(RevokedTokenUsers::UserId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(RevokedTokenUsers::IatCutoff)
                            .big_integer()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(RevokedTokenUsers::PurgeAfter)
                            .big_integer()
                            .not_null(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(RevokedTokenUsers::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RevokedTokenJtis::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum RevokedTokenJtis {
    Table,
    Jti,
    ExpiresAt,
}

#[derive(DeriveIden)]
enum RevokedTokenUsers {
    Table,
    UserId,
    IatCutoff,
    PurgeAfter,
}
