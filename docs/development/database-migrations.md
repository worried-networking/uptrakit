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
**partial (filtered) unique index**, which requires raw SQL via `execute_unprepared()`:

```sql
CREATE UNIQUE INDEX uq_plugin_configs_active_name
    ON plugin_configs (name) WHERE deactivated_at IS NULL;
```

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
| `CREATE UNIQUE INDEX … WHERE col IS NULL` | Partial/filtered index; builder has no `WHERE` |
| `INSERT INTO … SELECT strftime(…)` | `strftime` is a SQLite-specific function |
| `PRAGMA foreign_keys = OFF / ON` | PRAGMA has no sea_query equivalent |
| `SELECT typeof(col) FROM …` | `typeof()` is SQLite-specific; use `query_all_raw` / `query_one_raw` |

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

**Approved exception — repair migrations that atomically fix FK-violating data**

A repair migration may temporarily disable FK enforcement when it must update both sides of a
FK relationship as an atomic unit and the intermediate state is necessarily inconsistent.
All three conditions must hold:

1. The migration fixes existing broken data (not a normal schema or seed change).
2. The PRAGMA is set immediately before `begin()`, re-enabled immediately after `commit()`.
3. An inline comment explains why the intermediate state violates FK constraints.

```rust
// Both `role_permissions.permission_id → permissions.id` and the reverse
// parent-key-update check fire during the TEXT→BLOB conversion.  The
// intermediate state is FK-inconsistent; turning enforcement off for the
// duration of the transaction is the only safe option.
//
// PRAGMA foreign_keys has no sea_query equivalent; execute_unprepared is
// the approved exception for PRAGMA statements.
db.execute_unprepared("PRAGMA foreign_keys = OFF").await?;
let txn = db.begin().await?;
// … updates …
txn.commit().await?;
db.execute_unprepared("PRAGMA foreign_keys = ON").await?;
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

## Cross-references

- [Data migration guide](../end-user/db-migration.md) — operator-facing instructions for `db-migrate`
- [Testing](testing.md) — test DB helper pattern, FK conventions
- [Coding Standards](coding-standards.md) — general code quality rules
- [Security: Database](../security/secrets-encryption.md) — at-rest encryption for sensitive
  columns
