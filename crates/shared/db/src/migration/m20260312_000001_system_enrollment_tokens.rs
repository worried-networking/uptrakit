use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

/// Create `system_enrollment_tokens` table and add `system_enrollment_token_id`
/// to `system_services`.
///
/// System enrollment tokens bring system service enrollment to full parity with
/// tenant enrollment tokens: multiple named tokens, backend-generated random
/// secrets, Argon2id hashing, usage limits, TTL, and a "shown once" UX.
///
/// Unlike tenant tokens, system tokens:
/// - Have no `tenant_id` column (global scope)
/// - Have no `allowed_capabilities` (system services have fixed capabilities)
/// - Have no FK to `users` for `created_by_user_id` (users are tenant-scoped;
///   the ID is stored for audit purposes only)
///
/// Also removes the old `system_services.enrollment_token` plaintext setting.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // --- system_enrollment_tokens ---
        manager
            .create_table(
                Table::create()
                    .table(SystemEnrollmentTokens::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SystemEnrollmentTokens::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(string(SystemEnrollmentTokens::Name))
                    .col(string_uniq(SystemEnrollmentTokens::TokenHash))
                    .col(
                        ColumnDef::new(SystemEnrollmentTokens::MaxUses)
                            .integer()
                            .null(),
                    )
                    .col(
                        ColumnDef::new(SystemEnrollmentTokens::CurrentUses)
                            .integer()
                            .not_null()
                            .default(0),
                    )
                    .col(timestamp_null(SystemEnrollmentTokens::ExpiresAt))
                    .col(timestamp(SystemEnrollmentTokens::CreatedAt))
                    .col(timestamp_null(SystemEnrollmentTokens::RevokedAt))
                    .col(
                        ColumnDef::new(SystemEnrollmentTokens::CreatedByUserId)
                            .uuid()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // --- Add system_enrollment_token_id to system_services ---
        // No FK constraint: tokens can be revoked/deleted after a service enrolls.
        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .add_column(
                        ColumnDef::new(SystemServices::SystemEnrollmentTokenId)
                            .uuid()
                            .null(),
                    )
                    .to_owned(),
            )
            .await?;

        // --- Remove old plaintext enrollment token from global_settings ---
        // The old system_services.enrollment_token setting is superseded by the
        // new system_enrollment_tokens table. Delete it from the DB so the
        // application no longer reads it.
        let db = manager.get_connection();
        db.execute_unprepared(
            "DELETE FROM global_settings WHERE key = 'system_services.enrollment_token'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .alter_table(
                Table::alter()
                    .table(SystemServices::Table)
                    .drop_column(SystemServices::SystemEnrollmentTokenId)
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(SystemEnrollmentTokens::Table)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum SystemEnrollmentTokens {
    Table,
    Id,
    Name,
    TokenHash,
    MaxUses,
    CurrentUses,
    ExpiresAt,
    CreatedAt,
    RevokedAt,
    CreatedByUserId,
}

#[derive(DeriveIden)]
enum SystemServices {
    Table,
    SystemEnrollmentTokenId,
}
