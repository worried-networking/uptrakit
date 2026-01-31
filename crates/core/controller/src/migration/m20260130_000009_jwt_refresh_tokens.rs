use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Rename token_hash → refresh_token_hash
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .rename_column(Sessions::TokenHash, Sessions::RefreshTokenHash)
                    .to_owned(),
            )
            .await?;

        // Add token_type column with default 'refresh_token'
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .add_column(
                        ColumnDef::new(Sessions::TokenType)
                            .string()
                            .not_null()
                            .default("refresh_token"),
                    )
                    .to_owned(),
            )
            .await?;

        // Add revoked_at column (nullable timestamp)
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .add_column(ColumnDef::new(Sessions::RevokedAt).timestamp().null())
                    .to_owned(),
            )
            .await?;

        // Drop last_activity_at column
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .drop_column(Sessions::LastActivityAt)
                    .to_owned(),
            )
            .await?;

        // Rename the existing index on token_hash
        manager
            .drop_index(
                Index::drop()
                    .name("idx_sessions_token_hash")
                    .table(Sessions::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_refresh_token_hash")
                    .table(Sessions::Table)
                    .col(Sessions::RefreshTokenHash)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Drop the new index
        manager
            .drop_index(
                Index::drop()
                    .name("idx_sessions_refresh_token_hash")
                    .table(Sessions::Table)
                    .to_owned(),
            )
            .await?;

        // Add last_activity_at back
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .add_column(
                        ColumnDef::new(Sessions::LastActivityAt)
                            .timestamp()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await?;

        // Drop revoked_at
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .drop_column(Sessions::RevokedAt)
                    .to_owned(),
            )
            .await?;

        // Drop token_type
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .drop_column(Sessions::TokenType)
                    .to_owned(),
            )
            .await?;

        // Rename refresh_token_hash back to token_hash
        manager
            .alter_table(
                Table::alter()
                    .table(Sessions::Table)
                    .rename_column(Sessions::RefreshTokenHash, Sessions::TokenHash)
                    .to_owned(),
            )
            .await?;

        // Recreate original index
        manager
            .create_index(
                Index::create()
                    .name("idx_sessions_token_hash")
                    .table(Sessions::Table)
                    .col(Sessions::TokenHash)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum Sessions {
    Table,
    TokenHash,
    RefreshTokenHash,
    TokenType,
    RevokedAt,
    LastActivityAt,
}
