use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;

use super::helpers::timestamp;

/// Drop the legacy `permissions` / `role_permissions` tables (M1.8).
///
/// Authorization has read `access_grants` exclusively since M1.4–M1.7;
/// these tables have been dead data since. This migration is destructive:
/// `down()` recreates the schema only — seeded rows are NOT restored.
///
/// `down()` must recreate the tables (a no-op is not enough): the
/// m20260728 down-path parks `role_permissions` rows by the literal
/// column names `role_id`/`permission_id` during its SQLite roles-table
/// recreation, so the table must exist (with exactly those column names)
/// for every down-chain that crosses this migration. The constraint is
/// SQLite-only — the PostgreSQL path uses plain `ALTER TABLE` — but the
/// recreation is harmless there.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // FK child first, then parent.
        manager
            .drop_table(Table::drop().table(RolePermissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Permissions::Table).to_owned())
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Schema-only recreation, parent first, then FK child — the exact
        // shape of the initial migration (m20260209). No indexes beyond
        // PK/UNIQUE ever existed on either table. Data is not restored.
        manager
            .create_table(
                Table::create()
                    .table(Permissions::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Permissions::Id)
                            .uuid()
                            .not_null()
                            .primary_key(),
                    )
                    .col(string_uniq(Permissions::Name))
                    .col(string_null(Permissions::Description))
                    .col(timestamp(Permissions::CreatedAt))
                    .to_owned(),
            )
            .await?;
        manager
            .create_table(
                Table::create()
                    .table(RolePermissions::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(RolePermissions::RoleId).uuid().not_null())
                    .col(
                        ColumnDef::new(RolePermissions::PermissionId)
                            .uuid()
                            .not_null(),
                    )
                    .primary_key(
                        Index::create()
                            .col(RolePermissions::RoleId)
                            .col(RolePermissions::PermissionId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(RolePermissions::Table, RolePermissions::RoleId)
                            .to(Roles::Table, Roles::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(RolePermissions::Table, RolePermissions::PermissionId)
                            .to(Permissions::Table, Permissions::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await
    }
}

#[derive(DeriveIden)]
enum Permissions {
    Table,
    Id,
    Name,
    Description,
    CreatedAt,
}

#[derive(DeriveIden)]
enum RolePermissions {
    Table,
    RoleId,
    PermissionId,
}

#[derive(DeriveIden)]
enum Roles {
    Table,
    Id,
}

#[cfg(test)]
mod tests {
    use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection};
    use sea_orm_migration::prelude::*;

    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    async fn table_exists(db: &DatabaseConnection, name: &str) -> bool {
        db.query_one(
            &Query::select()
                .column(Alias::new("name"))
                .from(Alias::new("sqlite_master"))
                .and_where(Expr::col(Alias::new("type")).eq("table"))
                .and_where(Expr::col(Alias::new("name")).eq(name))
                .to_owned(),
        )
        .await
        .expect("sqlite_master query")
        .is_some()
    }

    #[tokio::test]
    async fn up_drops_both_tables() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");
        assert!(
            !table_exists(&db, "permissions").await,
            "permissions must be dropped at tip"
        );
        assert!(
            !table_exists(&db, "role_permissions").await,
            "role_permissions must be dropped at tip"
        );
    }

    #[tokio::test]
    async fn down_recreates_empty_schema() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");
        Migrator::down(&db, Some(1)).await.expect("down");
        assert!(
            table_exists(&db, "permissions").await,
            "down must recreate permissions (schema-only)"
        );
        assert!(
            table_exists(&db, "role_permissions").await,
            "down must recreate role_permissions (schema-only)"
        );
        let rows = db
            .query_all(
                &Query::select()
                    .column(Alias::new("role_id"))
                    .from(Alias::new("role_permissions"))
                    .to_owned(),
            )
            .await
            .expect("query recreated table");
        assert!(
            rows.is_empty(),
            "recreation is schema-only — no data restored"
        );
    }
}
