use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add the `view_notifications` and `manage_notifications` permissions and
/// assign them to the `settings_manager` role.
///
/// ## Motivation
///
/// These permissions were listed in the `settings_manager` role definition
/// inside `m20260310_000002_granular_permissions` but were added to that
/// migration *after* it had already been applied on existing databases.
/// This migration backfills them idempotently for all existing installations.
///
/// ## Role assignments
///
/// - `settings_manager`: granted `view_notifications` and `manage_notifications`
///
/// ## Idempotency
///
/// Permission INSERTs use `ON CONFLICT DO NOTHING` on the `name` column.
/// Role-permission INSERTs use `ON CONFLICT DO NOTHING` on the composite
/// `(role_id, permission_id)` PK.  Both make the migration safe to re-run.
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

        // Insert view_notifications permission (idempotent check-then-insert).
        for (perm_name, perm_desc) in [
            (
                "view_notifications",
                "View notification channels, rules, and delivery log.",
            ),
            (
                "manage_notifications",
                "Create, update, delete, and test notification channels and rules.",
            ),
        ] {
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
        }

        // Assign both permissions to the settings_manager role.
        grant_permission(manager, "settings_manager", "view_notifications").await?;
        grant_permission(manager, "settings_manager", "manage_notifications").await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove role-permission assignments then the permissions themselves.
        for perm_name in ["view_notifications", "manage_notifications"] {
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

            manager
                .exec_stmt(
                    Query::delete()
                        .from_table(Alias::new("permissions"))
                        .and_where(Expr::col(Alias::new("name")).eq(perm_name))
                        .to_owned(),
                )
                .await?;
        }

        Ok(())
    }
}
