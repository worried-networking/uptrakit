use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add `view_system_services` and `manage_system_services` permissions and
/// assign them to the `owner` and `admin` roles.
///
/// ## Role assignments
///
/// - `owner`: granted both `view_system_services` and `manage_system_services`
/// - `admin`: granted both `view_system_services` and `manage_system_services`
/// - `user`: not granted (view-only role does not include system services)
///
/// ## Idempotency
///
/// Permission INSERTs use `ON CONFLICT DO NOTHING` on the `name` column.
/// Role-permission INSERTs use `ON CONFLICT DO NOTHING` on the composite
/// `(role_id, permission_id)` PK.  Both make the migration safe to re-run.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

/// Insert a permission by name.  Idempotent (check-then-insert).
///
/// Uses check-then-insert instead of ON CONFLICT DO NOTHING because
/// sea-query generates invalid MySQL syntax for INSERT ... ON CONFLICT.
/// UUIDs must be bound via sea-query (not format!) to store as BLOB on SQLite.
async fn insert_permission(
    manager: &SchemaManager<'_>,
    perm_id: uuid::Uuid,
    name: &str,
    description: &str,
    now: time::OffsetDateTime,
) -> Result<(), DbErr> {
    let exists = manager
        .get_connection()
        .query_one_raw(sea_orm::Statement::from_string(
            manager.get_database_backend(),
            format!("SELECT 1 FROM permissions WHERE name = '{name}' LIMIT 1"),
        ))
        .await?;
    if exists.is_some() {
        return Ok(());
    }
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
                .values_panic([perm_id.into(), name.into(), description.into(), now.into()])
                .to_owned(),
        )
        .await
}

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
        let view_perm_id = Uuid::now_v7();
        let manage_perm_id = Uuid::now_v7();

        insert_permission(
            manager,
            view_perm_id,
            "view_system_services",
            "View system services (MQTT bridge, external scheduler).",
            now,
        )
        .await?;

        insert_permission(
            manager,
            manage_perm_id,
            "manage_system_services",
            "Manage system services: approve, reject, deactivate, configure enrollment token.",
            now,
        )
        .await?;

        for role in ["owner", "admin"] {
            grant_permission(manager, role, "view_system_services").await?;
            grant_permission(manager, role, "manage_system_services").await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for perm_name in ["view_system_services", "manage_system_services"] {
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
