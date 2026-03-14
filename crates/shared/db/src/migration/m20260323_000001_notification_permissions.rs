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

        // Insert view_notifications permission (idempotent).
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
                        "view_notifications".into(),
                        "View notification channels, rules, and delivery log.".into(),
                        now.into(),
                    ])
                    .on_conflict(
                        OnConflict::column(Alias::new("name"))
                            .do_nothing()
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;

        // Insert manage_notifications permission (idempotent).
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
                        "manage_notifications".into(),
                        "Create, update, delete, and test notification channels and rules.".into(),
                        now.into(),
                    ])
                    .on_conflict(
                        OnConflict::column(Alias::new("name"))
                            .do_nothing()
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;

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
