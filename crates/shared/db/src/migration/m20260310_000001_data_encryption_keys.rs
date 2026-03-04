use sea_orm_migration::prelude::*;

/// Add a table for storing wrapped data encryption keys (DEKs).
///
/// Envelope encryption: the master key (KEK) wraps DEKs stored in this table.
/// Data is encrypted with DEKs — never directly with the KEK. This enables
/// O(1) master key rotation (re-wrap DEKs only, no data re-encryption).
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(DataEncryptionKeys::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(DataEncryptionKeys::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(DataEncryptionKeys::KeyId)
                            .text()
                            .not_null()
                            .unique_key(),
                    )
                    .col(
                        ColumnDef::new(DataEncryptionKeys::WrappedKey)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DataEncryptionKeys::KekFingerprint)
                            .text()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DataEncryptionKeys::Status)
                            .text()
                            .not_null()
                            .default("active"),
                    )
                    .col(
                        ColumnDef::new(DataEncryptionKeys::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(DataEncryptionKeys::RetiredAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(DataEncryptionKeys::Table).to_owned())
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum DataEncryptionKeys {
    Table,
    Id,
    KeyId,
    WrappedKey,
    KekFingerprint,
    Status,
    CreatedAt,
    RetiredAt,
}
