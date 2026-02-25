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

```
m{YYYYMMDD}_{6-digit-sequence}_{short_description}.rs
```

Examples:

```
m20260209_000001_initial.rs
m20260224_000001_mqtt_ha_discovery.rs
m20260225_000001_rename_docker_provider.rs
m20260225_000002_phs_discovery_only.rs
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

## Cross-references

- [Testing](testing.md) — test DB helper pattern, FK conventions
- [Coding Standards](coding-standards.md) — general code quality rules
- [Security: Database](../security/secrets-encryption.md) — at-rest encryption for sensitive
  columns
