use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // tenant_discovery_allowlist: tenant-wide plugin type allowlist.
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("tenant_discovery_allowlist"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("tenant_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("plugin_type"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                Alias::new("tenant_discovery_allowlist"),
                                Alias::new("tenant_id"),
                            )
                            .to(Alias::new("tenants"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("tenant_discovery_allowlist"))
                    .name("idx_tenant_discovery_allowlist_tenant_plugin")
                    .col(Alias::new("tenant_id"))
                    .col(Alias::new("plugin_type"))
                    .unique()
                    .to_owned(),
            )
            .await?;

        // host_discovery_allowlist: per-host plugin type allowlist.
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("host_discovery_allowlist"))
                    .col(
                        ColumnDef::new(Alias::new("id"))
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("tenant_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("host_id"))
                            .uuid()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("plugin_type"))
                            .string()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(Alias::new("created_at"))
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                Alias::new("host_discovery_allowlist"),
                                Alias::new("tenant_id"),
                            )
                            .to(Alias::new("tenants"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(
                                Alias::new("host_discovery_allowlist"),
                                Alias::new("host_id"),
                            )
                            .to(Alias::new("hosts"), Alias::new("id"))
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("host_discovery_allowlist"))
                    .name("idx_host_discovery_allowlist_host_plugin")
                    .col(Alias::new("host_id"))
                    .col(Alias::new("plugin_type"))
                    .unique()
                    .to_owned(),
            )
            .await?;

        manager
            .create_index(
                Index::create()
                    .table(Alias::new("host_discovery_allowlist"))
                    .name("idx_host_discovery_allowlist_tenant_id")
                    .col(Alias::new("tenant_id"))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("host_discovery_allowlist"))
                    .to_owned(),
            )
            .await?;

        manager
            .drop_table(
                Table::drop()
                    .table(Alias::new("tenant_discovery_allowlist"))
                    .to_owned(),
            )
            .await
    }
}
