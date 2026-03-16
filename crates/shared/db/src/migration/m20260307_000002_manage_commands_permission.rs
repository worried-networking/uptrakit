use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add the `manage_commands` permission and assign it to the `owner` and
/// `admin` roles.
///
/// ## Motivation
///
/// The existing `manage_software` permission covered both non-command
/// operations (software items, version tracking) and command-bearing plugin
/// config fields (shell commands, Docker `post_pull_command`, custom hook
/// `commands` arrays). This conflation means anyone with `manage_software`
/// effectively has code-execution authority on all managed hosts.
///
/// `manage_commands` separates that authority. Users with `manage_software`
/// but **without** `manage_commands` can create and manage software items,
/// version tracking, and the scheduler, but cannot modify plugin config
/// command fields.
///
/// ## Role assignments
///
/// - `owner`: granted `manage_commands`
/// - `admin`: granted `manage_commands`
/// - `user`: not granted (read-only role, unchanged)
///
/// ## Idempotency
///
/// Permission INSERT uses `ON CONFLICT DO NOTHING` on the `name` column.
/// Role-permission INSERTs use `ON CONFLICT DO NOTHING` on the composite
/// `(role_id, permission_id)` PK.  Both make the migration safe to re-run.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Helper: insert a row into `role_permissions` by resolving role and
/// permission by name via a subquery.  Idempotent (uses `WHERE NOT EXISTS`
/// to skip if the assignment already exists — portable across all backends).
async fn grant_permission(
    manager: &SchemaManager<'_>,
    role_name: &str,
    perm_name: &str,
) -> Result<(), DbErr> {
    // `INSERT ... SELECT ... ON CONFLICT DO NOTHING` doesn't translate to
    // valid MySQL/MariaDB syntax in sea_query. Use raw SQL with
    // `WHERE NOT EXISTS` which is portable across SQLite, PG, and MariaDB.
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
        let perm_id = Uuid::now_v7();

        // 1. Insert the new permission (idempotent: skip if already exists).
        //    Uses check-then-insert instead of ON CONFLICT DO NOTHING because
        //    sea-query generates invalid MySQL syntax for INSERT ... ON CONFLICT.
        //    UUIDs must be bound via sea-query (not format!) to store as BLOB on SQLite.
        {
            let exists = manager
                .get_connection()
                .query_one_raw(sea_orm::Statement::from_string(
                    manager.get_database_backend(),
                    "SELECT 1 FROM permissions WHERE name = 'manage_commands' LIMIT 1".to_owned(),
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
                                perm_id.into(),
                                "manage_commands".into(),
                                "Modify command-bearing plugin config fields (shell commands, hooks). \
                                 Grants effective code-execution authority on managed hosts."
                                    .into(),
                                now.into(),
                            ])
                            .to_owned(),
                    )
                    .await?;
            }
        }

        // 2. Grant to owner and admin (join by name to avoid hardcoding UUIDs).
        grant_permission(manager, "owner", "manage_commands").await?;
        grant_permission(manager, "admin", "manage_commands").await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Remove role-permission assignments then the permission itself.
        manager
            .exec_stmt(
                Query::delete()
                    .from_table(Alias::new("role_permissions"))
                    .and_where(
                        Expr::col(Alias::new("permission_id")).in_subquery(
                            Query::select()
                                .from(Alias::new("permissions"))
                                .column(Alias::new("id"))
                                .and_where(Expr::col(Alias::new("name")).eq("manage_commands"))
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
                    .and_where(Expr::col(Alias::new("name")).eq("manage_commands"))
                    .to_owned(),
            )
            .await?;

        Ok(())
    }
}
