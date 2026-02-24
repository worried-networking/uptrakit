use sea_orm_migration::prelude::*;

/// Rename provider type `"docker_registry"` → `"docker"` in `provider_configs`.
///
/// The Docker provider was renamed from `docker_registry` to `docker` in the
/// `ProviderType` enum and the wire protocol.  This migration updates any
/// existing rows so they point at the correct provider implementation after
/// the upgrade.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE provider_configs \
                 SET provider_type = 'docker' \
                 WHERE provider_type = 'docker_registry'",
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .get_connection()
            .execute_unprepared(
                "UPDATE provider_configs \
                 SET provider_type = 'docker_registry' \
                 WHERE provider_type = 'docker'",
            )
            .await?;
        Ok(())
    }
}
