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

/// Insert a permission by name.  Idempotent (`ON CONFLICT DO NOTHING` on
/// the unique `name` column).
///
/// The `perm_id` UUID is bound as a 16-byte BLOB via sea-query so that
/// SQLite stores it identically to the initial migration.  Never pass a UUID
/// as a `format!`-interpolated string — that stores TEXT instead of BLOB and
/// breaks SeaORM's uuid deserialiser.
async fn insert_permission(
    manager: &SchemaManager<'_>,
    perm_id: uuid::Uuid,
    name: &str,
    description: &str,
    now: time::OffsetDateTime,
) -> Result<(), DbErr> {
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
                .on_conflict(
                    OnConflict::column(Alias::new("name"))
                        .do_nothing()
                        .to_owned(),
                )
                .to_owned(),
        )
        .await
}

/// Grant a permission to a role by resolving both by name via a subquery.
/// Idempotent (`ON CONFLICT DO NOTHING` on the composite PK).
async fn grant_permission(
    manager: &SchemaManager<'_>,
    role_name: &str,
    perm_name: &str,
) -> Result<(), DbErr> {
    let insert = Query::insert()
        .into_table(Alias::new("role_permissions"))
        .columns([Alias::new("role_id"), Alias::new("permission_id")])
        .select_from(
            Query::select()
                .from_as(Alias::new("roles"), Alias::new("r"))
                .from_as(Alias::new("permissions"), Alias::new("p"))
                .column((Alias::new("r"), Alias::new("id")))
                .column((Alias::new("p"), Alias::new("id")))
                .and_where(Expr::col((Alias::new("r"), Alias::new("name"))).eq(role_name))
                .and_where(Expr::col((Alias::new("p"), Alias::new("name"))).eq(perm_name))
                .to_owned(),
        )
        .map_err(|e| DbErr::Migration(e.to_string()))?
        .on_conflict(
            OnConflict::columns([Alias::new("role_id"), Alias::new("permission_id")])
                .do_nothing()
                .to_owned(),
        )
        .to_owned();

    manager.exec_stmt(insert).await
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
