use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

/// This migration previously created mqtt_services, mqtt_service_certificates,
/// and mqtt_enrollment_tokens tables. These have been merged into the unified
/// `services` and `service_certificates` tables (migrations 007 and 008).
/// The mqtt_enrollment_tokens table has been replaced by settings-based token
/// hashes. This migration is now intentionally empty for schema compatibility.
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Ok(())
    }
}
