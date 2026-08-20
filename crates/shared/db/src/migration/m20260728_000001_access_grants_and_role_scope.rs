use sea_orm_migration::prelude::*;

use super::helpers::{self, CrashRecoveryState};

/// Create `access_grants` and rescope `roles` for per-tenant custom roles
/// (`06-grant-model.md` §Storage schema / §Roles as data).
///
/// `roles` gains a nullable `tenant_id` (NULL = global built-in) and trades
/// its column-level `UNIQUE(name)` for a partial unique index pair — a single
/// composite `(tenant_id, name)` index cannot express the global scope
/// because NULL ≠ NULL in unique indexes on both SQLite and PostgreSQL.
///
/// SQLite rebuilds `roles` via the table-recreation pattern (the guide's
/// reference implementation is `m20260318_000002_cron_to_interval.rs`);
/// PostgreSQL uses plain `ALTER TABLE`. FK enforcement during recreation is
/// owned by the migration runner (see `helpers`) — but `roles` is a CASCADE
/// parent, so the rebuild additionally parks and restores its two FK
/// children (`user_roles`, `role_permissions`); see
/// [`recreate_roles_sqlite`].
///
/// `down()` is a documented best-effort dev/test reversal: once
/// M1.6a-created custom tenant roles exist, a tenant role sharing a global
/// name makes the `UNIQUE(name)` recreation fail. Production rollback is not
/// a supported path.
#[derive(DeriveMigrationName)]
pub(super) struct Migration;

#[derive(Copy, Clone, DeriveIden)]
enum AccessGrants {
    Table,
    Id,
    TenantId,
    SubjectType,
    SubjectId,
    Patterns,
    Selector,
    Description,
    CreatedAt,
    UpdatedAt,
    CreatedBy,
}

#[derive(Copy, Clone, DeriveIden)]
enum Roles {
    Table,
    Id,
    Name,
    Description,
    IsBuiltIn,
    CreatedAt,
    TenantId,
}

#[derive(DeriveIden)]
enum Tenants {
    Table,
    Id,
}

/// Data columns shared by the old and new `roles` schemas (everything except
/// the added `tenant_id`, which defaults to NULL on copy).
const ROLES_DATA_COLS: [&str; 5] = ["id", "name", "description", "is_built_in", "created_at"];

fn build_access_grants_table() -> TableCreateStatement {
    Table::create()
        .table(AccessGrants::Table)
        .col(
            ColumnDef::new(AccessGrants::Id)
                .uuid()
                .not_null()
                .primary_key(),
        )
        .col(ColumnDef::new(AccessGrants::TenantId).uuid().null())
        .col(ColumnDef::new(AccessGrants::SubjectType).text().not_null())
        .col(ColumnDef::new(AccessGrants::SubjectId).uuid().not_null())
        .col(
            ColumnDef::new(AccessGrants::Patterns)
                .json_binary()
                .not_null(),
        )
        .col(
            ColumnDef::new(AccessGrants::Selector)
                .json_binary()
                .not_null(),
        )
        .col(ColumnDef::new(AccessGrants::Description).text().null())
        .col(helpers::timestamp(AccessGrants::CreatedAt))
        .col(helpers::timestamp(AccessGrants::UpdatedAt))
        .col(ColumnDef::new(AccessGrants::CreatedBy).uuid().null())
        .foreign_key(
            ForeignKey::create()
                .name("fk_access_grants_tenant")
                .from(AccessGrants::Table, AccessGrants::TenantId)
                .to(Tenants::Table, Tenants::Id)
                .on_delete(ForeignKeyAction::Restrict),
        )
        .to_owned()
}

/// Build the `roles` schema. `scoped == true` is the new shape (nullable
/// `tenant_id` + tenant FK, no name constraint); `scoped == false` restores
/// the pre-migration shape (column-level `UNIQUE(name)`, no `tenant_id`).
fn build_roles_schema(table_name: impl IntoTableRef + Clone, scoped: bool) -> TableCreateStatement {
    let mut t = Table::create();
    t.table(table_name.clone())
        .col(ColumnDef::new(Roles::Id).uuid().not_null().primary_key());
    if scoped {
        t.col(ColumnDef::new(Roles::Name).string().not_null());
    } else {
        t.col(ColumnDef::new(Roles::Name).string().not_null().unique_key());
    }
    t.col(ColumnDef::new(Roles::Description).string().null())
        .col(
            ColumnDef::new(Roles::IsBuiltIn)
                .boolean()
                .not_null()
                .default(false),
        )
        .col(helpers::timestamp(Roles::CreatedAt));
    if scoped {
        t.col(ColumnDef::new(Roles::TenantId).uuid().null())
            .foreign_key(
                ForeignKey::create()
                    .name("fk_roles_tenant")
                    .from(table_name, Roles::TenantId)
                    .to(Tenants::Table, Tenants::Id)
                    .on_delete(ForeignKeyAction::Restrict),
            );
    }
    t.to_owned()
}

/// Build and execute `INSERT INTO <target> (<cols>) SELECT <cols> FROM <source>`
/// (the `copy_table` idiom from `m20260302_000003_host_packages_has_update.rs`;
/// copies in-database, so ids and timestamps never round-trip through Rust —
/// deliberately: the live deployment's stored values are re-emitted verbatim,
/// immune to any legacy encoding a Rust decode would trip on).
async fn copy_columns(
    manager: &SchemaManager<'_>,
    cols: &[&str],
    source: &str,
    target: &str,
) -> Result<(), DbErr> {
    let idens: Vec<Alias> = cols.iter().map(|c| Alias::new(*c)).collect();
    let select = Query::select()
        .columns(idens.clone())
        .from(Alias::new(source))
        .to_owned();

    let mut insert = Query::insert()
        .into_table(Alias::new(target))
        .columns(idens)
        .to_owned();

    insert
        .select_from(select)
        .map_err(|e| DbErr::Custom(e.to_string()))?;

    manager.execute(insert).await
}

async fn create_partial_name_indexes(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    // SQLite does not support partial indexes via sea_query's `.and_where()`,
    // so we use `execute_unprepared` with raw SQL. This is the same pattern
    // used by other migrations that need partial unique indexes
    // (m20260309_000003_host_tags.rs). The SQL is identical on both backends.
    #[expect(
        clippy::disallowed_methods,
        reason = "builder limitation: partial index with a WHERE clause is not expressible via sea_query's CREATE INDEX builder"
    )]
    manager
        .get_connection()
        .execute_unprepared(
            "CREATE UNIQUE INDEX uix_roles_global_name ON roles (name) \
             WHERE tenant_id IS NULL",
        )
        .await?;
    #[expect(
        clippy::disallowed_methods,
        reason = "builder limitation: partial index with a WHERE clause is not expressible via sea_query's CREATE INDEX builder"
    )]
    manager
        .get_connection()
        .execute_unprepared(
            "CREATE UNIQUE INDEX uix_roles_tenant_name ON roles (tenant_id, name) \
             WHERE tenant_id IS NOT NULL",
        )
        .await?;
    Ok(())
}

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(build_access_grants_table()).await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_access_grants_subject")
                    .table(AccessGrants::Table)
                    .col(AccessGrants::SubjectType)
                    .col(AccessGrants::SubjectId)
                    .to_owned(),
            )
            .await?;
        manager
            .create_index(
                Index::create()
                    .name("idx_access_grants_tenant")
                    .table(AccessGrants::Table)
                    .col(AccessGrants::TenantId)
                    .to_owned(),
            )
            .await?;

        if helpers::is_sqlite(manager) {
            recreate_roles_sqlite(manager, "roles_new", true).await?;
        } else {
            rescope_roles_postgres(manager).await?;
        }
        create_partial_name_indexes(manager).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        // Best-effort dev/test reversal — see the struct docs.
        helpers::drop_index_if_exists(manager, "uix_roles_global_name", "roles").await?;
        helpers::drop_index_if_exists(manager, "uix_roles_tenant_name", "roles").await?;

        if helpers::is_sqlite(manager) {
            recreate_roles_sqlite(manager, "roles_bak", false).await?;
        } else {
            manager
                .alter_table(
                    Table::alter()
                        .table(Roles::Table)
                        .drop_foreign_key(Alias::new("fk_roles_tenant"))
                        .to_owned(),
                )
                .await?;
            manager
                .alter_table(
                    Table::alter()
                        .table(Roles::Table)
                        .drop_column(Roles::TenantId)
                        .to_owned(),
                )
                .await?;
            // Raw SQL: sea_query's Table::alter() has no builder for adding a
            // table-level UNIQUE constraint (only FK builders exist).
            #[expect(
                clippy::disallowed_methods,
                reason = "builder limitation: ALTER TABLE ADD CONSTRAINT UNIQUE has no sea_query builder (only FK builders exist)"
            )]
            manager
                .get_connection()
                .execute_unprepared("ALTER TABLE roles ADD CONSTRAINT roles_name_key UNIQUE (name)")
                .await?;
        }

        manager
            .drop_table(Table::drop().table(AccessGrants::Table).to_owned())
            .await
    }
}

/// Parking-table schema for a `roles` FK child: plain typed columns, no PK,
/// no FKs — it exists only to hold rows across the parent rebuild.
fn build_child_bak_table(name: &str, cols: &[(&str, bool)]) -> TableCreateStatement {
    let mut t = Table::create();
    t.table(Alias::new(name));
    for (col, is_timestamp) in cols {
        if *is_timestamp {
            t.col(helpers::timestamp(Alias::new(*col)));
        } else {
            t.col(ColumnDef::new(Alias::new(*col)).uuid().not_null());
        }
    }
    t.to_owned()
}

/// SQLite path: table recreation with crash recovery (create new → copy →
/// drop old → rename), shared by `up()` (temp `roles_new`, scoped) and
/// `down()` (temp `roles_bak`, unscoped) — distinct temp names per direction
/// so crash-recovery state stays unambiguous (the
/// `HostPackagesNew`/`HostPackagesBak` precedent in m20260302_000003).
///
/// CASCADE-child preservation: `user_roles.role_id` and
/// `role_permissions.role_id` both FK `roles.id` with `ON DELETE CASCADE`.
/// When the connection has foreign keys ON (in-memory test databases — the
/// file-backed production runner migrates FK-OFF), SQLite's
/// `DROP TABLE roles` performs an implicit DELETE that fires the cascades
/// and silently wipes both children. The rebuild therefore parks both
/// children's rows in-database (`INSERT…SELECT` into `_mig_bak` tables — no
/// value ever round-trips through Rust, so legacy encodings are re-emitted
/// verbatim), explicitly empties the children so BOTH FK modes take the
/// identical path, drops/renames `roles`, copies the rows back, and drops
/// the parking tables. On the production file path the PRODUCTION RUNNER's
/// outer transaction makes the whole sequence all-or-nothing; the direct
/// `Migrator::up` test path is not transactional, so intermediate crash
/// states there (including `RenameOnly`, where the parking tables are gone)
/// are best-effort — the `RenameOnly` arm is kept for parity with the
/// crash-recovery helper contract.
async fn recreate_roles_sqlite(
    manager: &SchemaManager<'_>,
    temp_name: &str,
    scoped: bool,
) -> Result<(), DbErr> {
    let state = helpers::check_crash_recovery(manager, "roles", temp_name).await?;
    if state == CrashRecoveryState::Normal {
        manager
            .create_table(build_roles_schema(Alias::new(temp_name), scoped))
            .await?;
        copy_columns(manager, &ROLES_DATA_COLS, "roles", temp_name).await?;

        // Park the CASCADE children in-database, then empty them so the
        // parent drop cascades over nothing under either FK mode.
        for (child, bak, cols) in child_parking_plan() {
            manager
                .drop_table(Table::drop().table(Alias::new(bak)).if_exists().to_owned())
                .await?;
            manager
                .create_table(build_child_bak_table(bak, cols))
                .await?;
            let col_names: Vec<&str> = cols.iter().map(|(c, _)| *c).collect();
            copy_columns(manager, &col_names, child, bak).await?;
            manager
                .exec_stmt(Query::delete().from_table(Alias::new(child)).to_owned())
                .await?;
        }

        helpers::drop_original(manager, "roles").await?;
        helpers::rename_temp(manager, temp_name, "roles").await?;

        // Restore the children and drop the parking tables.
        for (child, bak, cols) in child_parking_plan() {
            let col_names: Vec<&str> = cols.iter().map(|(c, _)| *c).collect();
            copy_columns(manager, &col_names, bak, child).await?;
            manager
                .drop_table(Table::drop().table(Alias::new(bak)).to_owned())
                .await?;
        }
    } else {
        // RenameOnly: unreachable under the production runner's outer
        // transaction; best-effort for the non-transactional test path.
        helpers::rename_temp(manager, temp_name, "roles").await?;
    }
    Ok(())
}

/// Columns of a `roles` FK child table, paired with an is-timestamp flag.
type ChildCols = &'static [(&'static str, bool)];

/// One CASCADE-child parking plan entry: (child table, parking table, columns).
type ChildPlanEntry = (&'static str, &'static str, ChildCols);

/// (child table, parking table, columns + is-timestamp flag).
fn child_parking_plan() -> [ChildPlanEntry; 2] {
    [
        (
            "user_roles",
            "user_roles_mig_bak",
            &[
                ("tenant_id", false),
                ("user_id", false),
                ("role_id", false),
                ("assigned_at", true),
            ],
        ),
        (
            "role_permissions",
            "role_permissions_mig_bak",
            &[("role_id", false), ("permission_id", false)],
        ),
    ]
}

/// PostgreSQL path: plain `ALTER TABLE`.
async fn rescope_roles_postgres(manager: &SchemaManager<'_>) -> Result<(), DbErr> {
    manager
        .alter_table(
            Table::alter()
                .table(Roles::Table)
                .add_column(ColumnDef::new(Roles::TenantId).uuid().null())
                .to_owned(),
        )
        .await?;
    // TableForeignKey chain + `add_foreign_key(&fk)` verified against the
    // pinned sea-query 1.0.1 source (table/alter.rs:343, foreign_key/common.rs).
    let mut fk = TableForeignKey::new();
    fk.name("fk_roles_tenant")
        .from_tbl(Roles::Table)
        .from_col(Roles::TenantId)
        .to_tbl(Tenants::Table)
        .to_col(Tenants::Id)
        .on_delete(ForeignKeyAction::Restrict);
    manager
        .alter_table(
            Table::alter()
                .table(Roles::Table)
                .add_foreign_key(&fk)
                .to_owned(),
        )
        .await?;
    // `roles_name_key` is PostgreSQL's deterministic auto-name for the inline
    // UNIQUE from the initial CREATE TABLE's `string_uniq(Roles::Name)`; the
    // Docker database suite (Task 5) is the empirical proof — a wrong name
    // fails there loudly, never silently.
    #[expect(
        clippy::disallowed_methods,
        reason = "frozen merged migration: builder-expressible, but rewriting a shipped migration body risks live-vs-fresh-install divergence"
    )]
    manager
        .get_connection()
        .execute_unprepared("ALTER TABLE roles DROP CONSTRAINT roles_name_key")
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sea_orm::{
        ActiveModelTrait, ConnectOptions, ConnectionTrait, Database, DatabaseConnection, Set,
        TryGetable,
    };
    use sea_orm_migration::prelude::*;
    use uuid::Uuid;

    use crate::entity::{role, tenant};
    use crate::is_unique_constraint_violation;
    use crate::migration::Migrator;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        Database::connect(opt).await.expect("test db")
    }

    fn migration_index() -> u32 {
        <u32 as std::convert::TryFrom<usize>>::try_from(
            Migrator::migrations()
                .iter()
                .position(|m| m.name() == "m20260728_000001_access_grants_and_role_scope")
                .expect("access_grants migration must be registered"),
        )
        .expect("migration index fits u32")
    }

    fn drop_migration_index() -> u32 {
        <u32 as std::convert::TryFrom<usize>>::try_from(
            Migrator::migrations()
                .iter()
                .position(|m| m.name() == "m20260807_000001_drop_permissions_tables")
                .expect("drop migration must be registered"),
        )
        .expect("migration index fits u32")
    }

    async fn role_ids_by_name(db: &DatabaseConnection) -> BTreeMap<String, Uuid> {
        let rows = db
            .query_all(
                &Query::select()
                    .columns([Alias::new("id"), Alias::new("name")])
                    .from(Alias::new("roles"))
                    .to_owned(),
            )
            .await
            .expect("select roles");
        rows.iter()
            .map(|row| {
                let id = Uuid::try_get_by_index(row, 0).expect("role id");
                let name = String::try_get_by_index(row, 1).expect("role name");
                (name, id)
            })
            .collect()
    }

    async fn default_tenant_id(db: &DatabaseConnection) -> Uuid {
        let row = db
            .query_one(
                &Query::select()
                    .column(Alias::new("id"))
                    .from(Alias::new("tenants"))
                    .to_owned(),
            )
            .await
            .expect("tenant query")
            .expect("default tenant is seeded by the initial migration");
        Uuid::try_get_by_index(&row, 0).expect("tenant id")
    }

    async fn row_count(db: &DatabaseConnection, table: &str) -> usize {
        db.query_all(
            &Query::select()
                .column(Alias::new("role_id"))
                .from(Alias::new(table))
                .to_owned(),
        )
        .await
        .expect("count query")
        .len()
    }

    fn new_role(name: &str, tenant_id: Option<Uuid>) -> role::ActiveModel {
        role::ActiveModel {
            id: Set(Uuid::now_v7()),
            name: Set(name.to_string()),
            description: Set(None),
            is_built_in: Set(false),
            created_at: Set(time::OffsetDateTime::now_utc()),
            tenant_id: Set(tenant_id),
        }
    }

    /// Roles are reused in place: ids stable across the rebuild, and a
    /// pre-existing user_roles assignment survives untouched (the spec's
    /// "remap" test — the courtesy remap is vacuous, assignment preservation
    /// is what matters).
    #[tokio::test]
    async fn rebuild_preserves_role_ids_and_assignments() {
        let db = test_db().await;
        // Everything BEFORE this migration.
        Migrator::up(&db, Some(migration_index()))
            .await
            .expect("migrations before access_grants must apply");

        let before = role_ids_by_name(&db).await;
        assert_eq!(before.len(), 8, "granular_permissions seeds eight roles");
        let perms_before = row_count(&db, "role_permissions").await;
        assert!(
            perms_before > 0,
            "granular_permissions must have seeded role_permissions (guard is vacuous otherwise)"
        );
        let tenant_id = default_tenant_id(&db).await;
        let viewer_id = *before.get("viewer").expect("viewer role exists");

        // Seed a user + an assignment through the pre-migration schema.
        let user_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("users"))
                .columns([
                    Alias::new("id"),
                    Alias::new("email"),
                    Alias::new("first_name"),
                    Alias::new("last_name"),
                    Alias::new("is_active"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    Expr::value(user_id),
                    Expr::value("m12@example.com"),
                    Expr::value("M"),
                    Expr::value("Twelve"),
                    Expr::value(true),
                    Expr::value(now),
                    Expr::value(now),
                ])
                .to_owned(),
        )
        .await
        .expect("insert user");
        db.execute(
            &Query::insert()
                .into_table(Alias::new("user_roles"))
                .columns([
                    Alias::new("tenant_id"),
                    Alias::new("user_id"),
                    Alias::new("role_id"),
                    Alias::new("assigned_at"),
                ])
                .values_panic([
                    Expr::value(tenant_id),
                    Expr::value(user_id),
                    Expr::value(viewer_id),
                    Expr::value(now),
                ])
                .to_owned(),
        )
        .await
        .expect("insert user_roles row");

        // Run everything up to (but not including) the M1.8 drop migration
        // — the tables this test counts must still exist at the stopping
        // point. Bounded by name, never by a hardcoded step count.
        Migrator::up(&db, Some(drop_migration_index() - migration_index()))
            .await
            .expect("remaining migrations before the drop apply");

        let after = role_ids_by_name(&db).await;
        assert_eq!(before, after, "role ids must be preserved by the rebuild");
        assert_eq!(
            row_count(&db, "role_permissions").await,
            perms_before,
            "role_permissions must survive the rebuild — SQLite DROP TABLE on a \
             CASCADE parent wipes children when foreign keys are ON"
        );

        let assignment = db
            .query_one(
                &Query::select()
                    .column(Alias::new("role_id"))
                    .from(Alias::new("user_roles"))
                    .and_where(Expr::col(Alias::new("user_id")).eq(user_id))
                    .to_owned(),
            )
            .await
            .expect("select user_roles")
            .expect("assignment must survive the roles rebuild");
        let kept = Uuid::try_get_by_index(&assignment, 0).expect("role_id");
        assert_eq!(kept, viewer_id, "assignment must still point at viewer");
    }

    /// Per-scope uniqueness, both directions per index.
    #[tokio::test]
    async fn role_name_uniqueness_is_per_scope() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        let tenant_id = default_tenant_id(&db).await;

        // Duplicate global name → rejected by uix_roles_global_name.
        let err = new_role("viewer", None)
            .insert(&db)
            .await
            .expect_err("duplicate global role name must be rejected");
        assert!(
            is_unique_constraint_violation(&err),
            "expected unique violation, got: {err}"
        );

        // Same name, tenant scope → accepted.
        new_role("viewer", Some(tenant_id))
            .insert(&db)
            .await
            .expect("global + tenant scope may share a name");

        // Duplicate (tenant, name) → rejected by uix_roles_tenant_name.
        let err = new_role("viewer", Some(tenant_id))
            .insert(&db)
            .await
            .expect_err("duplicate (tenant, name) must be rejected");
        assert!(
            is_unique_constraint_violation(&err),
            "expected unique violation, got: {err}"
        );

        // Two different tenants may reuse a name.
        let other_tenant = tenant::ActiveModel {
            id: Set(Uuid::now_v7()),
            name: Set("Other".to_string()),
            slug: Set("other".to_string()),
            is_default: Set(false),
            created_at: Set(time::OffsetDateTime::now_utc()),
            updated_at: Set(time::OffsetDateTime::now_utc()),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert second tenant");
        new_role("viewer", Some(other_tenant.id))
            .insert(&db)
            .await
            .expect("a second tenant may reuse the name");
    }

    /// Both partial indexes exist with their WHERE clauses after up.
    #[tokio::test]
    async fn partial_indexes_are_partial() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("migrations apply");
        for (index_name, where_clause) in [
            ("uix_roles_global_name", "WHERE tenant_id IS NULL"),
            ("uix_roles_tenant_name", "WHERE tenant_id IS NOT NULL"),
        ] {
            let row = db
                .query_one(
                    &Query::select()
                        .column(Alias::new("sql"))
                        .from(Alias::new("sqlite_master"))
                        .and_where(Expr::col(Alias::new("type")).eq("index"))
                        .and_where(Expr::col(Alias::new("name")).eq(index_name))
                        .to_owned(),
                )
                .await
                .expect("index lookup should succeed")
                .expect("partial index should exist");
            let sql = String::try_get_by_index(&row, 0).expect("index sql");
            assert!(
                sql.contains(where_clause),
                "{index_name} must carry `{where_clause}`; got: {sql}"
            );
        }
    }

    /// up → down → up round-trip (guards the table-recreation reversal). The
    /// down() content assertions live in the seed migration's own tests.
    /// Post-M1.8, `role_permissions` is empty at tip, so the down-direction
    /// child-parking data-survival guard rides on `user_roles` (the other
    /// `child_parking_plan()` entry) via a seeded assignment.
    #[tokio::test]
    async fn up_down_up_round_trips() {
        let db = test_db().await;
        Migrator::up(&db, None).await.expect("up");

        // Seed one user + assignment at tip so the down-chain parking has a
        // surviving data row to prove itself on.
        let tenant_id = default_tenant_id(&db).await;
        let viewer_id = *role_ids_by_name(&db)
            .await
            .get("viewer")
            .expect("viewer role exists");
        let user_id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        db.execute(
            &Query::insert()
                .into_table(Alias::new("users"))
                .columns([
                    Alias::new("id"),
                    Alias::new("email"),
                    Alias::new("first_name"),
                    Alias::new("last_name"),
                    Alias::new("is_active"),
                    Alias::new("created_at"),
                    Alias::new("updated_at"),
                ])
                .values_panic([
                    Expr::value(user_id),
                    Expr::value("m18@example.com"),
                    Expr::value("M"),
                    Expr::value("Eighteen"),
                    Expr::value(true),
                    Expr::value(now),
                    Expr::value(now),
                ])
                .to_owned(),
        )
        .await
        .expect("insert user");
        db.execute(
            &Query::insert()
                .into_table(Alias::new("user_roles"))
                .columns([
                    Alias::new("tenant_id"),
                    Alias::new("user_id"),
                    Alias::new("role_id"),
                    Alias::new("assigned_at"),
                ])
                .values_panic([
                    Expr::value(tenant_id),
                    Expr::value(user_id),
                    Expr::value(viewer_id),
                    Expr::value(now),
                ])
                .to_owned(),
        )
        .await
        .expect("insert user_roles row");

        // Roll back every migration from this one onward — computed
        // dynamically via `migration_index()` rather than hardcoded, so
        // appending a future migration after this one cannot silently
        // under-roll-back.
        let total = Migrator::migrations().len();
        let steps = total - usize::try_from(migration_index()).expect("index fits usize");
        let steps = <u32 as std::convert::TryFrom<usize>>::try_from(steps).expect("steps fits u32");
        Migrator::down(&db, Some(steps))
            .await
            .expect("down must revert cleanly");
        // access_grants must be gone after down.
        let gone = db
            .query_one(
                &Query::select()
                    .column(Alias::new("name"))
                    .from(Alias::new("sqlite_master"))
                    .and_where(Expr::col(Alias::new("type")).eq("table"))
                    .and_where(Expr::col(Alias::new("name")).eq("access_grants"))
                    .to_owned(),
            )
            .await
            .expect("sqlite_master query");
        assert!(gone.is_none(), "access_grants must be dropped by down");
        // The M1.8 drop's down() recreated role_permissions (schema-only),
        // so the parking path below it ran against a real table…
        let rp = db
            .query_one(
                &Query::select()
                    .column(Alias::new("name"))
                    .from(Alias::new("sqlite_master"))
                    .and_where(Expr::col(Alias::new("type")).eq("table"))
                    .and_where(Expr::col(Alias::new("name")).eq("role_permissions"))
                    .to_owned(),
            )
            .await
            .expect("sqlite_master query");
        assert!(
            rp.is_some(),
            "role_permissions must exist after down (recreated by the drop's down())"
        );
        // …and the surviving child-parking entry proves data survival.
        assert_eq!(
            row_count(&db, "user_roles").await,
            1,
            "user_roles row must survive the down-direction rebuild"
        );
        Migrator::up(&db, None)
            .await
            .expect("re-apply after rollback");
        for table in ["permissions", "role_permissions"] {
            let present = db
                .query_one(
                    &Query::select()
                        .column(Alias::new("name"))
                        .from(Alias::new("sqlite_master"))
                        .and_where(Expr::col(Alias::new("type")).eq("table"))
                        .and_where(Expr::col(Alias::new("name")).eq(table))
                        .to_owned(),
                )
                .await
                .expect("sqlite_master query");
            assert!(present.is_none(), "{table} must be dropped again at tip");
        }
        assert_eq!(
            row_count(&db, "user_roles").await,
            1,
            "user_roles row must survive the full round trip"
        );
    }
}
