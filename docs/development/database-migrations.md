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
SQLite's `strftime` function via `execute_unprepared()` (the typed API cannot express this):

```rust
// Recreate the table with TEXT timestamps; convert via strftime
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

### Checklist when adding a new table

For each new table, verify:

- Primary key column — always indexed (implicit for `PRIMARY KEY`)
- `tenant_id` column — included in composite PK or has a standalone index
- Every FK column referenced in `SELECT … WHERE fk = ?` or `JOIN … ON a.fk = b.id`
- Any column used in `ORDER BY` in API list endpoints (e.g. `created_at`)

---

## When to use `execute_unprepared()`

Prefer the typed SeaORM `SchemaManager` API. Raw SQL via `execute_unprepared()` is accepted
only when the typed API cannot express the operation:

| Acceptable | Example |
| --- | --- |
| Multi-table conditional DML | `UPDATE a SET x = (SELECT y FROM b WHERE ...)` |
| JSON field updates across rows | `UPDATE t SET config = json_patch(config, '...')` |
| Complex `ALTER TABLE` patterns | Renaming a column on SQLite (requires recreate-table) |
| Partial / expression indices | `CREATE INDEX ... WHERE deactivated_at IS NULL` |

Always add a comment that justifies the raw SQL:

```rust
// SQLite does not support ALTER TABLE ... RENAME COLUMN, so we
// recreate the table. SeaORM's SchemaManager cannot express this.
manager
    .get_connection()
    .execute_unprepared("...")
    .await?;
```

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
`sea-orm-migration` v2 enables `PRAGMA foreign_keys = ON` after migrating, so tests that
insert FK-dependent rows must create the parent rows first.

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
