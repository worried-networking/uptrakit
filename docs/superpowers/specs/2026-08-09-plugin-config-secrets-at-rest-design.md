# Plugin Configuration Secrets at Rest — Design

**Date:** 2026-08-09
**Status:** Approved for planning
**Owner round:** layer-3 treatment = reject sensitive keys (not encrypt); `instance_plugin_setting` included; `credential_updated_at` added to scope mid-grilling.

## 1. Problem

Provider credentials (GitHub/GitLab/Forgejo release tokens, Proxmox API tokens, Docker registry
credentials) are stored as plaintext JSON in the database, and two of the three config layers also
leak them unmasked over the REST API:

| Store                                                     | Column                            | At rest   | Read path over API                                                                                                             |
| --------------------------------------------------------- | --------------------------------- | --------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `plugin_configs` (profile layer)                          | `config` (`Json`)                 | plaintext | masked (`***`) via per-plugin fn-pointers                                                                                      |
| `plugin_type_settings` (tenant layer)                     | `config` (`Json`)                 | plaintext | **unmasked** — no `mask_config_secrets` call anywhere in `queries/plugin_type_settings.rs` or `routes/plugin_type_settings.rs` |
| `instance_plugin_setting` (instance layer)                | `config` (`Json`)                 | plaintext | **unmasked** — `routes/instance_plugins.rs` returns `current_config` raw; no mask/restore machinery exists for this store      |
| `host_software_item_plugins` (per-host override, layer 3) | `config` (`JsonBinary`, nullable) | plaintext | **unmasked** — `map_response_config_override` returns stored JSON verbatim to any `software:read` caller                       |

Additional verified facts:

- `SecretString` (`crates/shared/types/src/secret_string.rs`) is serde-transparent hygiene, not
  crypto: it serializes plaintext. Masking is presentation-only and per-plugin hand-written
  (`with_secrets_masked` / `restore_secrets_from` wired through `PluginDescriptor.config`
  fn-pointers).
- None of these columns appear in the master-key re-encryption pass
  (`crates/core/controller-runtime/src/reencrypt.rs`), so their contents are invisible to key
  rotation.
- Layer 3 is validated against the **full** plugin config schema (no `deny_unknown_fields`
  anywhere in the repo), the per-host editor (`EditHostAssignmentModal.svelte`) renders password
  fields, and the autodiscovery write path
  (`queries/autodiscovery/discovery_items.rs`, `config: Set(target.config_override.clone())`)
  applies **zero** validation. Per-host credentials are therefore fully possible today.
- Contrast: `notification_channels.config` already uses `EncryptedString` — but see §7
  (its restore-on-write is missing, a defect this design fixes rather than copies).

## 2. Decision summary

1. **Encrypt three stores at rest** — `plugin_configs.config`, `plugin_type_settings.config`,
   `instance_plugin_setting.config` — via per-column newtypes over
   `uptrakit_crypto::EncryptedString` that carry a **compile-time AAD** (§3). Distinct AADs:
   - `uptrakit:plugin_configs:config`
   - `uptrakit:plugin_type_settings:config`
   - `uptrakit:instance_plugin_setting:config`
2. **Do not encrypt layer 3.** Instead make it structurally secret-free: reject sensitive keys at
   both write paths (§6). Rationale recorded in §6.
3. **Schema-only migration + lazy startup encryption** (§4). No cryptography in migrations —
   matches every prior at-rest conversion in this codebase; the DEK ring initializes after
   migrations run, so migration-time encryption would emit wrong-format values.
4. **Schema-driven key-set masking** replaces the per-plugin mask/restore fn-pointers (§5).
   Sensitive fields are identified by `FormFieldDescriptor.sensitive` dotted paths (e.g.
   `auth_token`, `auth.password`), masking touches only keys present in the object.
5. **`credential_updated_at`** column on `plugin_configs`, stamped when a sensitive value
   changes, exposed as a non-skipped field on `PluginConfigView` (§8).
6. **Prerequisite defect fixes** shipped in the same series: `reencrypt_to_v3` OFFSET pagination,
   `notification_channels` missing restore-on-write, post-restore `"***"` sentinel assertion (§7).

REST contract shape is unchanged everywhere: `config` stays JSON in requests and responses;
encryption is storage-layer only.

## 3. Encrypted-column newtypes (compile-time AAD)

### Why not the existing bare-name AAD registry

The `EncryptedString` SeaORM integration looks up AAD from a process-global registry keyed by the
**result-set column name** at `TryGetable` time (SeaORM provides no table context). Three facts
make that registry unusable here:

1. `notification_channels.config` already owns the key `"config"`; registering the same key with a
   different AAD is a hard `DuplicateColumnAad` error. A column rename would be required — and
   `host_software_item_plugins.config` was already renamed once (`config_override` → `config` in
   `m20260312_000003`, a ~200-line SQLite table recreation that silently dropped three FKs).
2. The lookup key is the SELECT **alias**, not the physical name. The scheduler already reads
   `column_as(plugin_config::Column::Config, "profile_config")`
   (`crates/core/scheduler-runtime/src/executors/queries.rs:71`), and joined reads get sea-orm's
   `B_`-prefixed keys. Renamed columns still decrypt with the wrong (empty) AAD on any aliased or
   prefixed read — a runtime failure the compiler cannot see.
3. The registry is hand-duplicated in three binaries (controller `reencrypt.rs`, scheduler
   `standalone.rs`, `db_migrate`) and has already drifted (`user_totp.secret` missing from the
   scheduler copy).

### Chosen mechanism

Add a small additive API to `uptrakit-crypto` and a macro (location: `uptrakit-crypto` behind the
existing `sea-orm` feature):

- Extract the prefix-dispatch-and-decrypt logic that is currently **duplicated inline** in
  `ValueType::try_from` and `TryGetable::try_get_by` (`encrypted_string.rs` — `ENC:v1/v2/v3`
  prefix handling, plaintext-mode fallback, `decrypt_str` call) into one shared decode function
  taking an explicit AAD, e.g. `EncryptedString::decode_db_value(raw: String, aad: &str) ->
Result<Self>`. Note: the existing `from_db(plaintext, db_repr)` takes an _already-decrypted_
  plaintext and does no decryption — making it public is **not** the fix; the refactor is the
  extraction. Existing trait impls are reimplemented on the shared function (behavior unchanged).
  Behavior detail the extraction must preserve: the legacy `ENC:v1:` branch always decrypts with
  the **empty** AAD regardless of the caller-supplied one — the passed-in AAD applies to v2/v3
  only (irrelevant to the three new columns, which never held v1 data, but load-bearing for
  `EncryptedString`'s own legacy read path).
- `encrypted_column!(TypeName, "uptrakit:<table>:<column>");` generates a newtype wrapping
  `EncryptedString` plus an **eagerly parsed** `serde_json::Value` with:
  - `TryGetable` / `ValueType` / `Nullable` / `From<TypeName> for sea_orm::Value` that **hardcode
    the AAD** — no name lookup, immune to aliases and join prefixes, works identically in every
    binary with no per-binary registry entries;
  - constructor `new(plaintext_json: String) -> uptrakit_crypto::Result<Self>` (and a
    `from_json(serde_json::Value)` convenience) that **validates/parses the JSON at
    construction** — mirroring the crate's existing encrypt/decrypt-eagerly idiom. Parse failure
    is a construction/decode error (a new `CryptoError::InvalidJson` variant; at `TryGetable`
    decode time it surfaces as `TryGetError::DbErr`, same as decrypt failure). Consequently
    `as_json() -> &serde_json::Value` and `expose_secret()` are **infallible** accessors — no new
    fallible path at the ~25 call sites;
  - `PartialEq` comparing the **pre-parsed JSON** (structural, infallible) — preserves the
    equality semantics of the current `serde_json::Value` columns (key order, whitespace, and the
    Postgres `jsonb → text` normalization must not create phantom diffs), plus a
    `PartialEq<serde_json::Value>` cross-impl because
    `queries/autodiscovery/default_configs.rs` compares the entity field directly against a
    `&serde_json::Value` (`&cfg.config == config_json`) — comparison against the parsed value,
    no encryption per comparison;
  - redacting `Debug`, same legacy-plaintext tolerance and `needs_v3_upgrade()` passthrough as
    `EncryptedString`.

Feature gating, stated explicitly: the shared decode function, constructors, and accessors are
plain `uptrakit-crypto` API (ungated); only the generated SeaORM trait impls sit behind the
existing additive `sea-orm` feature, exactly as `EncryptedString`'s trait impls do today.

Three instantiations, defined next to their entities in `uptrakit-shared-db`:
`EncryptedPluginConfig`, `EncryptedPluginTypeConfig`, `EncryptedInstancePluginConfig`.

Entity field changes (compiler then forces every read/write site to convert):

- `plugin_config::Model.config: EncryptedPluginConfig` (drop `column_type = "Json"`; becomes Text)
- `plugin_type_setting::Model.config: EncryptedPluginTypeConfig`
- `instance_plugin_setting::Model.config: EncryptedInstancePluginConfig`

Decrypt failure is a `DbErr` surfaced through each query module's existing typed error — there is
**no plaintext fallback on error** (the `service_config` warn-and-fall-back pattern is explicitly
not copied).

### Read/write site inventory (compile-fix checklist)

The plan must enumerate these, not discover them by compiler error alone. Typed-model sites (the
compiler catches these):

- `queries/plugin_configs.rs` (mask/restore/create/update), `queries/plugin_type_settings.rs`,
  `queries/update_dispatch.rs` (`merged_plugin_config`),
  `queries/software_items/host_assignments.rs` (`find_raw_active_config_txn`),
  `queries/autodiscovery/default_configs.rs` (compare + overwrite)
- `routes/plugin_configs/{crud,test_action,discover}.rs`,
  `routes/plugin_type_settings.rs`, `routes/instance_plugins.rs`,
  `routes/software_items/{version_check,version_check_dispatch,crud}.rs`,
  `routes/service_ws/handler/discovery.rs`, `routes/service_ws/handler/updates/replay.rs`,
  `routes/service_ws/handler/messages/discovery.rs`
- `crates/core/scheduler-runtime/src/executors/{queries,detect_version,fetch_releases,awaiting_restart,discover_software}.rs`
- `crates/plugins/infrastructure/proxmox/src/{surfaces,update_protection,reset}.rs`

Runtime-shaped sites needing special attention (custom `column_as` selects into row structs —
change the row-struct field types to the newtypes; alias names become irrelevant because the AAD
is compiled in): `executors/queries.rs`, `executors/fetch_releases.rs`,
`executors/awaiting_restart.rs`, `routes/software_items/version_check_dispatch.rs`,
`routes/service_ws/handler/updates/replay.rs`. Precision for `executors/queries.rs`
(`AgentAssignmentRow`): only `profile_config` (from `plugin_configs.config`) becomes
`Option<EncryptedPluginConfig>`; `assignment_config` (from `host_software_item_plugins.config`,
layer 3) **stays** `Option<serde_json::Value>` — layer 3 is not encrypted (§6). The same
per-field discrimination applies to every mixed-layer row struct in this list.

## 4. Migration and lazy encryption

### Migration (schema only)

One migration `m20260809_xxxxxx_encrypt_plugin_config_columns.rs` in `crates/shared/db`
(filename per convention; sequence assigned at implementation time):

- **Postgres:** `ALTER COLUMN ... TYPE text` for the three columns (`json`/`jsonb` → `text`).
  Postgres has no assignment cast from `json` to `text`, so a `USING config::text` clause is
  near-certain to be required — this is **pre-approved here** as a documented raw-SQL exception
  per `docs/development/database-migrations.md`, with the required inline comment naming the
  limitation (sea_query's alter-column builder cannot express `USING`). Verify against the
  Docker `database` integration suite; drop to the typed builder only if the cast unexpectedly
  works without `USING`.
- **SQLite:** no-op — `Json` columns are already stored as TEXT; no table recreation. This is a
  deliberate exception to the migration doc's "column type change → table recreation" default
  and must be recorded as an inline comment in the migration naming the rationale (SQLite type
  affinity: `Json` and `Text` share TEXT storage) and as a decision bullet in the ADR. Note in
  the same comment that the SQLite declared column type remains `json` while the entity declares
  Text — harmless via affinity, but a future schema-comparison gate must not be surprised by it.
- `host_software_item_plugins.config` is **not** touched (stays JSONB; layer 3 is not encrypted).
- `credential_updated_at` (nullable timestamp, `OffsetDateTime`) added to `plugin_configs` in the
  same migration.

**`down()` semantics (decided): refuse-if-encrypted.** `down()` counts rows in the three tables
whose column value starts with `ENC:` (typed `sea_query` LIKE filter) and returns an error naming
the tables if any exist; otherwise it reverts the column types and drops
`credential_updated_at`. A lossy "leave it as text" down would leave schema and entity
inconsistent; silent decryption in a migration is impossible for the same reason encryption is
(no DEK ring). Both backends get down() coverage tests, matching the existing drop-migration
precedent.

### Lazy encryption of existing rows

Extend `reencrypt_to_v3` (`crates/core/controller-runtime/src/reencrypt.rs`) with three upgrade
functions following the existing shape (skip when `!needs_v3_upgrade()`, re-save through the
newtype). This is exactly how every prior conversion shipped (`ca_certificates.key_pem`,
`oidc_providers.client_secret` had no backfill migration). New writes are encrypted immediately at
the query layer, so the plaintext window for existing rows closes at the first controller boot
after upgrade — acceptable for the single live deployment, and the read path tolerates legacy
plaintext indefinitely (no hard cutover).

Two hardening items ride along:

- **Fix the pagination defect first (prerequisite):** the existing upgrade helpers paginate with
  `.offset()/.limit()` and **no `ORDER BY`** while updating rows — pages shift under the scan and
  rows are skipped nondeterministically once a table exceeds `UPGRADE_CHUNK_SIZE` (100). Convert
  the shared pattern to keyset pagination (`.filter(Column::Id.gt(last_id)).order_by_asc(Id)`;
  `instance_plugin_setting` keys on `plugin_type_id`). Regression test with >100 rows.
- **Observability:** after the pass, log the count of remaining non-`ENC:` rows per encrypted
  column, so a deployment where the pass never ran (external-scheduler-only operation, `db-migrate`
  which deliberately skips re-encryption) is visible instead of silently identical to a converted
  one. Both this counter and the migration `down()` `ENC:` check must read the column via raw
  typed-`sea_query` **String selects, not entity loads** — with the eager-parse newtype, a single
  undecryptable row fails an entire `Vec<Model>` load, which would make exactly these diagnostics
  unrunnable when they are most needed.

No changes to the scheduler-standalone or `db-migrate` AAD registries are needed — compile-time
AAD removes those columns from the registry mechanism entirely. `reencrypt_to_v3` remains
controller-boot-only.

## 5. Schema-driven key-set masking (replaces fn-pointer mask/restore)

### Why the existing mechanism cannot be reused

The `declare_plugin!`-generated `mask_secrets`/`restore_secrets` fn-pointers round-trip through the
typed config struct: deserialize → mask → re-serialize. Serde fills defaults, so masking a
**sparse** object (a type-settings row or a per-host override holding two keys) returns a **full**
config document; the three-layer shallow merge then lets that expansion clobber lower layers, and
a GET→PUT round-trip persists it. Worse, several maskers inject the sentinel unconditionally
(GitHub sets `auth_token = Some("***")` even when it was absent). Reusing this for the sparse
layers corrupts configs; keeping it for profiles preserves a second, drift-prone hand-written
mechanism per plugin.

### New mechanism

Defined in `uptrakit-plugin-infrastructure-core` but **exposed to production code exclusively
through the existing `PluginConfigOps` trait surface consumed via
`uptrakit-plugin-infrastructure-registry`** — the sanctioned boundary crossing all current
mask/restore call sites already use (`queries/plugin_configs.rs`, `notifications.rs` import
`PluginConfigOps` from the registry crate). Web-api routes (`plugin_type_settings.rs`,
`instance_plugins.rs`) call the new trait methods the same way; no direct
`uptrakit-plugin-infrastructure-core` import from prod code, so no
`ci/plugin_semantic_boundary_allowlist.toml` entry should be needed — if one turns out to be,
it must carry the standard "no plugin-specific types cross" justification.

- `sensitive_paths(plugin_type) -> Vec<String>` — dotted JSON paths of secret fields. **This is
  a first-class descriptor property, not a pure form-schema derivation**: the effective set is
  the **union** of the schema-derived paths and any explicitly declared paths from
  `declare_plugin!` — union semantics stated deliberately: an explicit declaration may only add,
  never shrink the derived set (replace-semantics would let one hand-declared path silently
  unmask the schema-derived ones). The derived half covers **all three** schema sources
  (`config_form_schema()`, `type_settings_form_schema()`, and the instance-plugin
  `instance_config` form schema) and treats a field as sensitive when `sensitive == true` **or**
  its type is `Password`/`SshPrivateKey` — proxmox's `api_token` is `Password`-typed with no
  `.sensitive()` flag, so a flag-only derivation would miss it. This is load-bearing, not
  optional: the telegram (`bot_token` **and** `webhook_secret` — it masks two fields) and
  webhook (`secret`) notification plugins mask secrets today but have **no form schema at all**
  (`form_schema()` defaults to empty — their UI forms are hand-built) — a schema-only
  derivation would silently unmask live credentials the moment the fn-pointers are deleted.
  Those plugins declare their paths explicitly, and the plan derives the complete
  underived-masker set mechanically (enumerate every `with_secrets_masked` impl; cross off
  those whose fields the widened derivation now covers; the known set is telegram, webhook,
  proxmox — email/npm/shell/proxmox-helper-scripts are default no-op maskers) rather than
  trusting this spec's enumeration. **Permanent forward guard** (survives the one-time parity
  gate): a catalog assertion that every `Password`/`SshPrivateKey`-typed schema field and every
  `SecretString`-typed config field resolves to a path in `sensitive_paths()` — a new plugin
  adding a credential field without declaration is a build break, not a silent unmask.
  Path normalization: the frontend strips a leading `_` per path segment when building JSON keys
  (`auth._type` → `auth.type` — a convention that today lives only in two Svelte files);
  `sensitive_paths()` applies the same normalization server-side so form-field keys and JSON
  keys cannot diverge, and a catalog-wide test asserts every sensitive path resolves against a
  **populated per-plugin secret fixture** (a path that matches nothing is a bug, not a no-op).
  Plain `sample_config()` cannot serve here: every secret field is `Option` +
  `skip_serializing_if` + `None`-default, so samples contain **no** secret keys at all, and
  docker's `auth.password`/`auth.token` are mutually exclusive enum variants no single sample
  satisfies — the fixture set is per-plugin, secrets populated, one fixture per variant where
  variants exist.
  Verified real examples: `api_token`, `auth_token`, `auth.password`, `auth.token`.
  Stated structural limit: dynamic key spaces (webhook's `headers` map, where any
  `x-api-key`-style header value may be secret) cannot be expressed as static paths — recorded
  as a known limitation of the mechanism, unchanged from today's behavior.
- `mask_present_keys(value, paths) -> Value` — replace values **only at paths present** in the
  object with `"***"`; absent paths stay absent. Sparse-preserving by construction.
- `restore_masked_keys(incoming, stored, paths)` — for each path where incoming equals `"***"`,
  copy the stored value; absent in incoming stays absent.
- **Post-validate prune (variant-switch correctness):** after restore and `validate_config`, the
  profile write path obtains the typed round-trip of the config and **prunes** (prune-only —
  never adds serde defaults back) any sensitive path present in the restored JSON but absent
  from the round-trip. Mechanism, stated explicitly because the existing `ConfigOps::validate`
  returns `()` and cannot supply a typed struct: `ConfigOps` gains a **new
  `normalize: fn(&Value) -> Result<Value, PluginConfigValidationError>` slot**, wired by
  `declare_plugin!` as deserialize→reserialize of the typed config (the macro change is budgeted
  work, not incidental) — the round-trip survives as a _normalization_ primitive; only the
  mask/restore fn-pointers are deleted. Without the prune, a Docker Basic→Bearer switch
  resurrects the abandoned `auth.password`: the frontend submits `"***"` for the now-invisible
  field, path-restore refills the old password, and tagged-enum deserialization tolerates the
  stale key — the retired credential would persist (encrypted) forever. Accepted residuals,
  recorded: non-sensitive stale variant keys (`auth.username`) still survive a switch (no
  credential exposure); prune applies to the profile layer only — sparse layers get a **catalog
  assertion that no type-settings-capable or instance-config-declaring plugin has a non-empty
  effective `sensitive_paths()`** (stated over the effective union, not over form schemas alone
  — explicit declarations from schema-less plugins must also trip it; true today: the
  type-settings plugins are package managers, all secret-free), converting the latent
  sparse-layer resurrection hole into a build break if it ever becomes live. A Basic↔Bearer
  transition test is mandatory (§9).
- `sensitive_value_changed(incoming_restored, stored, paths) -> bool` — drives
  `credential_updated_at` (§8).
- **Post-restore sentinel assertion — on every write path including create:** if any sensitive
  path still equals `"***"` after restore, the write is rejected with a 400 validation error.
  Create has no stored row to restore from, so on create the assertion fires directly — today a
  GET-masked → POST clone flow stores `"***"` as the live token (verified: `create_plugin_config`
  / `create_plugin_config_in_tx` have no restore call). This converts today's silent credential
  corruption (restore silently no-ops on deserialize failure) into a visible error. Consequence,
  documented in user-facing docs: a literal secret value `"***"` is unsettable.

`PluginConfigOps::mask_config_secrets` / `restore_config_secrets` keep their signatures but are
reimplemented on top of this (descriptor-driven, no per-plugin fn-pointers); the `ConfigOps`
`mask_secrets`/`restore_secrets` slots and the per-plugin `with_secrets_masked`/
`restore_secrets_from` impls are deleted. That deletion orphans the corresponding
`declare_plugin!` macro wiring (`macros.rs` mask/restore arms) and per-plugin trait imports — the
plan must name and prune them explicitly. Unknown plugin type keeps today's passthrough behavior
(mask is a no-op); this is unreachable in practice because responses are already filtered to
cataloged plugins.

**Parity gate before deletion (load-bearing, not a formality):** a catalog-wide unit test
asserts, for every plugin descriptor, that the field set masked by the legacy
`with_secrets_masked` is a subset of `sensitive_paths()` — run against the **populated
per-plugin secret fixtures** defined above, never against `sample_config()`: the legacy maskers
are `is_some()`-guarded, so on a `Default` sample they mask nothing and the gate would pass
vacuously exactly where it must bite (docker/webhook/telegram/proxmox). This is the gate that
catches schema-less maskers and any config struct whose secret field lacks a declaration — the
legacy path must not be deleted until it passes. **Fixture density is itself asserted** (the
vacuity hole must not survive one level down): for every plugin/variant with a legacy masker,
`masked_fixture != fixture` — a fixture that leaves a secret unset would let the `is_some()`
guard mask nothing and pass the subset check vacuously. The permanent successor test keys on
`sensitive_paths()` itself, not on `SecretString` typing (telegram's `bot_token` and webhook's
`secret` are plain `String` — a type-based net misses them): for every plugin, fixture values
at every sensitive path never survive `mask_present_keys`.

### Applied to each store

| Store                                               | Read (mask)                                                                                          | Write (restore + sentinel assert)                                                                                                                                                                                                                                                                                                                                  |
| --------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `plugin_configs`                                    | existing call sites, now sparse-preserving                                                           | existing call sites + assertion                                                                                                                                                                                                                                                                                                                                    |
| `plugin_type_settings`                              | **new** — `model_to_response` in `routes/plugin_type_settings.rs` masks before returning; list + get | **new** — upsert restores `"***"` from the stored row before validation/persist                                                                                                                                                                                                                                                                                    |
| `instance_plugin_setting`                           | **new** — `routes/instance_plugins.rs` masks `current_config`                                        | **new** — update path restores from stored row                                                                                                                                                                                                                                                                                                                     |
| ConfigTest (`routes/plugin_configs/test_action.rs`) | n/a                                                                                                  | **new** — restore stored secrets into the merged config before dispatch, then sentinel-assert; today a UI-echoed `"***"` overwrites the real token and tests a bogus credential. Overlap note: the pending "Plugin Config-Test Connectivity Probes" spec registers the same restore prerequisite — whichever lands first implements it, the other rebases onto it. |

Frontend: no shape change. The profile editor already round-trips `"***"`; verify the
type-settings and instance-plugin editors tolerate masked values the same way (expected: shared
form components; verification item for the plan).

## 6. Layer 3: reject sensitive keys (decided; owner-confirmed)

Per-host overrides exist for per-host _tweaks_ (`asset_patterns`, `channel`, `install_path`), not
credential storage. Instead of extending encryption + masking + a restore path to 8+ write sites
including one (autodiscovery) that has no request to restore from, layer 3 becomes structurally
secret-free:

- **HTTP path** (`validate_assignment` in `queries/software_items/host_assignments.rs`): reject
  any override containing a sensitive path (typed validation error → 400). Applies to assign and
  both update variants.
- **Autodiscovery path** (`queries/autodiscovery/discovery_items.rs`): strip sensitive paths from
  `target.config_override` with a `tracing::warn!` naming plugin type and dropped keys (agent
  reports must not be rejected wholesale over one key; agents never legitimately emit
  credentials here). **Fail-closed for uncataloged plugin types:** agent and controller have
  independent feature lanes, so an agent can report an override for a plugin type this controller
  binary has no descriptor for — its sensitive-path set is unknowable, and stripping against an
  empty set would be fail-open. When `ops.get(plugin_type)` is `None`, drop `config_override`
  **wholesale** with a warn naming the plugin type.
- **Startup sweep:** the reencrypt pass (§4) additionally counts existing layer-3 rows containing
  sensitive paths — plus rows whose plugin type has no live descriptor (same unknowable-paths
  reasoning) — and warn-logs both counts (single deployment: owner verifies zero; no destructive
  auto-strip of stored data).
- **Frontend:** `EditHostAssignmentModal.svelte` stops rendering sensitive fields in the override
  form (filter on the `sensitive` flag it already receives; both form and raw-JSON modes get
  server-side enforcement regardless).
- **Docs:** remove the doc-comment invitation to per-host token overrides
  (`crates/plugins/releases/github/src/config.rs:34-35`) and document the layer-3 no-secrets rule.

Consequence (accepted): the theoretical per-host-credential capability is removed. Anyone needing
per-host credentials creates a per-host plugin config profile instead — same capability, encrypted
and masked.

Because layer 3 can no longer carry secrets, it needs no encryption; this is the recorded answer
to scope item 4 of the original brief.

## 7. Prerequisite defect fixes (same series, before or with the main change)

1. **`reencrypt_to_v3` keyset pagination** — §4. Without it, the new plugin-config upgrade
   functions silently skip rows on any table >100 rows.
2. **`notification_channels` restore-on-write** — verified missing: no `restore` call anywhere in
   `queries/notifications.rs` or `routes/notifications.rs`. A GET→PUT of a channel persists
   `"***"` as the live credential. Fix with the **new** §5 mechanism (path-based restore +
   sentinel assert) — not the legacy fn-pointers, which this series deletes and whose masking is
   the only thing the channel plugins declare today. Telegram (`bot_token`) and webhook
   (`secret`) get explicit `sensitive_paths` declarations per §5 (they have no form schema to
   derive from).
3. **Sentinel assertion** — §5; applied to every restore site including the two above.

## 8. `credential_updated_at`

- Nullable `timestamp` column on `plugin_configs` (migration §4).
- Stamped (set to now) when:
  - **create**: the incoming config contains at least one sensitive path with a non-empty,
    **non-sentinel** value (a `"***"` at a sensitive path on create is already a 400 per §5's
    create-path assertion — it must never read as "credential freshly set");
  - **update**: after `restore_masked_keys`, `sensitive_value_changed(incoming, stored, paths)`
    is true (covers add, change, and removal of a credential).
- Never stamped by non-sensitive edits, enable/disable, or soft delete.
- **Third writer covered:** `find_or_create_default_plugin_config`
  (`queries/autodiscovery/default_configs.rs`) creates and unconditionally overwrites profile
  configs from agent discovery reports, bypassing the REST paths — it goes through the same
  assert+stamp helper as create/update. Failure semantics on this non-REST path, stated
  explicitly: a sentinel violation **skips that config write** (stored config kept, warn naming
  plugin type) and never fails the discovery report as a whole. This path has **no restore**
  (there is no masked round-trip to restore from): an agent report omitting a secret replaces
  the stored config and drops the credential — accepted, and honestly stamped, since the stamp
  fires on any sensitive-value change. Without the routing, an agent report could silently
  replace or remove a credential while `credential_updated_at` stays stale — a lying timestamp
  is worse than none for the future rotation-reminder feature.
- Exposed as a non-skipped field on `PluginConfigView` (the audit snapshot view) — a timestamp is
  not secret material, so it is safe alongside the `#[audit(skip)]` config field.
- Not added to `PluginConfigResponse` / REST in this change (rotation reminders are a separate
  item and can expose it later; keeping it out avoids an OpenAPI/client/frontend regen cycle).
- Detection mechanism is the `sensitive_paths` set (§5), which subsumes the
  "value-differs-after-restore" formulation: comparison happens on the restored incoming config
  against the stored one, path-wise.

## 9. Testing and acceptance

Test-harness reality: `TestApp` runs `enable_plaintext_mode()` process-globally, so all existing
API tests exercise the new code paths with crypto as a passthrough — existing profile CRUD,
config-test, and effective-config tests must stay green unchanged (compile fixes only: direct
entity inserts in test files construct the newtypes via `::new()`; affected files include
`integration_tests/plugin_configs.rs`, `integration_tests/plugin_type_settings.rs`,
`routes/service_ws/handler/tests.rs`, `routes/service_ws/handler/messages/tests.rs`,
`routes/update_batches.rs`, `crates/core/functional-tests/tests/support/fixtures.rs`, proxmox
surface tests, and `crates/core/integration-tests/tests/database/migrations.rs` — the latter
runs with **no crypto initialization at all**, so it must enable plaintext mode (or init a key)
before constructing the newtypes, else `::new()` returns `NotInitialized`).

Real-key tests (placed where a real master key can be initialized without colliding with
plaintext-mode binaries — the `controller-runtime` reencrypt test module pattern, key `[0x42;32]`):

1. **Raw-column acceptance:** create a plugin config with a known token through the query fn; read
   the raw column back via a typed `sea_query` select as `String`; assert the value starts with
   `ENC:v3:` and does not contain the token substring. Same for a type-settings row and an
   instance-plugin-settings row.
2. **Lazy upgrade:** insert legacy plaintext rows via typed builders, run `reencrypt_to_v3`,
   assert all three tables converted; >100-row regression test proving keyset pagination misses
   nothing.
3. **Aliased-read decryption:** run the real scheduler assignment query
   (`executors/queries.rs`) against an encrypted row with a real key and assert the profile
   config decrypts — proves alias-immunity of the compile-time AAD end to end.
4. **Rotation coverage:** once rows are `ENC:v3`, master-key rotation is DEK re-wrap by design;
   the acceptance claim "key rotation re-encrypts both tables" is satisfied by v3 conversion —
   assert `needs_v3_upgrade()` is false for all rows post-pass.

Behavioral tests (plaintext-mode, `TestApp`):

1. Type-settings GET returns `"***"` at sensitive paths; PUT echoing `"***"` preserves the stored
   secret; PUT with a new value replaces it; sparse type-settings round-trip does **not** gain
   keys (anti-expansion regression).
2. Instance-plugin-settings GET masked / PUT restore, same shape.
3. **Docker Basic↔Bearer variant switch:** update from Basic auth to Bearer with the frontend's
   actual submission shape (stale `auth.password: "***"` included) — persisted config carries
   the new token and **no** resurrected password (post-validate prune, §5); create with `"***"`
   at a sensitive path → 400 (create-path sentinel).
4. Layer-3: HTTP override containing `auth_token` → 400; autodiscovery `config_override` with a
   sensitive key → stored stripped + warn; non-sensitive overrides unaffected.
5. `credential_updated_at`: stamped on secret create/change, untouched on non-secret update.
6. Sentinel assertion: update where restore cannot resolve `"***"` → 400 (both plugin config and
   notification channel variants).
7. Masker unit tests: sparse preservation, dotted paths, absent-path no-injection, parity gate
   (§5).
8. Migration `down()` on both backends: refuses with `ENC:` rows present, succeeds on
   plaintext-only (extends the existing both-backend down() test precedent).

Success/failure paths covered per the testing standard; no upstream-crate behavior tested.

## 10. Alternatives considered

- **Column rename + bare-name AAD registry entries** (original brief's implied shape) — rejected:
  the registry keys on the SELECT alias, so renames don't protect aliased/joined reads
  (`"profile_config"` alias is live today); requires a second recreation of a table whose first
  recreation already dropped FKs; adds entries to three drifting per-binary registry copies.
- **Manual `encrypt_str`/`decrypt_str` in the query layer** (service-config precedent) — rejected:
  no compiler-forced coverage of ~25 read sites across four crates, and the existing
  implementation of that pattern silently falls back to plaintext on error.
- **Encrypting in the migration** — rejected: DEK ring and AAD context initialize after
  migrations; would emit `ENC:v2` values inconsistent with the v3 fleet; contradicts every shipped
  conversion in this codebase.
- **Encrypt + mask layer 3** — rejected by owner in favor of sensitive-key rejection (§6): smaller
  surface, no restore path for the request-less autodiscovery writer, and per-host credentials
  remain available via per-host profiles.
- **Reusing per-plugin `with_secrets_masked` fn-pointers for sparse layers** — rejected: serde
  round-trip expands sparse objects and corrupts the three-layer merge (§5).

## 11. Deliverables

Code (crates): `uptrakit-crypto` (decode API + `encrypted_column!`), `uptrakit-shared-db`
(newtypes, entity changes, migration), `uptrakit-plugin-infrastructure-core` (key-set masker,
sensitive-path collection, fn-pointer retirement), `uptrakit-plugin-infrastructure-registry`
(trait-surface exposure of the new masker methods; possible
`ci/plugin_semantic_boundary_allowlist.toml` entry per §5), `uptrakit-web-api-queries`,
`uptrakit-web-api`, `uptrakit-scheduler-runtime` (scheduler-runtime executors),
`uptrakit-controller-runtime` (reencrypt), plugin crates (delete
`with_secrets_masked`/`restore_secrets_from` impls; github doc comment), frontend
(`EditHostAssignmentModal` sensitive-field filter; `unflattenConfig` in both
`PluginConfigsTab.svelte` and `EditHostAssignmentModal.svelte` must skip non-visible fields so
hidden `"***"` values are not submitted — server-side prune remains the enforcement; verify
type-settings editor sentinel round-trip).

Documentation (non-optional):

- **New ADR** via `adrs new "Compile-time AAD encrypted column newtypes"` — records: newtype
  mechanism vs bare-name registry, the alias/prefix failure mode, layer-3 no-secrets rule,
  refuse-if-encrypted down() policy, the SQLite no-table-recreation exception (type-affinity
  rationale, §4), and the intent to migrate existing registry columns to newtypes over time (out
  of scope here).
- `docs/security/secrets-and-encryption.md` — add the three stores to the encrypted-at-rest
  inventory; document the `"***"` sentinel semantics (unsettable literal) and
  `credential_updated_at`.
- `docs/development/plugin-guidelines.md` — masking is now schema-driven: `.sensitive()` on the
  form field **is** the masking/stamping mechanism; plugin authors must mark credential fields;
  the `with_secrets_masked` contract is removed.
- `docs/development/plugin-system.md` — layer-3 overrides cannot carry sensitive fields; per-host
  credentials via per-host profiles.
- `AGENTS.md` — update the existing command-bearing/secret-handling rule area with one line:
  plugin config stores are encrypted at rest; layer-3 overrides reject sensitive fields (within
  size budget; run markdownlint).
- `audit-catalog.toml` — no new state-changing sites expected (`credential_updated_at` rides the
  existing catalogued update action; the reencrypt pass is startup maintenance, not a catalogued
  mutation) — verify with `cargo xtask audit-coverage-check`.
- No OpenAPI/asyncapi regen expected (REST and wire shapes unchanged) — verify with the staleness
  gates; if any response doc-comment changes touch the schema, run `./scripts/regen-api.sh`.

## 12. Out of scope

- Rotation reminders and credential validation (`ConfigTest` still fake-succeeds for
  controller-side plugins — separate known defect; only the `"***"`-merge fix in §5 ships here).
- Exposing `credential_updated_at` over REST.
- Migrating the existing bare-name registry columns (`notification_channels.config`,
  `ca_certificates.key_pem`, …) to compile-time-AAD newtypes — recorded in the ADR as direction.
- NATS/wire transport of merged configs (already protected in transit by
  `config_protection.rs` + mTLS; unchanged).
- The pre-existing `docs/api` REST path drift (tracked separately).
- Secrets living outside the three converted columns, e.g. telegram's _global_ bot token
  (`global_telegram.bot_token` in global settings) — different store, existing settings-layer
  encryption machinery applies; this spec covers the plugin-config stores only. (Plan-time
  verification item: confirm that setting actually flows through the encrypted settings store
  rather than plaintext `global_settings` — asserted here, not yet verified.)

## 13. Constraints checklist (standards snapshot conformance)

- No raw SQL: migration uses typed builders; the single potential exception (Postgres `USING`
  cast) is pre-declared with the required inline-comment policy if it materializes.
- Migration filename/timestamp conventions, `down()` implemented, both-backend coverage.
- No `unwrap`/`panic!`/`#[allow]`; typed errors + `rootcause` (`report!`/`bail!`); `Result<T>`
  alias coverage in touched query modules.
- No new dependencies — everything is workspace-internal; no `[workspace.dependencies]` changes
  expected.
- Feature flags: `encrypted_column!` lives behind the existing additive `sea-orm` feature; no
  `cfg(not(feature))`.
- Tenant scoping via `TenantDb` untouched; no N+1 introduced (reencrypt uses keyset batches).
- Tests: success + failure paths; no real sleeps; `TestApp` for endpoint tests; no upstream-crate
  behavior tests.
- Gates: fmt, clippy (both feature sets), deny, tests, `verify_db_access_policy`,
  `audit-coverage-check`, `openapi-client-check`, markdownlint, AGENTS.md budget.
