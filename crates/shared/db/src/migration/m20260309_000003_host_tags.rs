use sea_orm_migration::prelude::*;

/// Create `host_tags` and `host_tag_assignments` tables.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // ── host_tags ────────────────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(HostTags::Table)
                    .col(ColumnDef::new(HostTags::Id).uuid().not_null().primary_key())
                    .col(ColumnDef::new(HostTags::TenantId).uuid().not_null())
                    .col(ColumnDef::new(HostTags::Name).string().not_null())
                    .col(ColumnDef::new(HostTags::Color).text().not_null())
                    .col(ColumnDef::new(HostTags::Description).text().null())
                    .col(
                        ColumnDef::new(HostTags::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HostTags::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .col(
                        ColumnDef::new(HostTags::DeactivatedAt)
                            .timestamp_with_time_zone()
                            .null(),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_tags_tenant_id")
                            .from(HostTags::Table, HostTags::TenantId)
                            .to(Tenants::Table, Tenants::Id)
                            .on_delete(ForeignKeyAction::Restrict),
                    )
                    .to_owned(),
            )
            .await?;

        // Index for tenant-scoped queries
        manager
            .create_index(
                Index::create()
                    .name("idx_host_tags_tenant_id")
                    .table(HostTags::Table)
                    .col(HostTags::TenantId)
                    .to_owned(),
            )
            .await?;

        // Partial unique: one active tag name per tenant.
        //
        // SQLite does not support partial indexes via sea_query's `.and_where()`,
        // so we use `execute_unprepared` with DB-specific raw SQL. This is the
        // same pattern used by other migrations that need partial unique indexes.
        let backend = manager.get_database_backend();
        if backend == sea_orm::DatabaseBackend::MySql {
            // MariaDB does not support partial indexes. Use a regular composite
            // unique index on (tenant_id, name, deactivated_at) instead.
            manager
                .create_index(
                    Index::create()
                        .name("uix_host_tags_tenant_name")
                        .table(HostTags::Table)
                        .col(HostTags::TenantId)
                        .col(HostTags::Name)
                        .col(HostTags::DeactivatedAt)
                        .unique()
                        .to_owned(),
                )
                .await?;
        } else {
            let partial_unique_sql = "CREATE UNIQUE INDEX uix_host_tags_tenant_name \
                 ON host_tags (tenant_id, name) \
                 WHERE deactivated_at IS NULL";
            manager
                .get_connection()
                .execute_unprepared(partial_unique_sql)
                .await?;
        }

        // ── host_tag_assignments ─────────────────────────────────────────
        manager
            .create_table(
                Table::create()
                    .table(HostTagAssignments::Table)
                    .col(
                        ColumnDef::new(HostTagAssignments::HostTagId)
                            .uuid()
                            .not_null(),
                    )
                    .col(ColumnDef::new(HostTagAssignments::HostId).uuid().not_null())
                    .col(
                        ColumnDef::new(HostTagAssignments::AssignedAt)
                            .timestamp_with_time_zone()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(HostTagAssignments::HostTagId)
                            .col(HostTagAssignments::HostId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_tag_assignments_tag_id")
                            .from(HostTagAssignments::Table, HostTagAssignments::HostTagId)
                            .to(HostTags::Table, HostTags::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .name("fk_host_tag_assignments_host_id")
                            .from(HostTagAssignments::Table, HostTagAssignments::HostId)
                            .to(Hosts::Table, Hosts::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Index for host lookups (find all tags for a host)
        manager
            .create_index(
                Index::create()
                    .name("idx_host_tag_assignments_host_id")
                    .table(HostTagAssignments::Table)
                    .col(HostTagAssignments::HostId)
                    .to_owned(),
            )
            .await?;

        // Index for tag lookups (find all hosts with a tag)
        manager
            .create_index(
                Index::create()
                    .name("idx_host_tag_assignments_tag_id")
                    .table(HostTagAssignments::Table)
                    .col(HostTagAssignments::HostTagId)
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(HostTagAssignments::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(HostTags::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum HostTags {
    Table,
    Id,
    TenantId,
    Name,
    Color,
    Description,
    CreatedAt,
    UpdatedAt,
    DeactivatedAt,
}

#[derive(DeriveIden)]
enum HostTagAssignments {
    Table,
    HostTagId,
    HostId,
    AssignedAt,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

#[derive(DeriveIden)]
enum Hosts {
    Table,
    Id,
}
