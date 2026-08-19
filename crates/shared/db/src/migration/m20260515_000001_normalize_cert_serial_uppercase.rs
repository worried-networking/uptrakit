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
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        db.execute_unprepared(
            "UPDATE service_certificates SET serial_number = UPPER(serial_number)",
        )
        .await?;
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        db.execute_unprepared(
            "UPDATE system_service_certificates SET serial_number = UPPER(serial_number)",
        )
        .await?;
        Ok(())
    }

    async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
        let db = m.get_connection();
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        db.execute_unprepared(
            "UPDATE service_certificates SET serial_number = LOWER(serial_number)",
        )
        .await?;
        #[expect(
            clippy::disallowed_methods,
            reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
        )]
        db.execute_unprepared(
            "UPDATE system_service_certificates SET serial_number = LOWER(serial_number)",
        )
        .await?;
        Ok(())
    }
}
