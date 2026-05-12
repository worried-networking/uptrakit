use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        // x509-parser produced lowercase colon-separated serials (e.g. "01:ab:cd").
        // x509-cert produces uppercase (e.g. "01:AB:CD"). Normalise existing rows
        // so lookups work without per-call .to_lowercase() adapters.
        db.execute_unprepared(
            "UPDATE service_certificate SET serial_number = UPPER(serial_number)",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE system_service_certificate SET serial_number = UPPER(serial_number)",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        db.execute_unprepared(
            "UPDATE service_certificate SET serial_number = LOWER(serial_number)",
        )
        .await?;
        db.execute_unprepared(
            "UPDATE system_service_certificate SET serial_number = LOWER(serial_number)",
        )
        .await?;
        Ok(())
    }
}
