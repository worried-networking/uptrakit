# Typed Dynamic Config Boundaries Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to
> implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Tighten dynamic config/public JSON boundaries so object-shaped config is represented explicitly, finite patch fields get typed parsing at
the Rust boundary without changing their current wire contract, and SMTP settings snapshots stop being rebuilt from ad hoc positional getter chains.

**Architecture:** Keep persistence storage JSON-backed, but move the Rust-side boundary to validated wrapper types and typed settings snapshots. The
first phase preserves external wire shapes for the named REST contracts while moving parsing, validation, and response construction onto typed Rust
structures. This plan runs after the plugin-extension typing track for `crates/plugins/notifications/email/src/surfaces.rs`; its email-surface work is
limited to settings/config shape cleanup on top of that earlier typed boundary.

**Current state (as of plan review 2026-06-03):** Tasks 1 and 2 are substantially already landed. `JsonObjectMap`, `JsonObjectInput`,
`IconUrlPatch`, `JsonObjectMapPatch`, `decode_prefixed_settings`, `load_typed_settings_by_prefix`, and `load_typed_global_settings_by_prefix` all
exist in the codebase. Tasks 1 and 2 therefore target cleanup, deduplication, and standards gaps rather than net-new code. Task 3 still has real
remaining work but the target symbols have changed.

**Tech Stack:** Rust workspace crates, `serde`, `serde_json`, web-api DTOs, email notification surfaces, shared settings store/raw settings helpers,
cargo package tests/checks

---

## File Structure

### Public request/response contracts

- Modify:
  [`crates/shared/web-api-types/src/notifications/channels.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/notifications/channels.rs)
  Responsibility: consolidate the duplicate `JsonObjectMap` definition — it exists in both `channels.rs` and `software_items.rs` as identical copies.
  One canonical definition should live in a shared location (e.g., `crates/shared/web-api-types/src/json_object.rs`) and be re-exported from both
  modules. Add `#[non_exhaustive]` to the public struct.
- Modify:
  [`crates/shared/web-api-types/src/software_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/software_items.rs)
  Responsibility: use the canonical `JsonObjectMap` from the shared location; verify `IconUrlPatch`, `JsonObjectMapPatch`, and
  `latest_release_metadata` doc annotations are correct and have `#[non_exhaustive]` where required.

### Settings snapshots

- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)
  Responsibility: replace the 11-argument `settings_map_to_snapshot` positional getter function with a call to
  `uptrakit_shared_db::raw_settings::decode_prefixed_settings` for all non-secret fields, while retaining the encryption/decryption path for the
  `password` field (which is `SecretString` at rest and cannot be serde-decoded directly).
- Verify (no changes — intentionally out of scope):
  [`crates/ui/web-api-queries/src/notification_settings.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-queries/src/notification_settings.rs)
  The flat `smtp.*`-keyed bag it produces is the wire contract consumed by `smtp_from_settings_map` in the email plugin. Replacing the flat map
  with a stripped-key re-serialization would break SMTP delivery. This file stays unchanged; its map-rebuild pattern is intentional and must be
  documented. The actual `dispatcher.rs` at `crates/ui/web-api/src/notifications/dispatcher.rs` is a one-line re-export and requires no changes.
- Verify (no changes expected):
  [`crates/ui/web-api-auth/src/settings_store.rs`](/Users/andreyyantsen/Development/uptrakit/crates/ui/web-api-auth/src/settings_store.rs)
  `load_typed_settings_by_prefix` and `load_typed_global_settings_by_prefix` already exist here.
- Verify (no changes expected):
  [`crates/shared/db/src/raw_settings.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/db/src/raw_settings.rs)
  `decode_prefixed_settings`, `RawSettingsError::Decode`, `load_settings_by_prefix`, and `load_global_settings_by_prefix` already exist here.

### Verification commands

- `cargo fmt --all`
- `cargo test -p uptrakit-web-api-types`
- `cargo check -p uptrakit-web-api-types`
- `cargo test -p uptrakit-notification-plugin-email`
- `cargo check -p uptrakit-notification-plugin-email`
- `cargo check -p uptrakit-web-api`
- `cargo check -p uptrakit-web-api-queries`
- `cargo clippy --all-targets --all-features`
- `python3 ci/check_plugin_semantic_boundary.py`
- Negative check (must return no matches):
  `rg -n "settings_map_to_snapshot\b" crates/plugins/notifications/email/src/surfaces.rs`
- Positive check (must return matches in surfaces.rs):
  `rg -n "decode_prefixed_settings" crates/plugins/notifications/email/src/surfaces.rs`

---

### Task 1: Share `JsonObjectMap` Internals and Fix `#[non_exhaustive]` Gap

**Context:** `JsonObjectMap` is defined identically in both `channels.rs` and `software_items.rs`, but the **two `TryFrom<Value>` impls emit
different `ValidationError.field` values** (`"config"` vs `"config_override"`). That difference is part of the wire-side error contract — serde
invokes `TryFrom::try_from(value)` (a single-argument trait method) on request body deserialization for `JsonObjectMapPatch` and on response
deserialization for `NotificationChannelResponse.config`, so the `field` baked into `TryFrom` is the field that surfaces to clients. Collapsing to
a single `JsonObjectMap` type with one `TryFrom` impl would silently change the error JSON at one of the call sites.

The fix: keep two distinct public newtypes (`ChannelConfigJsonMap`, `SoftwareItemConfigOverrideJsonMap` — or names that match the current call
sites), share their internals via a private inner type or helper module, and add `#[non_exhaustive]` to every public newtype and to `JsonObjectInput`.
This eliminates implementation drift without changing serde-observable behaviour.

**Files:**

- Modify:
  [`crates/shared/web-api-types/src/notifications/channels.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/notifications/channels.rs)
- Modify:
  [`crates/shared/web-api-types/src/software_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/software_items.rs)
- Create or modify: `crates/shared/web-api-types/src/json_object.rs` (new shared module)
- Modify: `crates/shared/web-api-types/src/lib.rs` (add `pub mod json_object;`)

- [ ] **Step 1: Audit existing `JsonObjectMap` definitions**

  Run:

  ```bash
  rg -n "pub struct JsonObjectMap" crates/shared/web-api-types/src/
  ```

  Confirm two definitions exist — one in `notifications/channels.rs`, one in `software_items.rs`.

- [ ] **Step 2: Add a private shared inner and add `#[non_exhaustive]` to the public newtypes in place**

  Create `crates/shared/web-api-types/src/json_object.rs` with **private** shared internals:

  ```rust
  /// Private, crate-internal building block shared by the public newtypes that need to enforce
  /// "JSON object only" with a call-site-specific `ValidationError.field` name.
  pub(crate) fn parse_json_object(
      value: serde_json::Value,
      field: &'static str,
  ) -> Result<serde_json::Map<String, serde_json::Value>, ValidationError> { /* … */ }

  pub(crate) fn validate_json_object(
      value: &serde_json::Value,
      field: &'static str,
  ) -> Result<(), ValidationError> { /* … */ }
  ```

  Add `pub(crate) mod json_object;` to `lib.rs`.

  Then, in each existing file:

  - `notifications/channels.rs` — keep the public newtypes named as today (`JsonObjectMap`, `JsonObjectInput`). Add `#[non_exhaustive]` to both
    structs. Replace the body of `TryFrom<Value> for JsonObjectMap` with `json_object::parse_json_object(value, "config").map(Self)`. Replace the
    body of `JsonObjectInput::validate` with `json_object::validate_json_object(&self.0, "config")`.
  - `software_items.rs` — keep the public newtype named as today (`JsonObjectMap`). Add `#[non_exhaustive]` to the struct only. Replace the body of
    `TryFrom<Value> for JsonObjectMap` with `json_object::parse_json_object(value, "config_override").map(Self)`.

  **Do not add `#[non_exhaustive]` to `IconUrlPatch` or `JsonObjectMapPatch`.** These are closed three-state sum types
  (`Keep` / `Set(...)` / `Clear`) with no foreseeable additional variants, and their `Set(...)` tuple variant is constructed from external crates
  today (`crates/ui/cli/src/commands/software_items.rs:586-590`,
  `crates/ui/web-api-queries/src/queries/software_items/host_assignments.rs:975-1001`,
  `crates/shared/openapi-client/src/software_items.rs:223,295`, and exhaustive matches in
  `crates/ui/web-api-queries/src/queries/software_items/crud.rs:363-365,795-797`). `#[non_exhaustive]` would break those construction and match
  sites (E0639 on tuple variants, `non_exhaustive_omitted_patterns` warnings on exhaustive matches). The standards-snapshot rule scopes the
  attribute to **extensible** public enums; these patch enums are not extensible.

  This intentionally keeps the **two distinct public `JsonObjectMap` types** so serde's single-arg `TryFrom::try_from(value)` continues to emit the
  correct per-site `ValidationError.field` ("config" vs "config_override"). What is removed is the duplicated parsing logic, not the duplicated
  public types.

  > **Why not consolidate into one shared `JsonObjectMap` type?** Serde's `#[serde(try_from = "...")]` calls the one-argument `TryFrom::try_from`
  > trait method. There is no way to pass a per-call-site field name through serde, so a single shared `JsonObjectMap` would have to pick one
  > `field` string and silently change the wire-side error JSON at the other call site. Two thin newtypes sharing a private parser preserves the
  > existing error contract while removing the implementation drift.

- [ ] **Step 3: Verify call sites compile and tests still pass**

  No changes to the public types' identity, names, or field locations are required — only `#[non_exhaustive]` is added and the `TryFrom` /
  `validate` bodies are delegated to the new private helpers. Run:

  ```bash
  rg -n "ValidationError \{ field: \"(config|config_override)\"," crates/shared/web-api-types/src/
  ```

  Expected: matches show that the call-site-correct `field` strings are still emitted from the `json_object::parse_json_object` / `validate_json_object`
  helpers — at minimum, the existing call sites in `channels.rs` and `software_items.rs` should be the only ones threading these field names through
  the helpers, and other unrelated `field: "config"` sites elsewhere in the crate are not touched.

- [ ] **Step 4: Verify build and tests pass**

  Run:

  ```bash
  cargo test -p uptrakit-web-api-types
  cargo check -p uptrakit-web-api-types
  ```

  Expected: PASS. No functional change — only type consolidation and attribute addition.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only crates/shared/web-api-types/ -m "refactor(web-api-types): share JsonObjectMap parsing helper and add non_exhaustive"
  ```

---

### Task 2: Verify Software Item Boundaries Are Complete

**Context:** `IconUrlPatch`, `JsonObjectMapPatch`, and all `HostPluginRoleAssignment` / `UpdateHostAssignmentRequest` / `HostPluginRoleSummary`
typed boundaries are already landed. This task validates the spec acceptance criteria are met and adds missing doc annotations.

**Files:**

- Modify:
  [`crates/shared/web-api-types/src/software_items.rs`](/Users/andreyyantsen/Development/uptrakit/crates/shared/web-api-types/src/software_items.rs)

- [ ] **Step 1: Confirm typed boundaries are in place**

  Run:

  ```bash
  rg -n "icon_url: IconUrlPatch|config_override: Option<JsonObjectMap>|config_override: JsonObjectMapPatch" crates/shared/web-api-types/src/software_items.rs
  ```

  Expected: all three field patterns found.

- [ ] **Step 2: Verify `latest_release_metadata` doc annotation**

  Confirm `SoftwareItemResponse.latest_release_metadata` and `SoftwareItemHostSummary.latest_release_metadata` both carry the doc comment:

  ```rust
  /// Intentionally left dynamic: payload shape is plugin-defined at the REST boundary.
  ```

  Add it if missing. After the step:

  ```bash
  rg -n "Intentionally left dynamic" crates/shared/web-api-types/src/software_items.rs
  ```

  Expected: at least 2 matches (one for each field).

- [ ] **Step 3: Confirm `#[non_exhaustive]` is correctly scoped**

  Run:

  ```bash
  rg -n "#\[non_exhaustive\]" crates/shared/web-api-types/src/software_items.rs
  ```

  Expected: `TriggerUpdateStatus` (extensible wire-protocol enum) carries it. The `JsonObjectMap` newtype struct in this file carries it (added by
  Task 1). `IconUrlPatch` and `JsonObjectMapPatch` must **not** carry `#[non_exhaustive]` — they are closed three-state sum types whose
  `Set(...)` tuple variant is constructed from external crates (CLI, query layer, openapi-client) and is exhaustively matched in
  `web-api-queries`. Adding `#[non_exhaustive]` there would break those callers (E0639 on tuple-variant construction and
  `non_exhaustive_omitted_patterns` warnings on exhaustive matches).

- [ ] **Step 4: Run DTO suite**

  Run:

  ```bash
  cargo test -p uptrakit-web-api-types
  ```

  Expected: PASS.

- [ ] **Step 5: Commit if changes were made**

  ```bash
  git commit --only crates/shared/web-api-types/src/software_items.rs -m "docs(web-api-types): annotate intentionally dynamic fields and add non_exhaustive to patch enums"
  ```

---

### Task 3: Replace SMTP Positional Getter Chains With Typed Snapshot Decoding

**Context:** Two distinct SMTP-map-rebuild patterns remain:

1. `settings_map_to_snapshot` in `email/src/surfaces.rs` — a 12-argument positional function that duplicates the field extraction logic that
   `decode_prefixed_settings` handles generically. The `password` field is `SecretString` encrypted at rest and **cannot** be serde-decoded from a
   raw DB map; the decryption path must be retained separately.

2. `smtp_raw_to_json_map` / `load_smtp_map` / `load_global_smtp_map` in
   `crates/ui/web-api-queries/src/notification_settings.rs` — build a raw `serde_json::Map` rather than a typed snapshot for the `deliver()` bag.
   This is the actual dispatcher-side target. Note: `crates/ui/web-api/src/notifications/dispatcher.rs` is a one-line re-export and is NOT a target
   for changes.

**Files:**

- Modify:
  [`crates/plugins/notifications/email/src/surfaces.rs`](/Users/andreyyantsen/Development/uptrakit/crates/plugins/notifications/email/src/surfaces.rs)

- [ ] **Step 1: Add a failing behavioural-parity test for the new SMTP snapshot helper**

  Generic `decode_prefixed_settings` coverage already exists in `raw_settings.rs` (`decode_prefixed_settings_deserializes_prefixed_values`,
  `decode_prefixed_settings_ignores_malformed_field_values`). The new test must instead protect the **specific** semantics that
  `settings_map_to_snapshot` currently provides over and above plain serde, so the Task 3 refactor cannot silently change them. At minimum, the
  parity test must cover:

  - empty string in a non-secret field collapses to `None` (current `get_str` filters `!s.is_empty()`)
  - port stored as JSON string (e.g. `"587"`) is accepted and yields `Some(587)` (current `get_u16` does `v.as_str().and_then(|s| s.parse().ok())`)
  - unknown / missing `tls_mode` normalizes to `"starttls"` (current `normalize_tls_mode` behaviour, preserved by `surfaces.rs:626`)
  - encrypted password ciphertext in the raw map round-trips through the existing decrypt path and ends up as the expected `SecretString`
    (use the plaintext-mode test helpers per `EncryptedString` testing convention)

  Add it in `email/src/surfaces.rs` against the new helper introduced in Step 2. **Assert against explicit expected values**, not against the
  legacy `settings_map_to_snapshot` (which Step 2 removes — referencing it from the test would leave the test temporarily unbuildable mid-task):

  ```rust
  #[test]
  fn smtp_snapshot_from_raw_preserves_legacy_get_str_get_u16_semantics() {
      // populate a raw map that exercises the four edge cases above
      // (empty-string-to-None, port-as-string accepted, unknown tls_mode → "starttls",
      // encrypted password round-trips via decrypt_str),
      // then assert each SmtpSettingsSnapshot field equals the explicit expected value.
  }
  ```

  Run:

  ```bash
  cargo test -p uptrakit-notification-plugin-email smtp_snapshot_from_raw_matches_legacy_settings_map_semantics -- --exact
  ```

  Expected: FAIL because `smtp_snapshot_from_raw` does not exist yet.

- [ ] **Step 2: Replace `settings_map_to_snapshot` in `surfaces.rs`**

  The 12-argument `settings_map_to_snapshot` function in `surfaces.rs` must be replaced with a two-phase approach:

  Phase 1 — decode all non-secret fields using `decode_prefixed_settings` into a private `SmtpNonSecretSnapshot` that derives `Deserialize` and
  covers `host`, `port`, `username`, `from_address`, `from_name`, `tls_mode`, `helo_host` (all `Option<T>`). **Parity requirement:** the existing
  `get_str` / `get_u16` helpers in `settings_map_to_snapshot` (a) filter empty strings to `None` and (b) accept `port` as either a JSON number or a
  JSON string. Plain `serde_json` deserialization does **not** preserve those semantics. Either provide custom `deserialize_with` functions on the
  affected fields or run a normalization pass on the raw `HashMap` before calling `decode_prefixed_settings`. The Step 1 parity test must drive
  both:

  ```rust
  #[derive(Debug, Default, serde::Deserialize)]
  struct SmtpNonSecretSnapshot {
      #[serde(default, deserialize_with = "deserialize_non_empty_string")]
      pub host: Option<String>,
      #[serde(default, deserialize_with = "deserialize_port_lenient")]
      pub port: Option<u16>,
      #[serde(default, deserialize_with = "deserialize_non_empty_string")]
      pub username: Option<String>,
      #[serde(default, deserialize_with = "deserialize_non_empty_string")]
      pub from_address: Option<String>,
      #[serde(default, deserialize_with = "deserialize_non_empty_string")]
      pub from_name: Option<String>,
      #[serde(default, deserialize_with = "deserialize_non_empty_string")]
      pub tls_mode: Option<String>,
      #[serde(default, deserialize_with = "deserialize_non_empty_string")]
      pub helo_host: Option<String>,
  }
  ```

  Phase 2 — extract and decrypt the `password` field using the existing `uptrakit_crypto::decrypt_str` path (currently in `settings_map_to_snapshot`
  at lines 403–420 in `surfaces.rs`). Keep that logic intact.

  Combine both phases into `SmtpSettingsSnapshot`:

  ```rust
  fn smtp_snapshot_from_raw(
      raw: &HashMap<String, serde_json::Value>,
      prefix: &str,
      password_key: &str,
      password_aad: &str,
      scope: &'static str,
      tenant_id: Option<uuid::Uuid>,
  ) -> SmtpSettingsSnapshot {
      let non_secret: SmtpNonSecretSnapshot = match
          uptrakit_shared_db::raw_settings::decode_prefixed_settings(prefix, raw)
      {
          Ok(snapshot) => snapshot,
          Err(error) => {
              tracing::warn!(
                  ?error, scope, prefix,
                  "smtp non-secret settings failed typed decode; falling back to defaults",
              );
              SmtpNonSecretSnapshot::default()
          }
      };
      let password = /* existing decryption logic for raw[password_key] */;
      SmtpSettingsSnapshot {
          host: non_secret.host,
          port: non_secret.port,
          username: non_secret.username,
          password,
          from_address: non_secret.from_address,
          from_name: non_secret.from_name,
          tls_mode: normalize_tls_mode(non_secret.tls_mode),
          helo_host: non_secret.helo_host,
      }
  }
  ```

  Update `db_load_tenant_smtp` and `db_load_global_smtp` to call `smtp_snapshot_from_raw` instead of `settings_map_to_snapshot`.

  Remove `settings_map_to_snapshot` and the `#[expect(clippy::too_many_arguments, ...)]` suppression on it.

- [ ] **Step 3: Evaluate `smtp_raw_to_json_map` in `notification_settings.rs` — confirm scope boundary**

  In `crates/ui/web-api-queries/src/notification_settings.rs`, `build_settings_bag` returns a JSON bag with the shape
  `{ "tenant": { "smtp.host": ..., "smtp.port": ..., ... }, "global": { "global_smtp.host": ..., ... } }` — preserving the **full key names
  including prefix**. The email plugin's `smtp_from_settings_map` (in `plugin.rs` line 100) reads from this bag using
  `format!("{prefix}{suffix}")` key lookups. This means the flat-key bag shape is a **wire contract** consumed by the plugin at runtime.

  Replacing `smtp_raw_to_json_map` with a typed snapshot and re-serializing with stripped keys would silently break SMTP delivery. This file is
  therefore **out of scope for Task 3**. The `notification_settings.rs` map-rebuild pattern is intentional given the plugin's `deliver()` contract.

  Document this explicitly in the module doc (see Task 4 Step 2): the flat `smtp.*` key names in the bag are consumed by `smtp_from_settings_map`
  and must be preserved.

  Keep the Telegram prefix pass-through unchanged.

- [ ] **Step 4: Run package checks**

  Run:

  ```bash
  cargo test -p uptrakit-notification-plugin-email
  cargo check -p uptrakit-notification-plugin-email
  cargo check -p uptrakit-web-api-queries
  cargo check -p uptrakit-web-api
  ```

  Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git commit --only crates/plugins/notifications/email/src/surfaces.rs -m "refactor(email): replace smtp positional getter chain with decode_prefixed_settings"
  ```

---

### Task 4: Add Boundary Documentation

**Context:** The spec acceptance criterion requires the codebase to record which boundaries remain intentionally dynamic and why in module/crate
documentation. An ADR is also warranted for the architectural decision to push untyped JSON to the outermost boundary only.

**Files:**

- Modify: `crates/shared/db/src/raw_settings.rs` — add module-level doc noting `decode_prefixed_settings` is the canonical pattern for typed
  settings snapshots and which crates should use it.
- Modify: `crates/plugins/notifications/email/src/surfaces.rs` — add module-level doc noting the `password` field is excluded from serde decode
  and why.
- Modify: `crates/ui/web-api-queries/src/notification_settings.rs` — add module-level doc noting the dynamic Telegram prefix is intentional and
  the SMTP portion is typed.
- Create: `docs/adr/0019-typed-dynamic-config-boundaries.md` — record the architectural decision.

- [ ] **Step 1: Add module doc to `raw_settings.rs`**

  Extend the existing module doc comment to include:

  > `decode_prefixed_settings` is the canonical way to convert a prefix-scoped settings map into a typed Rust snapshot. Plugin crates should use
  > this function rather than hand-rolling field-by-field getters. Fields that require runtime-decryption (e.g. `password` stored as `SecretString`)
  > must be extracted separately after the serde decode step.

- [ ] **Step 2: Add module doc to `surfaces.rs` and `notification_settings.rs`**

  Add a sentence each noting the intentional dynamic boundary for Telegram and the excluded `password` field.

- [ ] **Step 3: Write ADR**

  Create `docs/adr/0019-typed-dynamic-config-boundaries.md` documenting:
  - Context: raw `serde_json::Value` leaked too far inward in public DTOs and settings consumers.
  - Decision: typed wrappers at REST boundaries; serde-driven snapshots for settings; intentionally dynamic boundaries documented per site.
  - Consequences: stronger Rust invariants, eliminates duplicate getter families, `decode_prefixed_settings` is canonical.
  - **Explicit deferral:** `notification_settings.rs::build_settings_bag` still produces a flat `smtp.*`-keyed JSON bag that
    `smtp_from_settings_map` in the email plugin reads via `format!("{prefix}{suffix}")`. The spec acceptance criterion that names the dispatcher
    bag as a typed-snapshot target is therefore only partially satisfied in this phase: the email plugin's internal `settings_map_to_snapshot`
    getter chain is removed, but the cross-crate dispatcher → plugin bag stays flat. ADR-0019 must call this out as an intentional deferral and
    list it as a follow-up track ("typed dispatcher bag") so the gap is not silently lost.

  Run:

  ```bash
  markdownlint --config .markdownlint.json 'docs/adr/0019-typed-dynamic-config-boundaries.md'
  ```

- [ ] **Step 4: Commit**

  ```bash
  git commit --only docs/adr/0019-typed-dynamic-config-boundaries.md crates/shared/db/src/raw_settings.rs crates/plugins/notifications/email/src/surfaces.rs crates/ui/web-api-queries/src/notification_settings.rs -m "docs(adr): record typed dynamic config boundary decision (ADR-0019)"
  ```

---

### Task 5: Final Compatibility Verification

**Files:**

- No file modifications — verification only.

- [ ] **Step 1: Run full quality gate**

  Run:

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite
  cargo clippy --all-targets --all-features
  cargo test --all-features
  cargo deny check
  python3 ci/check_plugin_semantic_boundary.py
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: PASS.

- [ ] **Step 2: Confirm old patterns are gone from surfaces.rs**

  Run:

  ```bash
  rg -n "settings_map_to_snapshot\b" crates/plugins/notifications/email/src/surfaces.rs
  ```

  Expected: no matches.

- [ ] **Step 3: Confirm new patterns are present in surfaces.rs**

  Run:

  ```bash
  rg -n "decode_prefixed_settings" crates/plugins/notifications/email/src/surfaces.rs
  ```

  Expected: at least one match.

- [ ] **Step 4: Confirm `notification_settings.rs` bag shape is unchanged**

  Doc-only edits from Task 4 are allowed; runtime behaviour must be unchanged. Verify both the flat-key bag producers are still present:

  ```bash
  rg -n "smtp_raw_to_json_map|build_settings_bag" crates/ui/web-api-queries/src/notification_settings.rs
  ```

  Expected: both still present (intentionally preserved — flat-key bag is the plugin wire contract).

- [ ] **Step 5: Confirm shared parser is in use and `#[non_exhaustive]` is correctly scoped**

  Run:

  ```bash
  rg -n "json_object::parse_json_object|json_object::validate_json_object" crates/shared/web-api-types/src/
  rg -n -B1 "pub struct JsonObjectMap|pub struct JsonObjectInput" crates/shared/web-api-types/src/
  rg -n -B1 "pub enum JsonObjectMapPatch|pub enum IconUrlPatch" crates/shared/web-api-types/src/
  ```

  Expected: the helper appears at both call sites (`channels.rs` and `software_items.rs`); each `JsonObjectMap` and `JsonObjectInput` newtype
  struct has `#[non_exhaustive]` on the immediately preceding line; `JsonObjectMapPatch` and `IconUrlPatch` do **not** carry `#[non_exhaustive]`
  (closed sum types with external `Set(...)` construction sites).

---

## Self-Review

- Spec coverage: Task 1 shares the `JsonObjectMap` parsing helper across the two existing public newtypes and adds `#[non_exhaustive]` to every
  affected public newtype / patch enum, without collapsing them (so per-site `ValidationError.field` is preserved). Task 2 covers software item
  boundary verification. Task 3 covers SMTP/settings typed snapshots targeting `settings_map_to_snapshot` in `surfaces.rs` (the actual remaining
  code) with a behavioural parity test for empty-string, port-as-string, and `tls_mode` normalization semantics. Task 4 covers the spec's
  doc-recording requirement and ADR-0019, including an explicit deferral for the dispatcher bag. Task 5 covers wire-shape preservation
  verification.
- Placeholder scan: no unfinished-plan markers remain.
- Codebase-sync: plan targets verified against actual file state as of 2026-06-03. All symbol names match current code.
- Password handling: Task 3 explicitly retains the `uptrakit_crypto::decrypt_str` path for the `password` field, which cannot be decoded via
  plain serde from an encrypted-at-rest raw value.
- `notification_settings.rs` runtime behaviour is out of Task 3: its `smtp_raw_to_json_map` / `build_settings_bag` flat-key bag shape is the wire
  contract consumed by `smtp_from_settings_map` in the email plugin. Task 4 adds a module-level doc comment to that file; no behaviour changes.
- Spec acceptance gap (deliberate): the spec calls out the dispatcher path as a place that should stop hand-rebuilding typed state. This plan
  retires `settings_map_to_snapshot` inside the email plugin but leaves the cross-crate dispatcher → plugin bag flat-keyed. ADR-0019 (Task 4)
  records this as an intentional deferral and tracks it as a follow-up so the gap is visible.
- `dispatcher.rs` at `crates/ui/web-api/src/notifications/dispatcher.rs` is a one-line re-export (`pub use ...`); it is not a target for changes.
- `JsonObjectMap` error field name preservation: `channels.rs` currently emits `field: "config"`, `software_items.rs` emits
  `field: "config_override"`. Task 1 keeps the two newtypes distinct and routes their `TryFrom` impls through a private shared parser that takes the
  field name as a parameter, because serde's `#[serde(try_from)]` invokes `TryFrom::try_from(value)` with no way to thread the field name through.
  Collapsing into one shared `JsonObjectMap` type with a single `TryFrom` would silently change the error JSON on one of the call sites.
- Parity coverage: Task 3 Step 1 is a behavioural parity test that fixes empty-string-to-`None`, port-as-string acceptance, and `tls_mode`
  normalization before the refactor, so the move from `settings_map_to_snapshot` to `decode_prefixed_settings` cannot silently change snapshot
  contents for existing tenant settings.
