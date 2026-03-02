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
/// All statements use `INSERT OR IGNORE` semantics, making the migration safe
/// to re-run.
#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        let now = time::OffsetDateTime::now_utc();
        let view_perm_id = Uuid::now_v7();
        let manage_perm_id = Uuid::now_v7();

        // 1. Insert view_system_services permission.
        db.execute_unprepared(&format!(
            "INSERT OR IGNORE INTO permissions (id, name, description, created_at) \
             VALUES ('{view_perm_id}', 'view_system_services', \
                     'View system services (MQTT bridge, external scheduler).', \
                     '{now}')"
        ))
        .await?;

        // 2. Insert manage_system_services permission.
        db.execute_unprepared(&format!(
            "INSERT OR IGNORE INTO permissions (id, name, description, created_at) \
             VALUES ('{manage_perm_id}', 'manage_system_services', \
                     'Manage system services: approve, reject, deactivate, configure enrollment token.', \
                     '{now}')"
        ))
        .await?;

        // 3. Grant both to owner.
        for perm_name in ["view_system_services", "manage_system_services"] {
            db.execute_unprepared(&format!(
                "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) \
                 SELECT r.id, p.id \
                 FROM roles r, permissions p \
                 WHERE r.name = 'owner' AND p.name = '{perm_name}'"
            ))
            .await?;
        }

        // 4. Grant both to admin.
        for perm_name in ["view_system_services", "manage_system_services"] {
            db.execute_unprepared(&format!(
                "INSERT OR IGNORE INTO role_permissions (role_id, permission_id) \
                 SELECT r.id, p.id \
                 FROM roles r, permissions p \
                 WHERE r.name = 'admin' AND p.name = '{perm_name}'"
            ))
            .await?;
        }

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();

        for perm_name in ["view_system_services", "manage_system_services"] {
            db.execute_unprepared(&format!(
                "DELETE FROM role_permissions WHERE permission_id = \
                 (SELECT id FROM permissions WHERE name = '{perm_name}')"
            ))
            .await?;

            db.execute_unprepared(&format!(
                "DELETE FROM permissions WHERE name = '{perm_name}'"
            ))
            .await?;
        }

        Ok(())
    }
}
