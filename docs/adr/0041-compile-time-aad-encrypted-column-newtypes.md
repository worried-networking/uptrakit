# 0041 — Compile-time AAD encrypted column newtypes

Date: 2026-08-12

## Status

Accepted

## Context

Three plugin-config stores held sensitive credentials in plaintext at rest: `plugin_configs.config`,
`plugin_type_settings.config`, and `instance_plugin_setting.config` (layer 1 and layer 2 of the
three-layer effective-config merge — see [plugin-system.md](../development/plugin-system.md)). Bringing
them under encryption with the pre-existing mechanism was not viable.

The existing encrypted-column mechanism (`EncryptedString`, `crates/shared/crypto/src/lib.rs`) resolves
the AAD for `ENC:v2:`/`ENC:v3:` decryption through a single **runtime, global, bare-column-name
registry**: `register_column_aad()` takes `ColumnAadEntry { table, column, aad }` triples but keys the
lookup map on `column` alone, because `TryGetable::try_get_by` receives only a column/alias name from
SeaORM, never table context. Two properties of that design collide directly with these three tables:

- **Global column-name uniqueness.** `register_column_aad()` fails registration outright
  (`CryptoError::DuplicateColumnAad`) the moment two entries share a bare column name. All three tables
  name their sensitive column `config` — the same bare name already used by
  `notification_channels.config` — so a fourth `ColumnAadEntry { column: "config", .. }` per table could
  never coexist in the registry as originally designed.
- **Alias blindness.** Even column-name uniqueness would not save every read path: joined,
  `select_only().column_as()`-projected queries rename the result column before `TryGetable` ever
  sees it. `crates/core/scheduler-runtime/src/executors/queries.rs:71` renames
  `plugin_config::Column::Config` to `"profile_config"` in exactly such a joined row struct — a bare-name
  registry lookup for `"profile_config"` finds nothing, because the entry was registered under `"config"`.
  This is not a hypothetical; it is a live query on the batch-assignment read path that the old mechanism
  could not have served correctly.

A second, independent problem: `register_column_aad_mappings()` (or its equivalent) is called
separately by each of three binaries — `uptrakit-controller-runtime`, `uptrakit-agent-ssh-runtime`, and
`uptrakit-scheduler-runtime` (`standalone.rs`) — each maintaining its own `Vec<ColumnAadEntry>` literal.
Adding a table's entry to one binary's list and forgetting another is a silent, per-binary drift: the
binary missing the entry can insert/write the column fine but fails to decrypt it back, with no
compile-time signal that the lists have diverged.

## Decision

Encrypt the three columns using a **compile-time AAD encrypted-column newtype**, generated per column by
the `encrypted_column!` macro (`crates/shared/crypto/src/encrypted_column.rs`) and instantiated once per
column in `crates/shared/db/src/encrypted_columns.rs`:

```rust
encrypted_column!(EncryptedPluginConfig, "uptrakit:plugin_configs:config");
encrypted_column!(EncryptedPluginTypeConfig, "uptrakit:plugin_type_settings:config");
encrypted_column!(EncryptedInstancePluginConfig, "uptrakit:instance_plugin_setting:config");
```

Each generated type carries its own `AAD: &'static str` as an associated constant baked in at the
`encrypted_column!` call site, not looked up at runtime by column name. `ValueType`/`TryGetable` decode
that fixed AAD directly, so the type itself — not a name string flowing through a shared table — is the
unit of AAD resolution. A joined query that aliases the column to `"profile_config"` still decodes
correctly, because decoding never consults the alias: it consults the Rust type of the field the value is
read into. This closes both failure modes above by construction — no bare-name collision is possible
(three distinct Rust types, one registry entry each, no shared namespace) and no alias can desynchronize
decode from the correct AAD (the type, not the projected name, carries it). It also closes the
three-binary drift: the AAD is a `pub const` compiled once into `uptrakit-shared-db`, linked identically
into all three binaries — there is no per-binary list to keep in sync.

### Eager parse and infallible accessors

The generated type parses its plaintext as JSON at both construction (`new`/`from_json`) and decode, so
`as_json()`/`expose_secret()` are infallible. `PartialEq` compares the parsed JSON structurally (ciphertext
nonces differ per encryption, and Postgres `jsonb`→`text` normalization must not create phantom diffs), and
a `PartialEq<serde_json::Value>` impl lets call sites compare directly against a literal
`serde_json::Value` (used by `default_configs.rs:52` and equivalent fixture-comparison sites) without an
intermediate `EncryptedPluginConfig` wrapper.

### Feature-gating the whole macro module on `sea-orm`

`encrypted_column.rs` is gated `#[cfg(feature = "sea-orm")]` at the **module** level, not per trait impl
inside the macro expansion. A `#[cfg(feature = "sea-orm")]` written inside a `macro_rules!` body would
evaluate against the _consuming_ crate's feature flags at expansion time, not `uptrakit-crypto`'s own —
a cross-crate cfg leak that would silently omit the SeaORM trait impls (`ValueType`, `TryGetable`,
`Nullable`, `From<T> for sea_orm::Value`) in any consumer that didn't happen to also enable
`uptrakit-crypto/sea-orm` under the exact same flag name. Every real consumer of `encrypted_column!`
already enables `uptrakit-crypto/sea-orm` unconditionally, so gating the entire module here — rather than
per-impl inside the macro — costs nothing and removes the leak entirely.

### `down()` refuses when ciphertext is present

Each of the three migrations (`m20260812_000001`–`m20260812_000003`) defines `down()` to run a raw
`COUNT(*) WHERE config LIKE 'ENC:%'` before reverting the column type, and to fail with a named error
(`"cannot revert: encrypted rows present … decrypt is impossible in a migration (no DEK ring)"`) when that
count is non-zero. A `sea_orm_migration::MigrationTrait` implementation has no access to the running
process's master key or DEK ring — it cannot decrypt a row even in principle — so an unconditional `down()`
would either fail the underlying column-type cast or, on backends more permissive about the cast, silently
truncate ciphertext into a value no longer valid as JSON. Refusing outright is the only safe behavior; an
operator who genuinely needs to revert must first drain ciphertext from the column through the running
application (which does hold the keys), not through the migration.

### SQLite: declared no-op, not table recreation

On SQLite, `up()` is a deliberate no-op for the column-type change: SQLite's type-affinity model already
stores `json`-declared columns as `TEXT` on disk, so there is nothing to migrate at the storage level — the
declared column type stays `'json'` while the entity's `ValueType::column_type()` reports `Text`, and the
two coexist harmlessly under affinity rules. This is an explicit, commented exception to the project's
default column-type-change response (drop-and-recreate the table; see
[database-migrations.md](../development/database-migrations.md)) for the one case where the type change is
storage-invisible. A future schema-comparison or drift-detection gate must not treat the mismatched
declared-vs-entity type as an error on SQLite; the comment in each migration exists to prevent that
surprise from being rediscovered as a bug.

On PostgreSQL, where `json`→`text` genuinely changes on-disk representation, `up()` uses the pre-approved
raw-SQL exception (`ALTER TABLE … ALTER COLUMN config TYPE text USING config::text`). The `USING` clause
itself is builder-expressible (`ColumnDef::using()`); the exception rests on the frozen-merged-migration
rationale instead — the statement ships inside an already-merged `up()` body, and rewriting a shipped
migration body risks live-vs-fresh-install divergence. The cast is not optional either way: Postgres has no
implicit assignment cast from `json` to `text`. The symmetric `down()` cast (`… TYPE json USING
config::json`, likewise with no implicit `text`→`json` assignment cast) is a bare cast with no
`DROP DEFAULT`/`SET DEFAULT` dance — that was tested directly against real Postgres. For
`plugin_type_settings.config` and `instance_plugin_setting.config`, both created with `.json().default("{}")`,
the `DEFAULT '{}'` survives the round trip unmodified; no extra step is required or present in the migration.
`plugin_configs.config` was created with the bare `json()` schema helper and carries no column default at
all, so the same bare-cast `down()` applies there with no default to preserve in the first place.

### Layer 3 (per-assignment override) stays plaintext

The plugin-config effective-config merge has three layers: tenant-level type settings
(`plugin_type_settings.config`, now encrypted), named profile configs (`plugin_configs.config`, now
encrypted), and per-`host_software_item_plugin` overrides (`host_software_item_plugin.config` /
`assignment_config` in joined row structs). Layer 3 is **not** brought under `encrypted_column!` here: the
project's owner-level decision is that layer 3 must carry no secrets at all — narrower overrides exist for
non-sensitive per-host tuning, not credentials — and enforcing that rule (schema-level validation rejecting
secret-typed fields in a layer-3 payload) is scoped to Plan 3, not this one. Every touched read/write
site in this change discriminates the two: `plugin_config.config` converts to the new encrypted newtype,
`host_software_item_plugin.config`/`assignment_config` stays `Option<serde_json::Value>` unchanged.

### `migrate_core_tables.rs` copy path: passthrough, not decrypt-and-reencrypt

`crates/shared/db/src/migrate_core_tables.rs`'s `copy_one::<E>()` (used for the `db-migrate` subcommand's
cross-backend data copy, including `PluginConfig`) loads a page of `E::Model` from the source connection
and re-inserts the same `ActiveModel` values into the destination. Because the generated type's
`From<EncryptedPluginConfig> for sea_orm::Value` conversion returns the wrapped `EncryptedString`'s
existing database representation as-is (it does not re-encrypt), the copy is a byte-for-byte passthrough of
whatever is currently stored — plaintext copies as plaintext, `ENC:v3:` ciphertext copies as the same
ciphertext (readable at the destination only if the destination has the same DEK, which `db-migrate` does
not guarantee). Rows that are still plaintext at copy time are the common case (fresh copy ahead of first
boot) and convert to ciphertext automatically the first time the target controller boots and its lazy
`reencrypt_to_v3` pass runs — no special-casing of encrypted columns was added to the copy path itself.

`copy_one`'s pagination uses plain `.offset(offset).limit(batch_size)` (`OFFSET`-based, not the keyset
pagination this change applies to the `reencrypt_to_v3` upgrade passes). That is a known, accepted
inefficiency/correctness-under-concurrent-write gap in the pre-existing `db-migrate` copy path, explicitly
out of scope for this change — `db-migrate` runs offline against a quiescent source, where `OFFSET`'s
usual concurrent-mutation hazard does not apply, so it was left as-is rather than folded into this ADR's
scope.

### External-scheduler DEK-ring read precondition

The external scheduler (`uptrakit-scheduler`, `crates/core/scheduler-runtime/src/standalone.rs`) reads
`plugin_configs`, `plugin_type_settings`, and `instance_plugin_setting` directly for its assignment
queries, so it now carries a hard read dependency on a live master key and initialized DEK ring for those
three tables — without them, `EncryptedPluginConfig` decode fails on any row already converted to
ciphertext. The scheduler's master key initialization is conditional
(`creds.master_key_hex.is_some()`) and its DEK-ring initialization is best-effort (logs and continues
on failure), so a scheduler instance can start successfully right after deployment with a
missing/failed ring — because the three tables are still plaintext at that moment — and then fail only
later, at the first controller restart after the lazy `reencrypt_to_v3` pass has converted those rows.
This is a deploy-decoupled, delayed failure mode: the visible break happens on an unrelated controller
event, not on the scheduler's own (mis)configuration. `standalone.rs` now surfaces the precondition
explicitly: once ring initialization has been attempted, if `master_key_available()` is false or
`data_key_ring_available()` is false, it emits a `tracing::error!` naming all three tables as unreadable
until valid credentials arrive. This is visibility only — no fail-fast and no registry change — the
scheduler continues serving crypto-independent work.

### Existing encrypted columns outside this ADR use two other mechanisms

Encryption-at-rest for sensitive columns outside the three tables covered here is split across two other,
distinct mechanisms — this ADR's compile-time newtype is a third:

- **Runtime `ColumnAadEntry` registry.** `notification_channels.config`, `ca_certificates.key_pem`,
  `oidc_providers.client_secret`, `pending_oidc_flows.pkce_verifier`, and `user_totp.secret` are
  `EncryptedString` columns whose AAD is resolved at decode time through the bare-column-name registry
  described above (`register_column_aad_mappings()` in `crates/core/controller-runtime/src/reencrypt.rs`
  registers exactly these five entries unconditionally, plus a sixth, `ssh_hosts.private_key`
  (`crates/core/agent-ssh-runtime/src/handler.rs:57-63`), appended only when the `embedded-ssh-agent`
  feature is enabled — no others). These are the columns subject to the two failure modes (global name
  collision, alias blindness) and the three-binary drift risk this ADR's mechanism avoids.
- **Direct `encrypt_str`/`decrypt_str` with a call-site AAD constant.** Settings values
  (`auth.jwt_signing_key`, `nats.url`, `smtp.password`, the GitHub provider auth token, the system-services
  enrollment token, …) live in `settings.value` / `global_settings.value`, both typed plain
  `serde_json::Value` (`crates/shared/db/src/entity/setting.rs:12`,
  `crates/shared/db/src/entity/global_setting.rs:10`) — never `EncryptedString`. They never reach
  `TryGetable`, so they are not in the `ColumnAadEntry` registry at all. Instead, each call site encrypts
  or decrypts the JSON-embedded string value directly against a hardcoded AAD constant (for example
  `JWT_KEY_AAD` in `crates/ui/web-api-auth/src/settings_store.rs:24`, or `AAD_SETTINGS_NATS_URL` /
  `AAD_SETTINGS_SMTP_PASSWORD` in `crates/core/controller-runtime/src/reencrypt.rs`). This mechanism
  already ties AAD to a specific call site rather than a bare column name, so it does not suffer the
  registry's alias-blindness or collision failure modes — its drift risk is different in kind (a call site
  that forgets to encrypt/decrypt at all, not a registry collision).

Migrating the `ColumnAadEntry`-registry columns to `encrypted_column!` newtypes is judged worthwhile — the
same alias-blindness and cross-binary-drift risks that motivated this ADR apply to them in principle — and
is intended as incremental future work, out of scope for this change. The settings-value mechanism is a
separate case: `settings`/`global_settings` store heterogeneous JSON blobs under a single generic `value`
column shared by secret and non-secret keys alike, so it is not a like-for-like fit for a per-column
newtype without first splitting secret and non-secret settings apart. Whether and how to bring it under a
compile-time mechanism is a separate, not-yet-scoped question — it is not folded into the "future work" of
migrating the registry columns above.

## Consequences

**Whole-load blast radius is real and accepted.** The eager-parse design means one row whose `config`
value fails to decrypt or fails to parse as JSON fails the entire `Vec<Model>` load it is part of — there
is no per-row partial success at the `SeaORM` `find().all()` layer, and by design there is no
plaintext-fallback-on-decode-error escape hatch (a hard project-wide rule; see
[Secrets Handling](../security/secrets-and-encryption.md)). This is accepted as the cost of never silently
returning unauthenticated or wrong-AAD plaintext. It is mitigated in two ways: `TryGetable::try_get_by`'s
decode error embeds a greppable handle — the first 40 characters of the ciphertext (`ENC:v3:<key_id>:`
plus enough of the nonce to identify the row, but never plaintext) or `<plaintext, N bytes>` for
undecrypted rows — so an operator can locate the offending row directly
(`WHERE config LIKE 'ENC:v3:<key_id>:<nonce-prefix>%'`) without decrypting anything. And the lazy
`reencrypt_to_v3` upgrade passes (`crates/core/controller-runtime/src/reencrypt.rs`) recover from a
poisoned page during the upgrade sweep itself: on a page-load failure they fall back to a pk-only select
(primary keys are never encrypted) to advance the keyset cursor past the bad page rather than stalling the
whole table's pass. The same whole-load hazard is exactly why the plaintext-residue observability counters
(`count_plaintext_residue`) are typed raw `SELECT COUNT(*) … NOT LIKE 'ENC:%'` scalar queries rather than
entity loads — the diagnostic that tells an operator rows are stuck in plaintext must itself be immune to
the failure it is diagnosing.

**Migrations gained a documented raw-SQL exception and a documented SQLite affinity exception.** Both are
narrowly scoped (one `ALTER COLUMN … USING` cast per direction on Postgres; one no-op with an inline
rationale comment on SQLite) and follow the project's existing raw-SQL-exception comment protocol, so they
do not weaken the no-raw-SQL rule generally.

**`db-migrate` needs no encryption-aware logic.** Because the newtype's `sea_orm::Value` conversion is a
byte-for-byte passthrough of the stored representation, the cross-backend copy path required zero changes
to handle these three columns — it already treated them as opaque strings before this change and continues
to after.

**The scheduler's operational runbook gained one more startup-health signal to watch.** Operators running
an external scheduler must now treat a startup-time `data_key_ring_available() == false` warning as
actionable ahead of the next controller restart, not just informational.

**Drift between binaries on AAD assignment is now a compile error, not a runtime silent divergence**, for
these three columns specifically — extending the same protection to the columns still on the runtime
registry is the natural next increment, tracked as future work rather than committed to here.
