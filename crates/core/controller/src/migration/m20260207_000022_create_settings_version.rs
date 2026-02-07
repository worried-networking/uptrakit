use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::m20260129_000001_initial::Tenants;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(SettingsVersion::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(SettingsVersion::TenantId)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(big_integer(SettingsVersion::Version).default(0))
                    .col(big_integer(SettingsVersion::GlobalVersion).default(0))
                    .col(timestamp(SettingsVersion::UpdatedAt))
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_settings_version_tenant")
                            .from(SettingsVersion::Table, SettingsVersion::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Seed rows for all existing tenants
        let db = manager.get_connection();
        db.execute_unprepared(
            "INSERT INTO settings_version (tenant_id, version, global_version, updated_at) \
             SELECT id, 0, 0, CURRENT_TIMESTAMP FROM tenants",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(SettingsVersion::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum SettingsVersion {
    Table,
    TenantId,
    Version,
    GlobalVersion,
    UpdatedAt,
}
