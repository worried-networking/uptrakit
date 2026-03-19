use sea_orm_migration::prelude::*;

use super::helpers;

/// Simplify the `autodiscovery_ignores` table from per-plugin-config scoping
/// (`plugin_config_id` + `package_identifier`) to tenant-wide name-based
/// scoping (`name`).
///
/// The old schema was fragile for multi-target discovery items: a single
/// discovered item (e.g. a PHS GitHub-managed app) produces targets for
/// multiple plugin configs (GitHub Releases + PHS Shell), but the ignore rule
/// was only created for one config.  Switching to name-based ignoring means a
/// single rule covers all targets for an item.
///
/// Existing ignore data is truncated — the table is small and users can
/// re-create rules via the simplified API.
///
/// ## New schema
///
/// | Column       | Type      | Constraint                    |
/// |-------------|-----------|-------------------------------|
/// | id          | BLOB PK   |                               |
/// | tenant_id   | BLOB FK   | → tenants(id) ON DELETE CASCADE |
/// | name        | TEXT       | NOT NULL                      |
/// | created_at  | TIMESTAMP | NOT NULL                      |
///
/// Unique: `(tenant_id, name)`.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[derive(DeriveIden)]
enum AutodiscoveryIgnores {
    Table,
    Id,
    TenantId,
    Name,
    CreatedAt,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let is_postgres = manager.get_database_backend() == sea_orm::DbBackend::Postgres;

        if is_postgres {
            // PostgreSQL: truncate, drop old columns and indexes, add new column.
            manager
                .get_connection()
                .execute_unprepared("DELETE FROM autodiscovery_ignores")
                .await?;

            // Drop the old unique index (it references the columns we're about to drop).
            helpers::drop_index_if_exists(
                manager,
                "uq_autodiscovery_ignores_tenant_config_package",
                "autodiscovery_ignores",
            )
            .await?;

            manager
                .alter_table(
                    Table::alter()
                        .table(AutodiscoveryIgnores::Table)
                        .drop_column(Alias::new("plugin_config_id"))
                        .to_owned(),
                )
                .await?;

            manager
                .alter_table(
                    Table::alter()
                        .table(AutodiscoveryIgnores::Table)
                        .drop_column(Alias::new("package_identifier"))
                        .to_owned(),
                )
                .await?;

            manager
                .alter_table(
                    Table::alter()
                        .table(AutodiscoveryIgnores::Table)
                        .add_column(
                            ColumnDef::new(AutodiscoveryIgnores::Name)
                                .string()
                                .not_null(),
                        )
                        .to_owned(),
                )
                .await?;

            // Create the new unique index.
            manager
                .create_index(
                    Index::create()
                        .name("uix_autodiscovery_ignores_tenant_name")
                        .table(AutodiscoveryIgnores::Table)
                        .col(AutodiscoveryIgnores::TenantId)
                        .col(AutodiscoveryIgnores::Name)
                        .unique()
                        .to_owned(),
                )
                .await?;

            return Ok(());
        }

        // SQLite: drop + recreate (SQLite cannot ALTER TABLE DROP COLUMN
        // reliably — and we're truncating data anyway).
        manager
            .drop_table(
                Table::drop()
                    .table(AutodiscoveryIgnores::Table)
                    .if_exists()
                    .to_owned(),
            )
            .await?;

        // Recreate the table from scratch.
        manager
            .create_table(
                Table::create()
                    .table(AutodiscoveryIgnores::Table)
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::Id)
                            .uuid()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::TenantId)
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::Name)
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(AutodiscoveryIgnores::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_autodiscovery_ignores_tenant")
                            .from(AutodiscoveryIgnores::Table, AutodiscoveryIgnores::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .name("uix_autodiscovery_ignores_tenant_name")
                    .table(AutodiscoveryIgnores::Table)
                    .col(AutodiscoveryIgnores::TenantId)
                    .col(AutodiscoveryIgnores::Name)
                    .unique()
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Irreversible: the old schema's data was truncated.
        Err(DbErr::Migration(
            "cannot reverse autodiscovery_ignores simplification".to_owned(),
        ))
    }
}
