# Database Data Migration

The `uptrakit-controller db-migrate` subcommand copies all data from one
database to another. It is the supported path for moving a running installation
from SQLite (the default, developer-friendly backend) to PostgreSQL when you
need high availability or horizontal scalability.

---

## When to Use

Use this command when you want to:

- Move from SQLite to PostgreSQL for a production deployment.
- Clone an existing database for testing or disaster-recovery rehearsal.

> **Important**: This tool copies **data only**. Schema setup (tables, indexes,
> constraints) is handled automatically on the target by running the normal
> migration path before the copy begins.

---

## Prerequisites

Before running the migration:

1. **Stop the controller** — the migration reads from the source while the
   controller is not writing to it. Running both simultaneously is not
   supported and may produce an inconsistent copy.
2. **Back up both databases** — the migration erases all data in the target
   before writing. A backup of the source protects against unexpected failures;
   a backup of the target (if it contains data you want to keep) must be taken
   before running with `--force`.
3. **Note your master key** — the same master encryption key used during normal
   operation must be provided to `db-migrate`. Encrypted fields (CA keys, MQTT
   passwords, OIDC secrets) are copied as-is (the encrypted blobs are
   transferred verbatim), so the target must be started with the same key.

---

## Build Requirements

The `uptrakit-controller` binary must be compiled with support for **both** the
source and target database backends.

| Migration direction | Required `--features` |
| --- | --- |
| SQLite → SQLite | _(default, no extra flag needed)_ |
| SQLite → PostgreSQL | `--features db-postgres` |
| PostgreSQL → SQLite | _(SQLite is always compiled in by default)_ |

Example build for a SQLite → PostgreSQL migration:

```sh
cargo build --release --features db-postgres
```

---

## Step-by-Step: SQLite → PostgreSQL

```sh
# 1. Stop the controller.
systemctl stop uptrakit-controller

# 2. Create the empty target database in PostgreSQL.
createdb uptrakit

# 3. Run the migration.
uptrakit-controller \
  --master-key-file /etc/uptrakit/master.key \
  db-migrate \
  --source-db "sqlite:///var/lib/uptrakit/uptrakit.db" \
  --target-db "postgresql://uptrakit:password@localhost:5432/uptrakit"

# 4. Update your controller configuration to point at the new database.
# Set UPTRAKIT_DB_URL or --db-url to the PostgreSQL URL.

# 5. Restart the controller.
systemctl start uptrakit-controller
```

The command prints a confirmation prompt before erasing the target and copying
data. Type `yes` to proceed.

---

## Master Key Handling

Encrypted fields are stored as opaque blobs (format: `ENC:v3:<key_id>:<hex>`).
The migration copies these blobs **without decryption or re-encryption**,
including the `data_encryption_keys` table (wrapped DEKs). The target database
must therefore be used with the same master key as the source.

Provide the master key the same way you do for normal server operation:

```sh
# Via file
uptrakit-controller --master-key-file /path/to/master.key db-migrate ...

# Via environment variable
UPTRAKIT_MASTER_KEY=<64-char-hex> uptrakit-controller db-migrate ...
```

If you want to rotate the master key at the same time as migrating backends,
do so **after** the migration is complete and the controller is running on the
new database.

---

## Options Reference

```text
uptrakit-controller [GLOBAL OPTIONS] db-migrate [OPTIONS]
```

### Global options (shared with `serve`)

| Flag | Description |
| --- | --- |
| `--master-key-file <PATH>` | Path to a file containing the 64-char hex master key. |
| `--allow-plaintext-secrets` | Allow the controller to run without a master key (dev only). |

### Subcommand options

| Flag | Default | Description |
| --- | --- | --- |
| `--source-db <URL>` | _(required)_ | Source database URL to read from. |
| `--target-db <URL>` | _(required)_ | Target database URL to write to. |
| `--batch-size <N>` | `500` | Number of rows per read/write batch. |
| `--force` | `false` | Skip the non-empty target safety check. |
| `--yes` | `false` | Skip the interactive confirmation prompt (for scripted/CI use). |

---

## Non-Interactive Use

For scripted deployments or CI pipelines, pass `--yes` to skip the prompt:

```sh
uptrakit-controller \
  --master-key-file /etc/uptrakit/master.key \
  db-migrate \
  --source-db "$SOURCE_URL" \
  --target-db "$TARGET_URL" \
  --yes
```

Use `--force` if the target database already contains rows in the `users` table
and you want to overwrite them (e.g. retrying a failed migration).

---

## Verification

At the end of the migration, the tool prints a summary:

```text
Migration complete.
  Rows migrated : 142
  Elapsed       : 1.3s
```

Before printing the summary, the tool counts every application table on both
the source and target and reports an error if any count does not match. The
check covers all 34 application tables.

---

## Rollback

To roll back:

1. Stop the controller.
2. Restore the original `--db-url` configuration pointing at the source.
3. Restart the controller.

The source database is never modified by `db-migrate`. If the migration failed
or produced unexpected results, simply start the controller on the original
source again.

---

## Limitations

- **No concurrent writes** — the controller must be stopped during the
  migration. Running both simultaneously is unsupported.
- **Ephemeral state is included** — sessions, pending OAuth flows, pending
  device-auth flows, and rate-limit counters are copied. Most of these expire
  shortly after the controller restarts on the new database, which is the
  expected behaviour.
- **Offset pagination** — batching uses `OFFSET + LIMIT` pagination. For very
  large tables (> 1 M rows), this can be slow on some backends because the
  database must scan all preceding rows to compute the offset. Increase
  `--batch-size` to reduce the number of round-trips.
- **Feature flag required** — the binary must be compiled with support for the
  target backend. A mismatch between the URL scheme and the compiled features
  produces a clear error at startup.

---

## Related Documentation

- [Development: Database Migrations](../development/database-migrations.md) —
  schema migration internals and how to add new migrations.
- [Security: Secrets Encryption](../security/secrets-encryption.md) — master
  key requirements and encrypted field behaviour.
- [Deployment Map](deployment-map.md) — where db-migration fits in the wider
  deployment story.
