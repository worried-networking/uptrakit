use sea_orm_migration::prelude::*;

pub(super) struct Migration;

impl MigrationName for Migration {
    fn name(&self) -> &str {
        "m20260510_000001_create_instance_plugin_setting"
    }
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(InstancePluginSetting::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(InstancePluginSetting::PluginTypeId)
                            .text()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(InstancePluginSetting::Enabled)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(InstancePluginSetting::Config)
                            .json()
                            .not_null()
                            .default("{}"),
                    )
                    .col(
                        ColumnDef::new(InstancePluginSetting::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(InstancePluginSetting::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum InstancePluginSetting {
    Table,
    PluginTypeId,
    Enabled,
    Config,
    UpdatedAt,
}
