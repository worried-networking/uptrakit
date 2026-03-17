use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add the `test_plugin_configs` permission and assign it to the
/// `command_manager` and `software_manager` built-in roles.
///
/// ## Motivation
///
/// Plugin configuration dry-run / test functionality requires a dedicated
/// permission so that only users with command or software management
/// privileges can trigger config tests against hosts.
///
/// ## Role assignments
///
/// - `command_manager`: granted `test_plugin_configs`
/// - `software_manager`: granted `test_plugin_configs`
///
/// ## Idempotency
///
/// Permission INSERT uses check-then-insert (SELECT before INSERT).
/// Role-permission INSERTs use `WHERE NOT EXISTS` subqueries.
/// Both make the migration safe to re-run.
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

        // Insert test_plugin_configs permission (idempotent check-then-insert).
        let perm_name = "test_plugin_configs";
        let perm_desc = "Test plugin configurations against hosts";

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

        // Assign the permission to command_manager and software_manager roles.
        grant_permission(manager, "command_manager", perm_name).await?;
        grant_permission(manager, "software_manager", perm_name).await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let perm_name = "test_plugin_configs";

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
