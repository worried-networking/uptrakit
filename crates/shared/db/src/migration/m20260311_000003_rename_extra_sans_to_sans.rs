use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260311_000003_rename_extra_sans_to_sans"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Uses sea_query to correctly quote `key` (reserved word in MySQL/MariaDB).
        manager
            .exec_stmt(
                Query::update()
                    .table(Alias::new("global_settings"))
                    .value(Alias::new("key"), "network.sans")
                    .and_where(Expr::col(Alias::new("key")).eq("network.extra_sans"))
                    .to_owned(),
            )
            .await?;
        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .exec_stmt(
                Query::update()
                    .table(Alias::new("global_settings"))
                    .value(Alias::new("key"), "network.extra_sans")
                    .and_where(Expr::col(Alias::new("key")).eq("network.sans"))
                    .to_owned(),
            )
            .await?;
        Ok(())
    }
}
