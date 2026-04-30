use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add the `access_mcp` permission and grant it to roles that hold
/// `view_software` OR `trigger_updates`.
///
/// ## Role assignments
///
/// - `viewer`: granted (holds `view_software`)
/// - `operator`: granted (holds `trigger_updates`)
///   NOTE: `operator` lacks `view_software`, so it can call `trigger_update`
///   via MCP but will receive 403 on all history tools. In practice,
///   `AccessPreset::Operator` always bundles `viewer` + `operator`.
/// - `software_manager`: granted (holds `trigger_updates`; lacks `view_software`)
/// - `settings_manager`: NOT granted (lacks both)
///
/// ## Idempotency
///
/// INSERT uses check-then-insert (SELECT before INSERT).
/// Role grants use `WHERE NOT EXISTS` subqueries.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Grant a permission to a role by resolving both by name via a subquery.
/// Idempotent (uses `WHERE NOT EXISTS` — portable across all backends).
async fn grant_permission(
    manager: &SchemaManager<'_>,
    role_name: &str,
    perm_name: &str,
) -> Result<(), DbErr> {
    let sql = format!(
        "INSERT INTO role_permissions (role_id, permission_id) \
         SELECT r.id, p.id \
         FROM roles r, permissions p \
         WHERE r.name = '{role_name}' AND p.name = '{perm_name}' \
         AND NOT EXISTS ( \
           SELECT 1 FROM role_permissions rp \
           WHERE rp.role_id = r.id AND rp.permission_id = p.id \
         )"
    );
    manager.get_connection().execute_unprepared(&sql).await?;
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();
        let perm_name = "access_mcp";
        let perm_desc = "Access the MCP server endpoint";

        let exists = manager
            .get_connection()
            .query_one_raw(sea_orm::Statement::from_string(
                manager.get_database_backend(),
                format!("SELECT 1 FROM permissions WHERE name = '{perm_name}' LIMIT 1"),
            ))
            .await?;
        if exists.is_none() {
            manager
                .exec_stmt(
                    Query::insert()
                        .into_table(Alias::new("permissions"))
                        .columns([
                            Alias::new("id"),
                            Alias::new("name"),
                            Alias::new("description"),
                            Alias::new("created_at"),
                        ])
                        .values_panic([
                            Uuid::now_v7().into(),
                            perm_name.into(),
                            perm_desc.into(),
                            now.into(),
                        ])
                        .to_owned(),
                )
                .await?;
        }

        for role in ["viewer", "operator", "software_manager"] {
            grant_permission(manager, role, perm_name).await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let perm_name = "access_mcp";

        // Remove role-permission assignments first (FK constraint).
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("role_permissions"))
                    .and_where(
                        Expr::col(Alias::new("permission_id")).in_subquery(
                            Query::select()
                                .from(Alias::new("permissions"))
                                .column(Alias::new("id"))
                                .and_where(Expr::col(Alias::new("name")).eq(perm_name))
                                .to_owned(),
                        ),
                    )
                    .to_owned(),
            )
            .await?;

        // Remove the permission itself.
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("permissions"))
                    .and_where(Expr::col(Alias::new("name")).eq(perm_name))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
