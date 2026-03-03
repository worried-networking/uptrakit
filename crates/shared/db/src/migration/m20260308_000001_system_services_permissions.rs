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
/// The `role_permissions` INSERTs use `INSERT OR IGNORE`.  Both make the
/// migration safe to re-run.
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
        //
        //    IMPORTANT: use Query::insert() so that sea-query binds the Uuid as a
        //    16-byte BLOB on SQLite (the same encoding used by the initial migration).
        //    execute_unprepared(&format!("… VALUES ('{id}', …)")) would embed the UUID
        //    as a 36-character TEXT literal which causes SeaORM/sqlx to fail with
        //    `ParseByteLength { len: 36 }` when reading the row back.
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
                        view_perm_id.into(),
                        "view_system_services".into(),
                        "View system services (MQTT bridge, external scheduler).".into(),
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

        // 2. Insert manage_system_services permission.
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
                        manage_perm_id.into(),
                        "manage_system_services".into(),
                        "Manage system services: approve, reject, deactivate, configure enrollment token.".into(),
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
