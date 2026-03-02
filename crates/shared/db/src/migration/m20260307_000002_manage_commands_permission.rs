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
/// All statements use `INSERT OR IGNORE` (SQLite) / `INSERT IGNORE` (MySQL)
/// / `ON CONFLICT DO NOTHING` (PostgreSQL) semantics, making the migration
/// safe to re-run.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let now = time::OffsetDateTime::now_utc();
        let perm_id = Uuid::now_v7();

        // 1. Insert the new permission (idempotent: ignore if already exists).
        //    The permission name must match `Permission::ManageCommands::as_str()`.
        db.execute_unprepared(&format!(
            "INSERT OR IGNORE INTO permissions (id, name, description, created_at) \
             VALUES ('{perm_id}', 'manage_commands', \
                     'Modify command-bearing plugin config fields (shell commands, hooks). \
                      Grants effective code-execution authority on managed hosts.', \
                     '{now}')"
        ))
        .await?;

        // 2. Grant to owner — join roles and permissions by name to avoid
        //    hardcoding UUIDs.  INSERT OR IGNORE makes this idempotent.
        db.execute_unprepared(
            "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) \
             SELECT r.id, p.id \
             FROM roles r, permissions p \
             WHERE r.name = 'owner' AND p.name = 'manage_commands'",
        )
        .await?;

        // 3. Grant to admin.
        db.execute_unprepared(
            "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) \
             SELECT r.id, p.id \
             FROM roles r, permissions p \
             WHERE r.name = 'admin' AND p.name = 'manage_commands'",
        )
        .await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        // Remove role-permission assignments then the permission itself.
        db.execute_unprepared(
            "DELETE FROM role_permissions WHERE permission_id = \
             (SELECT id FROM permissions WHERE name = 'manage_commands')",
        )
        .await?;

        db.execute_unprepared("DELETE FROM permissions WHERE name = 'manage_commands'")
            .await?;

        Ok(())
    }
}
