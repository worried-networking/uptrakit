use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Migrate from coarse-grained permissions (16) and 3 built-in roles to
/// granular permissions (32) and 8 built-in roles.
///
/// ## Changes
///
/// 1. Add `is_built_in` column to `roles` table (default FALSE, then mark
///    existing roles as built-in).
/// 2. Insert 22 new permissions (32 total − 10 kept = 22 new).
/// 3. Create 8 new built-in roles with their permission assignments.
/// 4. Migrate existing user role assignments:
///    - `owner` → all 8 new roles
///    - `admin` → viewer, service_manager, software_manager, host_manager,
///      settings_manager, command_manager
///    - `user` → viewer
/// 5. Remove old role-permission assignments, user-role assignments, old roles,
///    and old permissions.
///
/// ## Idempotency
///
/// All INSERTs use `ON CONFLICT DO NOTHING` on the appropriate unique columns.
/// DELETEs are filtered by name, so re-running is safe.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

// ─── Helpers ─────────────────────────────────────────────────────────────

/// Insert a permission by name. Idempotent (check-then-insert).
///
/// Uses check-then-insert instead of ON CONFLICT DO NOTHING because
/// sea-query's ON CONFLICT support is not portable across all backends.
/// UUIDs must be bound via sea-query (not format!) to store as BLOB on SQLite.
async fn insert_permission(
    manager: &SchemaManager<'_>,
    perm_id: Uuid,
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

/// Insert a role by name. Idempotent (check-then-insert).
async fn insert_role(
    manager: &SchemaManager<'_>,
    role_id: Uuid,
    name: &str,
    description: &str,
    now: time::OffsetDateTime,
) -> Result<(), DbErr> {
    let exists = manager
        .get_connection()
        .query_one_raw(sea_orm::Statement::from_string(
            manager.get_database_backend(),
            format!("SELECT 1 FROM roles WHERE name = '{name}' LIMIT 1"),
        ))
        .await?;
    if exists.is_some() {
        return Ok(());
    }
    manager
        .exec_stmt(
            Query::insert()
                .into_table(Alias::new("roles"))
                .columns([
                    Alias::new("id"),
                    Alias::new("name"),
                    Alias::new("description"),
                    Alias::new("is_built_in"),
                    Alias::new("created_at"),
                ])
                .values_panic([
                    role_id.into(),
                    name.into(),
                    description.into(),
                    true.into(),
                    now.into(),
                ])
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

/// Migrate user_role assignments from an old role to a set of new roles.
///
/// For every `user_role` row referencing `old_role_name`, insert new rows for
/// each `new_role_name` (preserving tenant_id, user_id, assigned_at).
/// Uses `WHERE NOT EXISTS` for idempotency — portable across all backends.
async fn migrate_user_roles(
    manager: &SchemaManager<'_>,
    old_role_name: &str,
    new_role_names: &[&str],
) -> Result<(), DbErr> {
    for new_role_name in new_role_names {
        let sql = format!(
            "INSERT INTO user_roles (tenant_id, user_id, role_id, assigned_at) \
             SELECT ur.tenant_id, ur.user_id, new_r.id, ur.assigned_at \
             FROM user_roles ur \
             INNER JOIN roles old_r ON old_r.id = ur.role_id \
             CROSS JOIN roles new_r \
             WHERE old_r.name = '{old_role_name}' AND new_r.name = '{new_role_name}' \
             AND NOT EXISTS ( \
               SELECT 1 FROM user_roles ur2 \
               WHERE ur2.tenant_id = ur.tenant_id \
                 AND ur2.user_id = ur.user_id \
                 AND ur2.role_id = new_r.id \
             )"
        );
        manager.get_connection().execute_unprepared(&sql).await?;
    }
    Ok(())
}

/// Delete all user_role rows referencing roles with the given names.
async fn delete_user_roles_for_roles(
    manager: &SchemaManager<'_>,
    role_names: &[&str],
) -> Result<(), DbErr> {
    manager
        .exec_stmt(
            Query::delete()
                .from_table(Alias::new("user_roles"))
                .and_where(
                    Expr::col(Alias::new("role_id")).in_subquery(
                        Query::select()
                            .from(Alias::new("roles"))
                            .column(Alias::new("id"))
                            .and_where(Expr::col(Alias::new("name")).is_in(role_names.to_vec()))
                            .to_owned(),
                    ),
                )
                .to_owned(),
        )
        .await
}

/// Delete all role_permission rows referencing roles with the given names.
async fn delete_role_permissions_for_roles(
    manager: &SchemaManager<'_>,
    role_names: &[&str],
) -> Result<(), DbErr> {
    manager
        .exec_stmt(
            Query::delete()
                .from_table(Alias::new("role_permissions"))
                .and_where(
                    Expr::col(Alias::new("role_id")).in_subquery(
                        Query::select()
                            .from(Alias::new("roles"))
                            .column(Alias::new("id"))
                            .and_where(Expr::col(Alias::new("name")).is_in(role_names.to_vec()))
                            .to_owned(),
                    ),
                )
                .to_owned(),
        )
        .await
}

/// Delete roles by name.
async fn delete_roles(manager: &SchemaManager<'_>, role_names: &[&str]) -> Result<(), DbErr> {
    manager
        .exec_stmt(
            Query::delete()
                .from_table(Alias::new("roles"))
                .and_where(Expr::col(Alias::new("name")).is_in(role_names.to_vec()))
                .to_owned(),
        )
        .await
}

/// Delete permissions by name and their role_permission references.
async fn delete_permissions(manager: &SchemaManager<'_>, perm_names: &[&str]) -> Result<(), DbErr> {
    // Delete role_permission rows first (FK constraint).
    manager
        .exec_stmt(
            Query::delete()
                .from_table(Alias::new("role_permissions"))
                .and_where(
                    Expr::col(Alias::new("permission_id")).in_subquery(
                        Query::select()
                            .from(Alias::new("permissions"))
                            .column(Alias::new("id"))
                            .and_where(Expr::col(Alias::new("name")).is_in(perm_names.to_vec()))
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
                .and_where(Expr::col(Alias::new("name")).is_in(perm_names.to_vec()))
                .to_owned(),
        )
        .await
}

// ─── New roles and their permission assignments ──────────────────────────

/// Role definitions: (name, description, permissions).
const NEW_ROLES: &[(&str, &str, &[&str])] = &[
    (
        "viewer",
        "Read-only baseline access",
        &[
            "view_services",
            "view_software",
            "view_hosts",
            "view_settings",
        ],
    ),
    (
        "operator",
        "Day-to-day operations (approve/reject services, trigger checks/updates)",
        &[
            "approve_services",
            "reject_services",
            "trigger_checks",
            "trigger_updates",
        ],
    ),
    (
        "service_manager",
        "Full service lifecycle management",
        &[
            "approve_services",
            "reject_services",
            "remove_services",
            "update_services",
        ],
    ),
    (
        "software_manager",
        "Full software lifecycle management",
        &[
            "create_software",
            "update_software",
            "delete_software",
            "trigger_checks",
            "trigger_updates",
            "manage_scheduler",
            "manage_ignores",
        ],
    ),
    (
        "host_manager",
        "Full host management",
        &["update_hosts", "deactivate_hosts"],
    ),
    (
        "settings_manager",
        "Tenant administration",
        &[
            "manage_auth_settings",
            "manage_enrollment_tokens",
            "manage_agent_certs",
            "view_notifications",
            "manage_notifications",
            "view_audit_logs",
            "manage_users",
        ],
    ),
    (
        "command_manager",
        "Security-sensitive command config management",
        &["manage_commands"],
    ),
    (
        "system_administrator",
        "Infrastructure management",
        &[
            "manage_global_settings",
            "view_system_services",
            "approve_system_services",
            "reject_system_services",
            "remove_system_services",
            "update_system_services",
            "view_system_audit_logs",
        ],
    ),
];

/// Old role → new roles mapping for user migration.
const OLD_OWNER_NEW_ROLES: &[&str] = &[
    "viewer",
    "operator",
    "service_manager",
    "software_manager",
    "host_manager",
    "settings_manager",
    "command_manager",
    "system_administrator",
];

const OLD_ADMIN_NEW_ROLES: &[&str] = &[
    "viewer",
    "service_manager",
    "software_manager",
    "host_manager",
    "settings_manager",
    "command_manager",
];

const OLD_USER_NEW_ROLES: &[&str] = &["viewer"];

/// Permissions to remove (replaced by granular alternatives).
const OLD_PERMISSIONS: &[&str] = &[
    "view_agents",
    "manage_agents",
    "manage_software",
    "manage_hosts",
    "manage_settings",
    "manage_system_services",
];

/// Old roles to remove after migration.
const OLD_ROLES: &[&str] = &["owner", "admin", "user"];

// ─── New permissions (22 new ones) ───────────────────────────────────────

const NEW_PERMISSIONS: &[(&str, &str)] = &[
    ("view_services", "View tenant services and their status"),
    ("approve_services", "Approve pending service enrollments"),
    ("reject_services", "Reject pending service enrollments"),
    ("remove_services", "Deactivate/remove services"),
    (
        "update_services",
        "Update service settings (ping interval, freeze, merge)",
    ),
    ("approve_system_services", "Approve pending system services"),
    ("reject_system_services", "Reject pending system services"),
    ("remove_system_services", "Deactivate system services"),
    ("update_system_services", "Update system service settings"),
    (
        "create_software",
        "Create software items and plugin configs",
    ),
    ("update_software", "Edit software items and plugin configs"),
    (
        "delete_software",
        "Delete software items and plugin configs",
    ),
    ("trigger_checks", "Trigger version checks and autodiscovery"),
    (
        "trigger_updates",
        "Trigger update execution (single and batch)",
    ),
    ("manage_scheduler", "Manage scheduled tasks"),
    ("update_hosts", "Update host properties and tags"),
    ("deactivate_hosts", "Deactivate hosts"),
    (
        "manage_auth_settings",
        "Manage registration, authentication, OIDC providers",
    ),
    (
        "manage_enrollment_tokens",
        "Manage tenant enrollment tokens",
    ),
    ("manage_agent_certs", "Manage agent certificate settings"),
    ("manage_users", "Manage user roles and access"),
    ("manage_ignores", "Manage autodiscovery ignore rules"),
];

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let now = time::OffsetDateTime::now_utc();

        // ── Step 1: Add is_built_in column to roles ──────────────────────
        manager
            .alter_table(
                Table::alter()
                    .table(Alias::new("roles"))
                    .add_column(
                        ColumnDef::new(Alias::new("is_built_in"))
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .to_owned(),
            )
            .await?;

        // Mark existing roles as built-in.
        for role_name in OLD_ROLES {
            manager
                .exec_stmt(
                    Query::update()
                        .table(Alias::new("roles"))
                        .value(Alias::new("is_built_in"), true)
                        .and_where(Expr::col(Alias::new("name")).eq(*role_name))
                        .to_owned(),
                )
                .await?;
        }

        // ── Step 2: Insert new permissions ───────────────────────────────
        for (name, description) in NEW_PERMISSIONS {
            insert_permission(manager, Uuid::now_v7(), name, description, now).await?;
        }

        // ── Step 3: Create new built-in roles ────────────────────────────
        for (name, description, permissions) in NEW_ROLES {
            insert_role(manager, Uuid::now_v7(), name, description, now).await?;
            for perm_name in *permissions {
                grant_permission(manager, name, perm_name).await?;
            }
        }

        // ── Step 4: Migrate user assignments ─────────────────────────────
        migrate_user_roles(manager, "owner", OLD_OWNER_NEW_ROLES).await?;
        migrate_user_roles(manager, "admin", OLD_ADMIN_NEW_ROLES).await?;
        migrate_user_roles(manager, "user", OLD_USER_NEW_ROLES).await?;

        // ── Step 5: Clean up old data ────────────────────────────────────
        // Delete old user_role entries (must come before deleting roles).
        delete_user_roles_for_roles(manager, OLD_ROLES).await?;
        // Delete old role_permission entries.
        delete_role_permissions_for_roles(manager, OLD_ROLES).await?;
        // Delete old roles.
        delete_roles(manager, OLD_ROLES).await?;
        // Delete old permissions (and their remaining role_permission refs).
        delete_permissions(manager, OLD_PERMISSIONS).await?;

        Ok(())
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        // Destructive migration — reversal not supported.
        Err(DbErr::Migration(
            "Granular permissions migration cannot be reversed".to_string(),
        ))
    }
}
