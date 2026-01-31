use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();

        // 1. Clear role_permissions and permissions
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(RolePermissions::Table)
                    .to_owned(),
            )
            .await?;

        manager
            .exec_stmt(Query::delete().from_table(Permissions::Table).to_owned())
            .await?;

        // 2. Insert 4 new permissions
        let permissions = vec![
            ("view_settings", "View system settings"),
            ("manage_settings", "Create and modify system settings"),
            ("view_agents", "View monitoring agents"),
            ("manage_agents", "Approve, reject, and manage agents"),
        ];

        let mut permission_ids = Vec::new();
        for (name, description) in &permissions {
            let id = Uuid::now_v7();
            permission_ids.push((id, *name));

            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Permissions::Table)
                        .columns([
                            Permissions::Id,
                            Permissions::Name,
                            Permissions::Description,
                            Permissions::CreatedAt,
                        ])
                        .values_panic([
                            id.into(),
                            (*name).into(),
                            (*description).into(),
                            now.into(),
                        ])
                        .to_owned(),
                )
                .await?;
        }

        // 3. Find admin role and assign all 4 permissions
        let select_admin = Query::select()
            .column(Roles::Id)
            .from(Roles::Table)
            .and_where(Expr::col(Roles::Name).eq("admin"))
            .to_owned();
        let admin_role_rows = manager
            .get_connection()
            .query_all(&select_admin)
            .await?;

        let admin_role_id: Uuid = admin_role_rows
            .first()
            .ok_or(DbErr::Custom("admin role not found".to_string()))?
            .try_get("", "id")?;

        for (perm_id, _) in &permission_ids {
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(RolePermissions::Table)
                        .columns([RolePermissions::RoleId, RolePermissions::PermissionId])
                        .values_panic([admin_role_id.into(), (*perm_id).into()])
                        .to_owned(),
                )
                .await?;
        }

        // 4. Insert 'user' role
        let user_role_id = Uuid::now_v7();
        manager
            .exec_stmt(
                Query::insert()
                    .into_table(Roles::Table)
                    .columns([Roles::Id, Roles::Name, Roles::Description, Roles::CreatedAt])
                    .values_panic([
                        user_role_id.into(),
                        "user".into(),
                        "Standard user with limited access".into(),
                        now.into(),
                    ])
                    .to_owned(),
            )
            .await?;

        // 5. Assign view_agents to user role
        let view_agents_id = permission_ids
            .iter()
            .find(|(_, name)| *name == "view_agents")
            .map(|(id, _)| *id)
            .ok_or(DbErr::Custom(
                "view_agents permission not found".to_string(),
            ))?;

        manager
            .exec_stmt(
                Query::insert()
                    .into_table(RolePermissions::Table)
                    .columns([RolePermissions::RoleId, RolePermissions::PermissionId])
                    .values_panic([user_role_id.into(), view_agents_id.into()])
                    .to_owned(),
            )
            .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove user role (cascades to role_permissions and user_roles)
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Roles::Table)
                    .and_where(Expr::col(Roles::Name).eq("user"))
                    .to_owned(),
            )
            .await?;

        // Clear new permissions (cascades to role_permissions)
        let new_perms = [
            "view_settings",
            "manage_settings",
            "view_agents",
            "manage_agents",
        ];
        for name in new_perms {
            manager
                .exec_stmt(
                    Query::delete()
                        .from_table(Permissions::Table)
                        .and_where(Expr::col(Permissions::Name).eq(name))
                        .to_owned(),
                )
                .await?;
        }

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
