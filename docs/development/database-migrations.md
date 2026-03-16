# Database Migrations

This guide covers everything a contributor needs to know to write, register, and test database
migrations for Uptrakit.

See also: [Testing](testing.md) for the test DB helper pattern.

---

## Where migrations live

All migration source files are in `crates/shared/db/src/migration/`.

The module is gated behind the `migration` feature flag of `uptrakit-shared-db`:

```toml
# In crates that need to run migrations (e.g. the controller binary):
uptrakit-shared-db = { path = "../../shared/db", features = ["migration"] }

# In test code only (e.g. web-api dev-dependencies):
[dev-dependencies]
uptrakit-shared-db = { path = "../../shared/db", features = ["migration", "db-sqlite"] }
```

The public entry point exposed by the feature is:

```rust
uptrakit_shared_db::migration::run_migrations(&db).await?;
```

The controller's `src/migration/mod.rs` is a thin wrapper that calls this function and maps the
error into the controller's `DbError` type. **Do not add migration files to the controller crate.**

---

## Naming convention

Migration filenames follow SeaORM's standard convention:

```text
m{YYYYMMDD}_{6-digit-sequence}_{short_description}.rs
```

Examples:

```text
m20260209_000001_initial.rs
m20260226_000001_add_some_feature.rs
```

- The date is the wall-clock date when you write the migration (UTC).
- The sequence resets to `000001` each day; increment it if multiple migrations land on the same
  day.
- The description uses `_` as a separator and should be short but unambiguous.

---

## Migration skeleton

Every migration file follows this pattern:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(MyTable::Table)
                    .if_not_exists()
                    // ... columns ...
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(MyTable::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum MyTable {
    Table,
    Id,
    // ... other columns ...
}
```

---

## Timestamp Columns

All entity columns that store point-in-time values must use `time::OffsetDateTime`, backed by
SeaORM's `.timestamp()` column type (mapped to `TEXT` in SQLite via RFC 3339, and to the native
`TIMESTAMP` type on PostgreSQL and MySQL).

```rust
// ✓ Correct
.col(ColumnDef::new(MyTable::CreatedAt).timestamp().not_null())
.col(ColumnDef::new(MyTable::UpdatedAt).timestamp().not_null())

// ✗ Wrong — i64 unix timestamps silently store 0 on clock error; lose timezone info
.col(ColumnDef::new(MyTable::CreatedAt).integer().not_null())
```

In entity models, use `time::OffsetDateTime` directly:

```rust
// ✓ Correct
pub created_at: time::OffsetDateTime,
pub updated_at: time::OffsetDateTime,

// ✗ Wrong — i64 unix timestamps are not acceptable for entity fields
pub created_at: i64,
```

Always stamp new rows with `time::OffsetDateTime::now_utc()`. Never use `SystemTime::now()` with
`.unwrap_or(0)`: a system-clock error silently produces the Unix epoch (1970-01-01), which is
indistinguishable from a real timestamp and makes stale-detection logic incorrect.

### Converting existing INTEGER timestamps

If an existing table stores timestamps as `INTEGER` (unix seconds), write a data migration using
SQLite's `strftime` function. This is one of the approved `execute_unprepared()` exceptions
because `strftime` is a SQLite-specific built-in that sea_query cannot express:

```rust
// `strftime` is a SQLite-specific function with no sea_query equivalent.
// execute_unprepared is the approved exception for this pattern.
manager
    .get_connection()
    .execute_unprepared(
        "INSERT INTO my_table_new SELECT id, name, \
         strftime('%Y-%m-%dT%H:%M:%S+00:00', created_at, 'unixepoch'), \
         strftime('%Y-%m-%dT%H:%M:%S+00:00', updated_at, 'unixepoch') \
         FROM my_table",
    )
    .await?;
```

The reverse (`down()`) uses `CAST(strftime('%s', col) AS INTEGER)`.

---

## Foreign Key and Sort Column Indexing

Every column that appears in a `WHERE`, `JOIN ON`, or `ORDER BY` clause must have a standalone
index. SeaORM's `Index::create()` is the preferred API:

```rust
manager
    .create_index(
        Index::create()
            .name("idx_my_table_fk_col")
            .table(MyTable::Table)
            .col(MyTable::FkCol)
            .to_owned(),
    )
    .await?;
```

Always drop the index in `down()`:

```rust
manager
    .drop_index(
        Index::drop()
            .name("idx_my_table_fk_col")
            .table(MyTable::Table)
            .to_owned(),
    )
    .await?;
```

### Avoiding duplicate indexes

A `UNIQUE` constraint (created via `.unique_key()` on a column, `string_uniq()`, or
`Index::create().unique()`) already creates an implicit index in all three supported
backends (SQLite, PostgreSQL, MySQL). Do **not** add a separate non-unique index on the same
column — it wastes disk space and slows every write:

```rust
// ✗ Wrong — the UNIQUE key already creates an index on "slug"
.col(ColumnDef::new(Tenant::Slug).string().not_null().unique_key())
// ... and then separately ...
manager.create_index(
    Index::create().name("idx_tenants_slug").table(Tenant::Table).col(Tenant::Slug)
        .to_owned(),
).await?;

// ✓ Correct — the unique constraint's implicit index is sufficient
.col(ColumnDef::new(Tenant::Slug).string().not_null().unique_key())
```

The only case where an explicit index alongside a unique constraint is justified is a
**partial (filtered) unique index**. Use `Index::create()` with `.and_where()` from the
`ConditionalStatement` trait (re-exported via `sea_orm_migration::prelude::*`):

```rust
manager
    .create_index(
        Index::create()
            .name("uq_plugin_configs_active_name")
            .table(PluginConfigs::Table)
            .col(PluginConfigs::TenantId)
            .col(PluginConfigs::Name)
            .unique()
            .and_where(Expr::col(PluginConfigs::DeactivatedAt).is_null())
            .to_owned(),
    )
    .await?;
```

> **MySQL/MariaDB note:** MySQL/MariaDB do not support partial indexes. Branch on the backend
> to create a composite unique index including the filtered column instead. See
> [MySQL/MariaDB Compatibility Workarounds](#mysqlmariadb-compatibility-workarounds) for details.

### Composite indexes for multi-column queries

When a query filters or sorts on multiple columns together (e.g.
`WHERE user_id = ? ORDER BY expires_at`), create a composite index covering both columns
in query order:

```rust
manager
    .create_index(
        Index::create()
            .name("idx_sessions_user_expires")
            .table(Sessions::Table)
            .col(Sessions::UserId)       // equality filter — leftmost
            .col(Sessions::ExpiresAt)    // range/sort — second
            .to_owned(),
    )
    .await?;
```

Column order matters: put the equality-filtered column first; the range or sort column
second. The composite index also supports single-column lookups on the leftmost column.

### Checklist when adding a new table

For each new table, verify:

- Primary key column — always indexed (implicit for `PRIMARY KEY`)
- `tenant_id` column — included in composite PK or has a standalone index
- Every FK column referenced in `SELECT … WHERE fk = ?` or `JOIN … ON a.fk = b.id`
- Any column used in `ORDER BY` in API list endpoints (e.g. `created_at`)

---

## Composite foreign keys

When a table references another table via a composite primary key (e.g. `(host_id, software_item_id)`),
use `from_tbl`/`from_col`/`to_tbl`/`to_col` chaining on `ForeignKey::create()`:

```rust
manager
    .create_table(
        Table::create()
            .table(HostSoftwareItemPlugins::Table)
            .if_not_exists()
            // ... columns ...
            .foreign_key(
                &mut ForeignKey::create()
                    .name("fk_hsip_host_software_item")
                    .from_tbl(HostSoftwareItemPlugins::Table)
                    .from_col(HostSoftwareItemPlugins::HostId)
                    .from_col(HostSoftwareItemPlugins::SoftwareItemId)
                    .to_tbl(HostSoftwareItems::Table)
                    .to_col(HostSoftwareItems::HostId)
                    .to_col(HostSoftwareItems::SoftwareItemId)
                    .on_delete(ForeignKeyAction::Cascade)
                    .to_owned(),
            )
            .to_owned(),
    )
    .await?;
```

The single-column `.from(table, col)` / `.to(table, col)` shorthand only supports one column.
For composite keys, chain multiple `.from_col()` / `.to_col()` calls after `.from_tbl()` /
`.to_tbl()`.

---

## No raw SQL — use sea_query builders for DML

**Raw SQL strings are banned everywhere in this codebase**, including migrations and tests.
All DML (`INSERT`, `UPDATE`, `DELETE`, `SELECT`) that cannot be expressed with the SeaORM entity
API must use the **sea_query typed builder API** instead of `execute_unprepared()` or
`format!()`-constructed strings.

### Why

- Raw strings bypass the type system — a renamed column silently breaks a query at runtime.
- `format!()` SQL is a SQL-injection vector.
- sea_query builders adapt automatically to the active database dialect (SQLite / PostgreSQL /
  MySQL).

### UUID values must be bound as BLOB — never interpolated as strings

This is the single most important rule for SQLite migrations.

SeaORM/sqlx reads `uuid`-typed columns via `sqlite3_column_blob()`. When a UUID is stored as a
36-character TEXT string (e.g. the result of `format!("'{uuid}'")`), the read fails at runtime
with `ParseByteLength { len: 36 }`.

**Always** pass UUID values through `uuid.into()` inside `values_panic([…])`. sea-query then
binds it as `Value::Uuid` which SQLite stores as a 16-byte BLOB — the same encoding the entity
reader expects.

```rust
// ✓ Correct — Uuid bound as 16-byte BLOB
manager.exec_stmt(
    Query::insert()
        .into_table(Alias::new("permissions"))
        .columns([Alias::new("id"), Alias::new("name")])
        .values_panic([Uuid::now_v7().into(), "my_perm".into()])
        .to_owned(),
)
.await?;

// ✗ WRONG — UUID embedded as TEXT literal; breaks SeaORM reads
db.execute_unprepared(&format!(
    "INSERT INTO permissions (id, name) VALUES ('{uuid}', 'my_perm')"
))
.await?;
```

### INSERT via sea_query

In a `MigrationTrait` impl use `manager.exec_stmt(stmt)`, which takes ownership of the built
statement:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveIden)]
enum MyTable {
    Table,
    Id,
    Name,
    CreatedAt,
}

manager
    .exec_stmt(
        Query::insert()
            .into_table(MyTable::Table)
            .columns([MyTable::Id, MyTable::Name, MyTable::CreatedAt])
            .values_panic([Uuid::now_v7().into(), "example".into(), now.into()])
            .to_owned(),
    )
    .await?;
```

In tests (where there is no `manager`), use `db.execute(&stmt)` on a
`DatabaseConnection`:

```rust
use sea_orm::ConnectionTrait as _;

db.execute(
    &Query::insert()
        .into_table(Alias::new("my_table"))
        .columns([Alias::new("id"), Alias::new("name")])
        .values_panic([Uuid::now_v7().into(), "example".into()])
        .to_owned(),
)
.await?;
```

> `SchemaManager::exec_stmt` takes ownership (`stmt: impl StatementBuilder`).
> `ConnectionTrait::execute` takes a reference (`stmt: &impl StatementBuilder`).
> Never call `.build(DbBackend::…)` yourself; the framework picks the backend.

### INSERT with ON CONFLICT DO NOTHING (idempotent seeds)

For seed data that must be safe to re-run, add `on_conflict`:

```rust
manager
    .exec_stmt(
        Query::insert()
            .into_table(Alias::new("permissions"))
            .columns([Alias::new("id"), Alias::new("name"), Alias::new("created_at")])
            .values_panic([Uuid::now_v7().into(), "my_perm".into(), now.into()])
            .on_conflict(
                OnConflict::column(Alias::new("name")).do_nothing().to_owned(),
            )
            .to_owned(),
    )
    .await?;
```

For a composite-PK join table use `OnConflict::columns([…])`:

```rust
OnConflict::columns([Alias::new("role_id"), Alias::new("permission_id")])
    .do_nothing()
    .to_owned()
```

### INSERT … SELECT

To populate a table from another (e.g. granting a permission to a role by name without
hardcoding UUIDs):

```rust
// `select_from` returns Result<&mut Self, String>; map the error, then call
// .to_owned() to get an owned InsertStatement for exec_stmt.
let insert = Query::insert()
    .into_table(Alias::new("role_permissions"))
    .columns([Alias::new("role_id"), Alias::new("permission_id")])
    .select_from(
        Query::select()
            .from_as(Alias::new("roles"), Alias::new("r"))
            .from_as(Alias::new("permissions"), Alias::new("p"))
            .column((Alias::new("r"), Alias::new("id")))
            .column((Alias::new("p"), Alias::new("id")))
            .and_where(Expr::col((Alias::new("r"), Alias::new("name"))).eq("owner"))
            .and_where(Expr::col((Alias::new("p"), Alias::new("name"))).eq("my_perm"))
            .to_owned(),
    )
    .map_err(|e| DbErr::Migration(e.to_string()))?
    .on_conflict(
        OnConflict::columns([Alias::new("role_id"), Alias::new("permission_id")])
            .do_nothing()
            .to_owned(),
    )
    .to_owned();

manager.exec_stmt(insert).await?;
```

For repeated grants (e.g. multiple roles or multiple permissions), extract a helper:

```rust
async fn grant_permission(
    manager: &SchemaManager<'_>,
    role_name: &str,
    perm_name: &str,
) -> Result<(), DbErr> {
    let insert = Query::insert()
        .into_table(Alias::new("role_permissions"))
        // … same as above …
        .to_owned();
    manager.exec_stmt(insert).await
}
```

### INSERT … SELECT with literal values and CURRENT_TIMESTAMP

To seed a table from another while injecting literal values (e.g. initializing counters to 0
and timestamps to `CURRENT_TIMESTAMP`):

```rust
let insert = Query::insert()
    .into_table(SettingsVersion::Table)
    .columns([
        SettingsVersion::TenantId,
        SettingsVersion::Version,
        SettingsVersion::GlobalVersion,
        SettingsVersion::RevocationVersion,
        SettingsVersion::UpdatedAt,
    ])
    .select_from(
        Query::select()
            .column(Tenants::Id)
            .expr(Expr::val(0i32))
            .expr(Expr::val(0i32))
            .expr(Expr::val(0i32))
            .expr(Expr::current_timestamp())
            .from(Tenants::Table)
            .to_owned(),
    )
    .map_err(|e| DbErr::Migration(e.to_string()))?
    .to_owned();
manager.exec_stmt(insert).await?;
```

The number of `.columns()` entries must match the number of `SELECT` expressions exactly,
otherwise `select_from` returns an error.

### UPDATE via sea_query

```rust
manager
    .exec_stmt(
        Query::update()
            .table(Alias::new("scheduled_tasks"))
            .value(Alias::new("enabled"), false)
            .and_where(Expr::col(Alias::new("task_type")).eq("legacy_task"))
            .to_owned(),
    )
    .await?;
```

To set a binary (BLOB) value — needed when migrating UUID storage format:

```rust
txn.execute(
    &Query::update()
        .table(Alias::new("permissions"))
        .value(Alias::new("id"), Value::Bytes(Some(bytes)))
        .and_where(Expr::col(Alias::new("name")).eq("my_perm"))
        .to_owned(),
)
.await?;
```

### DELETE via sea_query

```rust
manager
    .exec_stmt(
        Query::delete()
            .from_table(Alias::new("scheduled_tasks"))
            .and_where(Expr::col(Alias::new("task_type")).eq("event_cleanup"))
            .to_owned(),
    )
    .await?;
```

To delete by a subquery (e.g. delete role assignments by permission name):

```rust
manager
    .exec_stmt(
        Query::delete()
            .from_table(Alias::new("role_permissions"))
            .and_where(
                Expr::col(Alias::new("permission_id")).in_subquery(
                    Query::select()
                        .from(Alias::new("permissions"))
                        .column(Alias::new("id"))
                        .and_where(Expr::col(Alias::new("name")).eq("my_perm"))
                        .to_owned(),
                ),
            )
            .to_owned(),
    )
    .await?;
```

### Test-only INSERT with out-of-band values

Tests sometimes need to inject rows that simulate broken or future states (e.g. a TEXT-stored
UUID to test a repair migration, or an unknown enum variant for forward-compatibility tests).
Pass the value directly; sea-query will bind it with the Rust type's `Into<Value>` conversion:

```rust
use sea_orm::ConnectionTrait as _;

// String → Value::String → SQLite stores as TEXT (used to simulate a broken UUID row)
db.execute(
    &Query::insert()
        .into_table(Alias::new("permissions"))
        .columns([Alias::new("id"), Alias::new("name")])
        .values_panic(["018f1234-0000-7000-8000-000000000001".to_owned().into(), "broken".into()])
        .to_owned(),
)
.await?;
```

For an unknown enum string, wrap with `Expr::value(...)`:

```rust
use sea_orm::sea_query::{Expr as SqExpr, Query};

let insert = Query::insert()
    .into_table(scheduled_task::Entity)
    .columns([scheduled_task::Column::Id, scheduled_task::Column::TaskType])
    .values_panic([
        SqExpr::value(Uuid::now_v7()),
        SqExpr::value("future_unknown_task_type"),
    ])
    .to_owned();
db.execute(&insert).await.expect("insert");
```

### Table recreation (SQLite column changes)

SQLite does not support `ALTER TABLE ALTER COLUMN`, and its `ALTER TABLE DROP COLUMN` fails
when the column is referenced by an index, FK constraint, trigger, or view. The standard
workaround is the **table recreation** pattern.

#### When to use table recreation

| Operation | Approach |
| --- | --- |
| Add a new column (no constraints referencing it) | `ALTER TABLE ADD COLUMN` — no recreation needed |
| Drop a column referenced by an index or FK | Table recreation |
| Change a column's type or nullability | Table recreation |
| Add a `GENERATED ALWAYS AS` stored column | Table recreation |
| Add a foreign key to an existing table | Table recreation (SQLite has no `ALTER TABLE ADD CONSTRAINT`) |
| Restructure a table's schema | Table recreation |

For PostgreSQL and MySQL, `ALTER TABLE ADD/DROP/ALTER COLUMN` and `ALTER TABLE ADD CONSTRAINT`
work directly. Migrations that need table recreation on SQLite should branch on the backend
using `helpers::is_sqlite(manager)` and use `ALTER TABLE` on other backends.

**All steps — table creation, data copy, drop, rename, and index recreation — must use
sea_query builders or the shared helpers.** Never use raw SQL strings for these operations.
The only accepted exception is the data copy step when it requires constructs that sea_query
cannot express (e.g., `CASE` expressions or SQLite-specific functions like `strftime`); see
[Data copy strategies](#data-copy-strategies) below.

#### Shared helpers

Reusable helpers live in `crates/shared/db/src/migration/helpers.rs` (imported as
`super::helpers` from migration modules):

| Helper | Purpose |
| --- | --- |
| `set_foreign_keys(manager, enabled)` | Suspend/resume FK enforcement on SQLite (no-op on PostgreSQL/MySQL) |
| `check_crash_recovery(manager, table, temp)` | Detect partial previous runs and return the appropriate recovery state |
| `drop_original(manager, table)` | Drop the original table after data has been copied to the temp table |
| `rename_temp(manager, temp, canonical)` | Rename the temp table to the canonical name |
| `is_sqlite(manager)` | Check whether the current backend is SQLite |

#### Crash recovery (three-state model)

A table recreation can crash at any point. The migration must handle three possible
states on re-entry:

| State | Original table | Temp table | Action |
| --- | --- | --- | --- |
| **A** (normal) | Exists | Does not exist | Full recreation: create → copy → drop → rename |
| **B** (partial) | Exists | Exists | Discard temp (original data intact), restart as State A |
| **C** (rename pending) | Does not exist | Exists | Skip to rename (data already copied) |

`check_crash_recovery()` detects the current state automatically. In State B it drops the
partial temp table and returns `Normal`. In State C it returns `RenameOnly`.

#### Complete pattern with helpers

```rust
use crate::migration::helpers::{self, CrashRecoveryState};

async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    // 1. Suspend FK enforcement (prevents violations during the swap).
    helpers::set_foreign_keys(manager, false).await?;

    // 2. Detect and recover from partial previous runs.
    let state = helpers::check_crash_recovery(
        manager, "my_table", "my_table_new",
    ).await?;

    if state == CrashRecoveryState::Normal {
        // 3. Create the replacement table with the new schema.
        manager.create_table(build_new_schema()).await?;

        // 4. Copy data from old → new.
        copy_data(manager).await?;

        // 5. Drop the original table (indexes are dropped implicitly).
        helpers::drop_original(manager, "my_table").await?;
    }
    // If RenameOnly, the temp table already has the complete dataset.

    // 6. Rename temp → canonical.
    helpers::rename_temp(manager, "my_table_new", "my_table").await?;

    // 7. Recreate indexes (they were dropped with the old table).
    create_indexes(manager).await?;

    // 8. Re-enable FK enforcement.
    helpers::set_foreign_keys(manager, true).await?;
    Ok(())
}
```

#### Data copy strategies

**Always prefer sea_query builders.** The data copy step must use sea_query's
`INSERT...SELECT` builder whenever possible:

```rust
let select = Query::select().columns(DATA_COLS).from(OldTable::Table).to_owned();
let mut insert = Query::insert()
    .into_table(NewTable::Table)
    .columns(DATA_COLS)
    .to_owned();
insert
    .select_from(select)
    .map_err(|e| DbErr::Custom(e.to_string()))?;
manager.execute(insert).await?;
```

**`execute_unprepared` exception** — only when the `INSERT...SELECT` requires constructs
that sea_query's builder cannot express (e.g., `CASE` expressions in the `SELECT` column
list, or SQLite-specific functions like `strftime`). Every such call **must** include an
inline comment naming the specific sea_query limitation:

```rust
// CASE expressions in INSERT...SELECT cannot be expressed with sea_query's
// typed builder. execute_unprepared is the accepted pattern for complex
// data transformations in migrations.
manager
    .get_connection()
    .execute_unprepared(
        "INSERT INTO my_table_new (id, name, category) \
         SELECT id, name, \
           CASE status WHEN 'active' THEN 'enabled' ELSE 'disabled' END \
         FROM my_table",
    )
    .await?;
```

If the copy is a straightforward column mapping (even with column renames), use
sea_query — do not reach for `execute_unprepared` as a convenience shortcut.

#### Backend branching

When a migration uses table recreation on SQLite but `ALTER TABLE` on PostgreSQL/MySQL:

```rust
async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    if helpers::is_sqlite(manager) {
        self.up_sqlite(manager).await
    } else {
        self.up_alter(manager).await
    }
}
```

#### Reference implementations

- `m20260302_000003_host_packages_has_update.rs` — table recreation with
  `GENERATED ALWAYS AS` stored column, both `up` and `down` use the helpers
- `m20260318_000001_cron_to_interval.rs` — table recreation with `CASE`-based
  data transformation, backend branching (SQLite recreation vs. PostgreSQL
  `ALTER TABLE`)

### DROP TABLE in tests

```rust
let drop = Table::drop().table(Alias::new("my_table")).to_owned();
db.execute(&drop).await?;
```

### When `execute_unprepared()` or raw statements are still allowed

Raw SQL is accepted **only** for constructs that have no sea_query equivalent.  Every such call
**must** include an inline comment naming the specific limitation.

| Construct | Reason sea_query cannot express it |
| --- | --- |
| `CREATE TABLE new AS SELECT * FROM old` | SQLite-specific shorthand; no builder equivalent |
| `INSERT INTO … SELECT strftime(…)` | `strftime` is a SQLite-specific function |
| `SELECT typeof(col) FROM …` | `typeof()` is SQLite-specific; use `query_all_raw` / `query_one_raw` |
| `PRAGMA foreign_keys` | SQLite-specific pragma; no sea_query equivalent |
| `SELECT … WHERE col LIKE pattern` in `query_all_raw` | Read-only SQLite-specific pattern matching |
| `CASE WHEN` in `ON CONFLICT DO UPDATE` | SeaORM's `on_conflict` builder limitation |

**Inline comment requirement:**

```rust
// `typeof()` is a SQLite-specific function with no sea_query equivalent;
// query_one_raw with a raw Statement is the approved exception for this pattern.
db.query_one_raw(Statement::from_string(
    DatabaseBackend::Sqlite,
    "SELECT typeof(id) FROM permissions WHERE name = 'x'",
))
.await?;
```

If you find yourself reaching for `execute_unprepared()` for a plain `DELETE`, `INSERT`,
`UPDATE`, or `SELECT` that does not fall into the table above, use a sea_query builder instead.

---

## Down migrations

Always implement `down()`. If a migration is destructive and data cannot be recovered:

```rust
async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
    // Down: no-op — dropped column data cannot be recovered.
    Ok(())
}
```

Document the no-op with a comment explaining why.

---

## Registering a new migration

1. Create the file `crates/shared/db/src/migration/m{date}_{seq}_{desc}.rs`.
2. In `crates/shared/db/src/migration/mod.rs`:
   - Add `mod m{date}_{seq}_{desc};` to the module declarations.
   - Append `Box::new(m{date}_{seq}_{desc}::Migration)` to the **end** of the `migrations()` vec.

```rust
// mod.rs excerpt
mod m20260209_000001_initial;
mod m20260224_000001_mqtt_ha_discovery;
// ... existing entries ...
mod m20260226_000001_add_foo_column;   // ← new

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260209_000001_initial::Migration),
            Box::new(m20260224_000001_mqtt_ha_discovery::Migration),
            // ... existing entries ...
            Box::new(m20260226_000001_add_foo_column::Migration),  // ← appended at end
        ]
    }
}
```

**Never reorder existing entries.** SeaORM applies migrations in order and records which ones
have run; reordering breaks existing databases.

---

## Running migrations in tests

**Never create tables manually in test setup.** Use `run_migrations` on a fresh in-memory
SQLite DB:

```rust
async fn setup_db() -> DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:");
    let db = Database::connect(opt).await.expect("test db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("migrations");
    db
}
```

This guarantees that tests run against the exact same schema as production.
`sea-orm-migration` v2 enables `PRAGMA foreign_keys = ON` after migrating, so tests must
create parent rows rather than disabling FK checks.

### Never use `PRAGMA foreign_keys = OFF` in tests or normal migrations

Disabling FK enforcement to avoid inserting parent rows is **forbidden**.
Create the required parent rows instead:

```rust
// ✗ Wrong — disabling FKs to work around missing parent data
db.execute_unprepared("PRAGMA foreign_keys = OFF").await?;

// ✓ Correct — insert the required parent row first
tenant::ActiveModel {
    id: ActiveValue::Set(Uuid::nil()),
    name: ActiveValue::Set("Test Tenant".to_string()),
    // …
}
.insert(&db)
.await
.expect("insert test tenant");
```

### Repair migrations that touch both sides of a FK relationship

`PRAGMA foreign_keys = OFF` is **silently ignored inside an active transaction** (SQLite
requirement: the pragma can only be changed outside a transaction).  sea-orm-migration v2
wraps every `up()` call in a transaction, so setting the PRAGMA inside `up()` is a no-op
and FK enforcement remains ON.

Instead use a **delete-fix-reinsert** sequence that never has an FK-violating intermediate
state:

1. SELECT the child-side rows that reference the soon-to-be-changed parent key.
2. DELETE those child rows (the old key still exists in the parent, so no FK error).
3. UPDATE the parent row to the new key value.
4. Re-INSERT the child rows with the corrected key (`ON CONFLICT DO NOTHING` for idempotency).

```rust
// 1. Collect which roles reference the TEXT permission_id.
let role_rows = txn
    .query_all(&Query::select()
        .from(Alias::new("role_permissions"))
        .column(Alias::new("role_id"))
        .and_where(Expr::col(Alias::new("permission_id")).eq(text_id.clone()))
        .to_owned())
    .await?;

// 2. Delete child rows — old TEXT key still exists in permissions, no FK error.
txn.execute(&Query::delete()
    .from_table(Alias::new("role_permissions"))
    .and_where(Expr::col(Alias::new("permission_id")).eq(text_id.clone()))
    .to_owned())
    .await?;

// 3. Fix the parent row TEXT id → BLOB id.
txn.execute(&Query::update()
    .table(Alias::new("permissions"))
    .value(Alias::new("id"), blob.clone())
    .and_where(Expr::col(Alias::new("name")).eq(name.as_str()))
    .to_owned())
    .await?;

// 4. Re-insert child rows with the corrected BLOB permission_id.
for role_id_bytes in role_ids {
    txn.execute(&Query::insert()
        .into_table(Alias::new("role_permissions"))
        .columns([Alias::new("role_id"), Alias::new("permission_id")])
        .values_panic([
            Expr::val(Value::Bytes(Some(role_id_bytes))),
            Expr::val(blob.clone()),
        ])
        .on_conflict(
            OnConflict::columns([Alias::new("role_id"), Alias::new("permission_id")])
                .do_nothing()
                .to_owned(),
        )
        .to_owned())
        .await?;
}
```

### Avoid the migration-seeded default tenant for tests with unique-per-tenant constraints

The initial migration creates a **default tenant** and seeds one row per `task_type` in
`scheduled_tasks` for it. Because the table has a `UNIQUE(tenant_id, task_type)` constraint,
inserting a second `auth_cleanup` row for the same tenant will fail.

**Do not fetch the default tenant** in tests that insert their own task rows. Create a fresh
non-default tenant instead:

```rust
async fn seed_tenant(db: &DatabaseConnection) -> tenant::Model {
    let now = OffsetDateTime::now_utc();
    // A fresh non-default tenant has no pre-seeded scheduled_tasks rows.
    tenant::ActiveModel {
        id: ActiveValue::Set(Uuid::now_v7()),
        name: ActiveValue::Set("Test Tenant".to_string()),
        slug: ActiveValue::Set(format!("test-{}", Uuid::now_v7())),
        is_default: ActiveValue::Set(false),
        deactivated_at: ActiveValue::Set(None),
        created_at: ActiveValue::Set(now),
        updated_at: ActiveValue::Set(now),
    }
    .insert(db)
    .await
    .expect("insert test tenant")
}
```

### Testing re-encryption without raw SQL `UPDATE`

The `reencrypt` module tests need to insert rows that simulate legacy plaintext values
(i.e., the `ENC:v1:…` prefix is absent). Do **not** use a raw `UPDATE` to backfill the column.
Instead, use the `testing` feature of `uptrakit-crypto`:

```toml
# crates/my-crate/Cargo.toml
[dev-dependencies]
uptrakit-crypto = { workspace = true, features = ["testing"] }
```

```rust
// Inserts a row using a plaintext (unencrypted) EncryptedString value —
// simulates a legacy row written before encryption was added.
let mut am: ca_certificate::ActiveModel = row.into();
am.key_pem = Set(EncryptedString::plaintext_for_test("raw_key".to_string()));
am.update(&db).await.expect("set plaintext key_pem");
```

`EncryptedString::plaintext_for_test` is gated on `#[cfg(any(test, feature = "testing"))]` in
`uptrakit-crypto/src/lib.rs` and **must never be called in production code**.

### Do not use `#[cfg(test)]` alone to share test helpers across crates

`#[cfg(test)]` is only active when the crate that declares the function is itself under test.
When another crate imports the function, `#[cfg(test)]` is not active for the dependency. Use a
`testing` Cargo feature (as shown above) for any helper that must be visible to dependent
crates' test builds.

See [Testing](testing.md) for the full test DB pattern and FK-helper conventions.

---

## Backend feature flags

The `db-sqlite`, `db-postgres`, and `db-mysql` features of `uptrakit-shared-db` propagate to
`sea-orm-migration` only when the `migration` feature is also enabled (weak dependency). The
controller activates the correct backend via its own feature flags:

| Controller feature | Shared-db features enabled |
| --- | --- |
| `db-sqlite` (default) | `db-sqlite`, `migration` |
| `db-postgres` | `db-postgres`, `migration` |
| `db-mysql` | `db-mysql`, `migration` |

In-memory SQLite tests in web-api use `features = ["migration", "db-sqlite"]` as a
dev-dependency.

---

---

## Data migration

Schema migrations (described above) only manage the **structure** of the
database — tables, indexes, and the small amount of seeded reference data
(tenant, roles, permissions, scheduled tasks). They do not copy **user data**
between database backends.

To move an existing dataset from SQLite to PostgreSQL or MySQL (or between any
two supported backends), use the `db-migrate` subcommand of the controller
binary:

```sh
uptrakit-controller \
  --master-key-file /path/to/master.key \
  db-migrate \
  --source-db sqlite:///path/to/uptrakit.db \
  --target-db postgresql://user:pass@host:5432/uptrakit
```

### Where the data migration code lives

| Path | Purpose |
| --- | --- |
| `crates/core/controller/src/db_migrate/mod.rs` | Top-level `run()` function, orchestration |
| `crates/core/controller/src/db_migrate/error.rs` | `DbMigrateError` and `Result<T>` type alias |
| `crates/core/controller/src/db_migrate/tables.rs` | Generic `migrate_table<E>`, `copy_all`, `clean_all`, `verify_all` |
| `crates/core/controller/src/cli.rs` | `ControllerCommand::DbMigrate`, `DbMigrateArgs` |

### Algorithm

1. Run `run_migrations()` on the target to create the schema and seed rows.
2. Delete all seeded rows from the target in reverse FK order (`clean_all`).
3. Copy each of the 34 application tables from source to target in FK-safe
   order using offset pagination (`copy_all`).
4. Count every table on both sides; fail on the first mismatch (`verify_all`).

Encrypted fields (CA keys, MQTT passwords, OIDC secrets) are copied as opaque
blobs — no decryption or re-encryption occurs during migration. The same master
key used in normal operation must be supplied to `db-migrate`.

See the end-user guide for operator-facing instructions:
[docs/end-user/db-migration.md](../end-user/db-migration.md).

---

## Multi-backend integration testing

All migrations are tested against SQLite, PostgreSQL, and MariaDB via the database integration
tests in `crates/core/integration-tests/tests/database/`. The `migrations.rs` module verifies
that migrations run successfully and all core tables exist on each backend.

```bash
# Run migration tests on all backends (requires Docker)
cargo test -p uptrakit-integration-tests --test database migrations -- --ignored
```

See [Testing — Database Integration Tests](testing.md#database-integration-tests) for the full
guide.

---

## MySQL/MariaDB Compatibility Workarounds

MariaDB/InnoDB has several limitations that require special handling in migrations.
All workarounds are implemented using helpers in `crates/shared/db/src/migration/helpers.rs`.

### TEXT columns in indexes must be VARCHAR(255)

MariaDB/InnoDB limits index key length to 3072 bytes. TEXT columns in indexes
exceed this limit. Use `.string()` (VARCHAR(255)) for any column referenced by
an index:

```rust
// Correct — VARCHAR(255) fits within the 3072-byte index key limit
.col(ColumnDef::new(MyTable::Name).string().not_null())

// Wrong — TEXT in index exceeds the 3072-byte limit on MariaDB
.col(ColumnDef::new(MyTable::Name).text().not_null())
```

### Partial indexes not supported

MariaDB does not support `WHERE` clauses on `CREATE INDEX`. Use a composite
unique index that includes the filtered column:

```rust
let is_mysql = manager.get_database_backend() == sea_orm::DbBackend::MySql;
if is_mysql {
    manager.create_index(
        Index::create()
            .name("uq_items_active_name")
            .table(Items::Table)
            .col(Items::TenantId)
            .col(Items::Name)
            .col(Items::DeactivatedAt)  // include the filter column
            .unique()
            .to_owned(),
    ).await?;
} else {
    manager.create_index(
        Index::create()
            .name("uq_items_active_name")
            .table(Items::Table)
            .col(Items::TenantId)
            .col(Items::Name)
            .unique()
            .and_where(Expr::col(Items::DeactivatedAt).is_null())
            .to_owned(),
    ).await?;
}
```

### Expression/functional indexes not supported

MariaDB does not support `LOWER()` or other expressions in index definitions.
Fall back to plain column indexes — MariaDB collations are case-insensitive
by default (`utf8mb4_general_ci`):

```rust
let sql = if manager.get_database_backend() == sea_orm::DatabaseBackend::MySql {
    "CREATE INDEX idx_items_tenant_name ON items (tenant_id, name)"
} else {
    "CREATE INDEX idx_items_tenant_name ON items (tenant_id, lower(name))"
};
```

### DROP INDEX IF EXISTS panics on MySQL

sea-query panics on `Index::drop().if_exists()` for the MySQL backend. Use the
`helpers::drop_index_if_exists()` helper which handles MySQL error 1091
(index doesn't exist) gracefully:

```rust
// Correct — safe on all backends
helpers::drop_index_if_exists(manager, "idx_my_table_col", "my_table").await?;
```

### FK-backed index drops

MariaDB/InnoDB implicitly uses user-created indexes as backing indexes for FK
constraints. Dropping such an index fails with error 1553. Use the
`drop_mysql_foreign_keys` / `recreate_mysql_foreign_keys` helpers to temporarily
remove all FK constraints before index operations:

```rust
let fks = helpers::drop_mysql_foreign_keys(manager, "my_table").await?;

helpers::drop_index_if_exists(manager, "idx_my_col", "my_table").await?;
manager.create_index(/* new index */).await?;

helpers::recreate_mysql_foreign_keys(manager, "my_table", &fks).await?;
```

These helpers are no-ops on non-MySQL backends.

### INSERT...ON CONFLICT generates invalid MySQL syntax

sea-query's `on_conflict(do_nothing)` generates malformed `INSERT IGNORE` on
MySQL. Use raw SQL with `WHERE NOT EXISTS`:

```rust
let sql = format!(
    "INSERT INTO role_permissions (role_id, permission_id) \
     SELECT r.id, p.id FROM roles r, permissions p \
     WHERE r.name = '{role}' AND p.name = '{perm}' \
     AND NOT EXISTS ( \
       SELECT 1 FROM role_permissions rp \
       WHERE rp.role_id = r.id AND rp.permission_id = p.id \
     )"
);
manager.get_connection().execute_unprepared(&sql).await?;
```

### Reserved word `key` must be quoted

MySQL reserves the word `key`. Use sea_query builders which auto-quote identifiers:

```rust
// Correct — sea_query quotes reserved words automatically
Expr::col(Alias::new("key")).eq(value)
```

### CHECK constraints cannot reference other columns

MariaDB CHECK constraints cannot reference columns other than the one being
checked. Make complex CHECK constraints conditional:

```rust
if manager.get_database_backend() != sea_orm::DbBackend::MySql {
    table.check(
        Expr::col(Sessions::AuthMethod).ne("oidc")
            .or(Expr::col(Sessions::OidcProviderId).is_not_null()),
    );
}
```

### Table recreation on SQLite and MariaDB

When ALTER TABLE operations are complex on MariaDB (FK constraints, column
drops with dependent indexes), use the same table recreation pattern as SQLite:

```rust
let is_postgres = manager.get_database_backend() == sea_orm::DbBackend::Postgres;

if is_postgres {
    // PostgreSQL: ALTER TABLE directly
} else {
    // SQLite / MariaDB: drop + recreate the table
    manager.drop_table(
        Table::drop().table(MyTable::Table).if_exists().to_owned(),
    ).await?;
    manager.create_table(/* new schema */).await?;
}
```

---

## Cross-references

- [Data migration guide](../end-user/db-migration.md) — operator-facing instructions for `db-migrate`
- [Testing](testing.md) — test DB helper pattern, FK conventions
- [Coding Standards](coding-standards.md) — general code quality rules
- [Security: Database](../security/secrets-encryption.md) — at-rest encryption for sensitive
  columns
