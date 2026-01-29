use sea_orm_migration::prelude::*;
use sea_orm_migration::schema::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Create roles table
        manager
            .create_table(
                Table::create()
                    .table(Roles::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(Roles::Id).uuid().not_null().primary_key())
                    .col(string_uniq(Roles::Name))
                    .col(string_null(Roles::Description))
                    .col(timestamp(Roles::CreatedAt))
                    .to_owned(),
            )
            .await?;

        // Create permissions table
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

        // Create role_permissions junction table
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
            .await?;

        // Create user_roles junction table
        manager
            .create_table(
                Table::create()
                    .table(UserRoles::Table)
                    .if_not_exists()
                    .col(ColumnDef::new(UserRoles::UserId).uuid().not_null())
                    .col(ColumnDef::new(UserRoles::RoleId).uuid().not_null())
                    .col(timestamp(UserRoles::AssignedAt))
                    .primary_key(
                        Index::create()
                            .col(UserRoles::UserId)
                            .col(UserRoles::RoleId),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserRoles::Table, UserRoles::UserId)
                            .to(Users::Table, Users::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .foreign_key(
                        ForeignKey::create()
                            .from(UserRoles::Table, UserRoles::RoleId)
                            .to(Roles::Table, Roles::Id)
                            .on_delete(ForeignKeyAction::Cascade),
                    )
                    .to_owned(),
            )
            .await?;

        // Seed default admin role and permissions
        let db = manager.get_connection();

        // Insert admin role
        let admin_role_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();

        db.execute_unprepared(&format!(
            "INSERT INTO roles (id, name, description, created_at) VALUES ('{}', 'admin', 'Administrator with full system access', '{}')",
            admin_role_id,
            now.unix_timestamp()
        )).await?;

        // Insert default permissions
        let permissions = vec![
            ("manage_users", "Create, update, and deactivate users"),
            ("manage_roles", "Assign and manage user roles"),
            ("manage_agents", "Manage monitoring agents"),
            ("view_system", "View system information"),
        ];

        for (name, description) in permissions {
            let permission_id = Uuid::now_v7();
            db.execute_unprepared(&format!(
                "INSERT INTO permissions (id, name, description, created_at) VALUES ('{}', '{}', '{}', '{}')",
                permission_id,
                name,
                description,
                now.unix_timestamp()
            )).await?;

            // Link permission to admin role
            db.execute_unprepared(&format!(
                "INSERT INTO role_permissions (role_id, permission_id) VALUES ('{}', '{}')",
                admin_role_id, permission_id
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(UserRoles::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(RolePermissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Permissions::Table).to_owned())
            .await?;
        manager
            .drop_table(Table::drop().table(Roles::Table).to_owned())
            .await?;
        Ok(())
    }
}

#[derive(DeriveIden)]
enum Roles {
    Table,
    Id,
    Name,
    Description,
    CreatedAt,
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
enum UserRoles {
    Table,
    UserId,
    RoleId,
    AssignedAt,
}

#[derive(DeriveIden)]
enum Users {
    Table,
    Id,
}
