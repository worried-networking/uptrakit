use sea_orm_migration::prelude::*;
use uuid::Uuid;

/// Add `view_audit_logs` and `view_system_audit_logs` permissions and assign
/// them to the appropriate roles.
///
/// ## Role assignments
///
/// - `owner`: granted both `view_audit_logs` and `view_system_audit_logs`
/// - `admin`: granted `view_audit_logs` only
/// - `user`: not granted either permission
///
/// ## Rationale
///
/// Tenant audit logs (`GET /api/v1/audit-logs`) record all authenticated
/// requests within a tenant. Both owner and admin need visibility.
///
/// System audit logs (`GET /api/v1/system-audit-logs`) record
/// infrastructure-scoped operations (CA rotation, global settings changes,
/// etc.).  Only the owner role receives this permission.
///
/// ## Idempotency
///
/// Permission INSERTs use `ON CONFLICT DO NOTHING` on the `name` column.
/// Role-permission INSERTs use `ON CONFLICT DO NOTHING` on the composite
/// `(role_id, permission_id)` PK.  Both make the migration safe to re-run.
#[derive(DeriveMigrationName)]
pub struct Migration;

/// Insert a permission by name.  Idempotent (`ON CONFLICT DO NOTHING` on
/// the unique `name` column).
///
/// The `perm_id` UUID is bound as a 16-byte BLOB via sea-query so that
/// SQLite stores it identically to the initial migration.  Never pass a UUID
/// as a `format!`-interpolated string — that stores TEXT instead of BLOB and
/// breaks SeaORM's uuid deserialiser.
async fn insert_permission(
    manager: &SchemaManager<'_>,
    perm_id: Uuid,
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
        let view_audit_logs_id = Uuid::now_v7();
        let view_system_audit_logs_id = Uuid::now_v7();

        insert_permission(
            manager,
            view_audit_logs_id,
            "view_audit_logs",
            "View tenant-scoped audit log entries (GET /api/v1/audit-logs).",
            now,
        )
        .await?;

        insert_permission(
            manager,
            view_system_audit_logs_id,
            "view_system_audit_logs",
            "View system-level audit log entries: CA rotation, global settings changes, \
             and other infrastructure-scoped operations (GET /api/v1/system-audit-logs).",
            now,
        )
        .await?;

        // view_audit_logs → owner and admin.
        for role in ["owner", "admin"] {
            grant_permission(manager, role, "view_audit_logs").await?;
        }

        // view_system_audit_logs → owner only.
        grant_permission(manager, "owner", "view_system_audit_logs").await?;

        Ok(())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        for perm_name in ["view_audit_logs", "view_system_audit_logs"] {
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
