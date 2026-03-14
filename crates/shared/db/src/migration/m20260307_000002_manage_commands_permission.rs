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
/// permission by name via a subquery.  Idempotent (`ON CONFLICT DO NOTHING`
/// on the composite PK).
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
        let perm_id = Uuid::now_v7();

        // 1. Insert the new permission (idempotent: ignore if already exists).
        //    The permission name must match `Permission::ManageCommands::as_str()`.
        //
        //    Use Query::insert() so that sea-query binds the Uuid as a 16-byte
        //    BLOB on SQLite.  execute_unprepared(&format!("VALUES ('{id}', …)"))
        //    would embed the UUID as a 36-char TEXT literal, causing SeaORM to
        //    fail with `ParseByteLength { len: 36 }` when reading it back.
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
                    .on_conflict(
                        OnConflict::column(Alias::new("name"))
                            .do_nothing()
                            .to_owned(),
                    )
                    .to_owned(),
            )
            .await?;

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
