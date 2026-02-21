use sea_orm_migration::prelude::*;
use uuid::Uuid;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();
        let db = manager.get_connection();

        // ============================================================
        // 1. Add ca_cert_pem column to mqtt_clients
        // ============================================================
        manager
            .alter_table(
                Table::alter()
                    .table(MqttClients::Table)
                    .add_column(ColumnDef::new(MqttClients::CaCertPem).text())
                    .to_owned(),
            )
            .await?;

        // ============================================================
        // 2. Add 4 new granular permissions
        // ============================================================
        let new_permissions = [
            (
                "view_software",
                "View software items, provider configs, and update history",
            ),
            (
                "manage_software",
                "Manage software items, provider configs, version checks, updates, and scheduler",
            ),
            ("view_hosts", "View hosts"),
            ("manage_hosts", "Manage hosts (update, deactivate)"),
        ];

        let mut new_perm_ids = Vec::new();
        for (name, description) in &new_permissions {
            let id = Uuid::now_v7();
            new_perm_ids.push((id, *name));

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

        // ============================================================
        // 3. Grant new permissions to existing roles that have the
        //    corresponding old (broader) permission.
        //
        //    Mapping:
        //      view_settings   => also gets view_software
        //      manage_settings => also gets manage_software
        //      view_agents     => also gets view_hosts
        //      manage_agents   => also gets manage_hosts
        // ============================================================
        let mappings = [
            ("view_software", "view_settings"),
            ("manage_software", "manage_settings"),
            ("view_hosts", "view_agents"),
            ("manage_hosts", "manage_agents"),
        ];

        for (new_perm_name, old_perm_name) in &mappings {
            let new_perm_id = new_perm_ids
                .iter()
                .find(|(_, name)| name == new_perm_name)
                .map(|(id, _)| *id)
                .ok_or(DbErr::Custom(format!(
                    "new permission '{new_perm_name}' not found"
                )))?;

            // Find the old permission ID
            let old_perm_select = sea_orm::sea_query::Query::select()
                .column(Permissions::Id)
                .from(Permissions::Table)
                .and_where(Expr::col(Permissions::Name).eq(*old_perm_name))
                .to_owned();

            let old_perm_rows = db.query_all(&old_perm_select).await?;
            for row in &old_perm_rows {
                use sea_orm::TryGetable;
                let old_perm_id: Uuid = Uuid::try_get_by(row, "id")
                    .map_err(|e| DbErr::Custom(format!("failed to get old perm ID: {e:?}")))?;

                // Find all roles that have the old permission
                let roles_select = sea_orm::sea_query::Query::select()
                    .column(RolePermissions::RoleId)
                    .from(RolePermissions::Table)
                    .and_where(Expr::col(RolePermissions::PermissionId).eq(old_perm_id))
                    .to_owned();

                let role_rows = db.query_all(&roles_select).await?;
                for role_row in &role_rows {
                    let role_id: Uuid = Uuid::try_get_by(role_row, "role_id")
                        .map_err(|e| DbErr::Custom(format!("failed to get role ID: {e:?}")))?;

                    // Grant new permission to this role
                    manager
                        .exec_stmt(
                            Query::insert()
                                .into_table(RolePermissions::Table)
                                .columns([RolePermissions::RoleId, RolePermissions::PermissionId])
                                .values_panic([role_id.into(), new_perm_id.into()])
                                .to_owned(),
                        )
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove the 4 new permissions and their role_permissions rows (cascading FK)
        let perm_names = [
            "view_software",
            "manage_software",
            "view_hosts",
            "manage_hosts",
        ];

        for name in &perm_names {
            manager
                .exec_stmt(
                    Query::delete()
                        .from_table(Permissions::Table)
                        .and_where(Expr::col(Permissions::Name).eq(*name))
                        .to_owned(),
                )
                .await?;
        }

        // Drop ca_cert_pem column
        manager
            .alter_table(
                Table::alter()
                    .table(MqttClients::Table)
                    .drop_column(MqttClients::CaCertPem)
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}

#[derive(DeriveIden)]
enum MqttClients {
    Table,
    CaCertPem,
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
