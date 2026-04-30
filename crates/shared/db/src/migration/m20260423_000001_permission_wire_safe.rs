use sea_orm_migration::prelude::*;

/// No-op migration that anchors the wire-safe Permission::Other(String) change
/// in the migration sequence. The code change is in uptrakit-shared-types;
/// no schema modification is required.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
