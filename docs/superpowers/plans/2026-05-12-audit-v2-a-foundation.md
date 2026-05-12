# Semantic Audit Logs V2 — Plan A: Foundation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the V2 schema, the new `uptrakit-audit-log-derive` proc-macro crate, the kind-parameterised `AuditEntry<K>` typestate, the
`AuditView` trait, the `emit_stateful` / `emit_event` / `AuditCommitHook` emitter APIs, and the workspace enablement of `serde_json/preserve_order`.
At the end of Plan A, V1 producers compile unchanged through a temporary shim; the breaking call-site cutover lands in Plan B.

**Architecture:** Add `AuditActionKind::{Stateful, Event}` to the registry and classify every V1 action. Replace flat `AuditEntry` with
`AuditEntry<K>` carrying `PhantomData<K>` through `.build()`. Wire snapshots through inline JSON columns on the V2 audit tables via a
drop-and-recreate migration. Add new emit paths: `emit_stateful(&tx, entry)` writes the audit row inside the supplied
`DatabaseTransaction`; `emit_event(entry)` keeps V1's fire-and-forget dispatcher. Journald multiplex for stateful events is deferred until
caller commit via an `AuditCommitHook` accumulator.

**Tech Stack:** Rust workspace, sea-orm (Postgres + SQLite), `syn` + `quote` + `proc-macro2` (new proc-macro crate), `parking_lot`, `tracing`,
`rootcause` + `thiserror`. Source of truth: spec `docs/superpowers/specs/2026-05-11-semantic-audit-logs-v2-design.md`. Standards snapshot:
`.superpowers/standards-snapshot.md`.

**Quality gates (run as final task):** `cargo fmt --all`, `cargo check --no-default-features --features db-sqlite`, `cargo check --all-features`,
`cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings`, `cargo clippy --all-targets --all-features -- -D warnings`,
`cargo test --all-features`, `cargo deny check`, `markdownlint --config .markdownlint.json '**/*.md'`.

---

## File structure

| File                                                               | Status | Responsibility                                                                                                                                                  |
| ------------------------------------------------------------------ | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Cargo.toml`                                                       | modify | Add `syn`/`quote`/`proc-macro2` to `[workspace.dependencies]`; switch `serde_json` to a table with `features = ["preserve_order"]`                              |
| `crates/shared/audit-log-derive/Cargo.toml`                        | create | New `proc-macro = true` crate manifest                                                                                                                          |
| `crates/shared/audit-log-derive/src/lib.rs`                        | create | `#[proc_macro_derive(AuditView, attributes(audit))]` + `audit_actions!`                                                                                         |
| `crates/shared/audit-log-derive/src/audit_view.rs`                 | create | Derive impl (struct attributes, field-level attributes, projection codegen)                                                                                     |
| `crates/shared/audit-log-derive/src/audit_actions.rs`              | create | Registry-side macro: per-action constructor methods on `AuditEntry`                                                                                             |
| `crates/shared/audit-log/Cargo.toml`                               | modify | Depend on `uptrakit-audit-log-derive`; re-export the derive                                                                                                     |
| `crates/shared/audit-log/src/action_type.rs`                       | modify | Add `AuditActionKind`; convert every `RegisteredAuditAction` constant to carry a kind; classify V1 actions                                                      |
| `crates/shared/audit-log/src/entry.rs`                             | modify | `AuditEntry<K>`, `AuditEntryBuilder<K>`, `Stateful`/`Event` zero-size markers, `HasBefore`/`HasAfter` intermediate markers, `AuditView` trait                   |
| `crates/shared/audit-log/src/lib.rs`                               | modify | Re-export new types + the derive                                                                                                                                |
| `crates/shared/audit-log/src/emitter.rs`                           | modify | `emit_stateful`, `emit_event`, `with_correlation`, `commit_hook`; keep `emit_best_effort` as deprecated `#[doc(hidden)]` shim that wraps an `AuditEntry<Event>` |
| `crates/shared/audit-log/src/commit_hook.rs`                       | create | `AuditCommitHook` accumulator + `flush_after_commit`                                                                                                            |
| `crates/shared/audit-log/src/backend.rs`                           | modify | `AuditLogBackend::write` becomes generic over the erased entry; new `write_in_tx` for stateful path                                                             |
| `crates/shared/db/src/migration/m20260512_000001_audit_logs_v2.rs` | create | Drop V1 audit tables; create V2 tables (action_kind, before_snapshot, after_snapshot, correlation_id) + CHECK constraints + indexes                             |
| `crates/shared/db/src/migration/mod.rs`                            | modify | Register the new migration                                                                                                                                      |
| `crates/shared/audit-log/tests/typestate_compile_fail/*.rs`        | create | `trybuild` compile-fail cases                                                                                                                                   |
| `crates/shared/audit-log/tests/typestate.rs`                       | create | `trybuild` runner                                                                                                                                               |

---

## Task 1: Branch and pre-flight

**Files:** none

- [ ] **Step 1: Cut a working branch from `main`**

  Run: `git checkout -b feat/audit-v2-foundation main`

- [ ] **Step 2: Confirm V1 schema migration filename**

  Run: `ls crates/shared/db/src/migration/ | grep audit`
  Expected output includes: `m20260417_000001_semantic_audit_logs.rs`

- [ ] **Step 3: Confirm the V1 emitter API surface**

  Run: `grep -n 'pub fn\|pub struct\|pub enum' crates/shared/audit-log/src/emitter.rs crates/shared/audit-log/src/entry.rs`
  Expected: lists `AuditEmitter::emit_best_effort`, `AuditEntry`, `AuditEntry::builder`, `AuditActorType`, `AuditOutcome`.

---

## Task 2: Workspace dependency wiring

**Files:**

- Modify: `Cargo.toml`

- [ ] **Step 1: Add proc-macro deps + `serde_json/preserve_order`**

  In `[workspace.dependencies]`, replace `serde_json = "1"` and append the new deps:

  ```toml
  serde_json = { version = "1", features = ["preserve_order"] }
  syn = { version = "2", features = ["full", "extra-traits"] }
  quote = "1"
  proc-macro2 = "1"
  ```

- [ ] **Step 2: Verify workspace still resolves**

  Run: `cargo check --workspace --no-default-features --features db-sqlite`
  Expected: no errors related to deps.

- [ ] **Step 3: Commit**

  ```bash
  git add Cargo.toml Cargo.lock
  git commit -m "build(audit-v2): enable serde_json/preserve_order; add syn/quote/proc-macro2"
  ```

---

## Task 3: Create the proc-macro crate skeleton

**Files:**

- Create: `crates/shared/audit-log-derive/Cargo.toml`
- Create: `crates/shared/audit-log-derive/src/lib.rs`
- Modify: `Cargo.toml` (workspace `members`)

- [ ] **Step 1: Add the crate to the workspace `members` list**

  In root `Cargo.toml` `[workspace] members = [...]`, append `"crates/shared/audit-log-derive"`.

- [ ] **Step 2: Write the manifest**

  Create `crates/shared/audit-log-derive/Cargo.toml`:

  ```toml
  [package]
  name = "uptrakit-audit-log-derive"
  description = "Proc-macros for uptrakit-audit-log (AuditView derive, audit_actions! registry macro)"
  edition.workspace = true
  license.workspace = true
  authors.workspace = true
  repository.workspace = true
  version = "0.0.1"

  [lib]
  proc-macro = true

  [dependencies]
  syn = { workspace = true }
  quote = { workspace = true }
  proc-macro2 = { workspace = true }

  [lints]
  workspace = true
  ```

- [ ] **Step 3: Write the lib stub**

  Create `crates/shared/audit-log-derive/src/lib.rs`:

  ```rust
  //! Proc-macros for the uptrakit semantic audit log subsystem.
  //!
  //! - [`AuditView`] derive — projects a domain entity into snapshot JSON.
  //! - [`audit_actions!`] — generates per-action constructor methods on `AuditEntry`.

  use proc_macro::TokenStream;

  mod audit_view;
  mod audit_actions;

  #[proc_macro_derive(AuditView, attributes(audit))]
  pub fn derive_audit_view(input: TokenStream) -> TokenStream {
      audit_view::expand(input)
  }

  #[proc_macro]
  pub fn audit_actions(input: TokenStream) -> TokenStream {
      audit_actions::expand(input)
  }
  ```

- [ ] **Step 4: Stub the two macro modules so the crate compiles**

  Create `crates/shared/audit-log-derive/src/audit_view.rs`:

  ```rust
  use proc_macro::TokenStream;

  pub fn expand(_input: TokenStream) -> TokenStream {
      // Implemented in Task 7.
      TokenStream::new()
  }
  ```

  Create `crates/shared/audit-log-derive/src/audit_actions.rs`:

  ```rust
  use proc_macro::TokenStream;

  pub fn expand(_input: TokenStream) -> TokenStream {
      // Implemented in Task 9.
      TokenStream::new()
  }
  ```

- [ ] **Step 5: Build the new crate**

  Run: `cargo check -p uptrakit-audit-log-derive`
  Expected: success.

- [ ] **Step 6: Commit**

  ```bash
  git add Cargo.toml crates/shared/audit-log-derive/
  git commit -m "feat(audit-v2): scaffold uptrakit-audit-log-derive proc-macro crate"
  ```

---

## Task 4: Add `AuditActionKind` to the audit-log crate

**Files:**

- Modify: `crates/shared/audit-log/src/action_type.rs`
- Test inline in the same file (existing convention)

- [ ] **Step 1: Write the failing test**

  Append to `crates/shared/audit-log/src/action_type.rs#[cfg(test)] mod tests`:

  ```rust
  #[test]
  fn audit_action_kind_as_str_round_trip() {
      assert_eq!(AuditActionKind::Stateful.as_str(), "stateful");
      assert_eq!(AuditActionKind::Event.as_str(), "event");
      assert_eq!(AuditActionKind::from_str("stateful"), Ok(AuditActionKind::Stateful));
      assert_eq!(AuditActionKind::from_str("event"), Ok(AuditActionKind::Event));
      assert!(AuditActionKind::from_str("other").is_err());
  }
  ```

- [ ] **Step 2: Run test (expected fail)**

  Run: `cargo test -p uptrakit-audit-log action_type::tests::audit_action_kind_as_str_round_trip`
  Expected: FAIL (unresolved type `AuditActionKind`).

- [ ] **Step 3: Add the enum**

  In `crates/shared/audit-log/src/action_type.rs`, above `RegisteredAuditAction`:

  ```rust
  use std::str::FromStr;

  /// Classifies a `RegisteredAuditAction` as either an entity-state mutation
  /// (snapshots required) or a discrete event (snapshots forbidden).
  ///
  /// Intentionally closed: adding a third kind is a deliberate contract change.
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub enum AuditActionKind {
      Stateful,
      Event,
  }

  impl AuditActionKind {
      #[must_use]
      pub const fn as_str(self) -> &'static str {
          match self {
              Self::Stateful => "stateful",
              Self::Event => "event",
          }
      }
  }

  impl std::fmt::Display for AuditActionKind {
      fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
          f.write_str(self.as_str())
      }
  }

  impl FromStr for AuditActionKind {
      type Err = ();
      fn from_str(value: &str) -> Result<Self, Self::Err> {
          match value {
              "stateful" => Ok(Self::Stateful),
              "event" => Ok(Self::Event),
              _ => Err(()),
          }
      }
  }
  ```

- [ ] **Step 4: Run test**

  Run: `cargo test -p uptrakit-audit-log action_type::tests::audit_action_kind_as_str_round_trip`
  Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/audit-log/src/action_type.rs
  git commit -m "feat(audit-v2): add AuditActionKind enum"
  ```

---

## Task 5: Extend `RegisteredAuditAction` to carry the kind

**Files:**

- Modify: `crates/shared/audit-log/src/action_type.rs`

- [ ] **Step 1: Write failing test**

  ```rust
  #[test]
  fn registered_action_carries_kind() {
      assert_eq!(AuditActionType::AUTH_LOGIN.kind(), AuditActionKind::Event);
      assert_eq!(AuditActionType::PLUGIN_CONFIG_UPDATE.kind(), AuditActionKind::Stateful);
  }
  ```

- [ ] **Step 2: Run test (expected fail)**

  Run: `cargo test -p uptrakit-audit-log action_type::tests::registered_action_carries_kind`
  Expected: FAIL.

- [ ] **Step 3: Extend the struct**

  Replace `pub struct RegisteredAuditAction(&'static str);` with:

  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
  pub struct RegisteredAuditAction {
      value: &'static str,
      kind: AuditActionKind,
  }

  impl RegisteredAuditAction {
      #[must_use]
      pub const fn new(value: &'static str, kind: AuditActionKind) -> Self {
          Self { value, kind }
      }

      #[must_use]
      pub const fn as_str(self) -> &'static str { self.value }

      #[must_use]
      pub const fn kind(self) -> AuditActionKind { self.kind }
  }
  ```

  Update `Display`, `PartialEq<…>` impls, and the `From<RegisteredAuditAction> for sea_orm::Value` impl to read `self.value` instead of
  `self.0`.

- [ ] **Step 4: Classify every V1 constant**

  For every `pub const NAME: RegisteredAuditAction = RegisteredAuditAction::new("…");` in this file, append the kind argument per the spec
  classification rules (spec §"Initial catalog classification" and §"Borderline call-outs"). Apply each in source order:

  Stateful actions (mutate one persisted entity row):

  ```text
  USER_CREATE, USER_UPDATE, USER_DELETE,
  OIDC_PROVIDER_CREATE, OIDC_PROVIDER_UPDATE, OIDC_PROVIDER_DELETE,
  PLUGIN_CONFIG_CREATE, PLUGIN_CONFIG_UPDATE, PLUGIN_CONFIG_DELETE,
  PLUGIN_TYPE_SETTINGS_UPSERT, PLUGIN_TYPE_SETTINGS_DELETE,
  INSTANCE_PLUGIN_TOGGLED, INSTANCE_PLUGIN_CONFIG_UPSERTED,
  NOTIFICATION_CHANNEL_CREATE, NOTIFICATION_CHANNEL_UPDATE, NOTIFICATION_CHANNEL_DELETE,
  NOTIFICATION_RULE_CREATE, NOTIFICATION_RULE_UPDATE, NOTIFICATION_RULE_DELETE,
  GLOBAL_SETTING_UPDATE, TENANT_SETTING_UPDATE,
  SCHEDULED_TASK_UPDATE,
  HOST_TAG_CREATE, HOST_TAG_UPDATE, HOST_TAG_DELETE,
  HOST_UPDATE, HOST_DEACTIVATE,
  SERVICE_UPDATE, SERVICE_APPROVE, SERVICE_REJECT, SERVICE_DEACTIVATE,
  SERVICE_UPDATE_FREEZE_ENABLE, SERVICE_UPDATE_FREEZE_DISABLE,
  SERVICE_CONFIG_STORE, SERVICE_CONFIG_DELETE,
  SOFTWARE_IGNORE_CREATE, SOFTWARE_IGNORE_DELETE,
  DISCOVERY_ALLOWLIST_CREATE, DISCOVERY_ALLOWLIST_DELETE,
  SOFTWARE_ITEM_CREATE, SOFTWARE_ITEM_UPDATE, SOFTWARE_ITEM_DELETE,
  SOFTWARE_ITEM_APPROVE,
  SOFTWARE_ITEM_ASSIGN_HOSTS, SOFTWARE_ITEM_UNASSIGN_HOST,
  SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT, SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT,
  API_TOKEN_CREATE, API_TOKEN_REVOKE,
  ENROLLMENT_TOKEN_CREATE, ENROLLMENT_TOKEN_REVOKE
  ```

  Event actions (workflow facts, point-in-time acts, pre-auth events, cross-entity operations):

  ```text
  AUTH_LOGIN, AUTH_LOGOUT, AUTH_API_TOKEN_AUTHENTICATE, AUTH_JWT_AUTHENTICATE,
  AUTH_SERVICE_AUTHENTICATE, AUTH_TOKEN_REFRESH,
  AUTH_DEVICE_START, AUTH_DEVICE_POLL, AUTH_DEVICE_APPROVE, AUTH_DEVICE_DENY,
  AUTH_OIDC_AUTHORIZE, AUTH_OIDC_CALLBACK, AUTH_OIDC_EXCHANGE, AUTH_OIDC_LINK,
  NOTIFICATION_CHANNEL_TEST, NOTIFICATION_RULE_TEST, NOTIFICATION_CALLBACK,
  TENANT_DATA_RESET,
  SYSTEM_CA_ROTATE, SYSTEM_SERVER_CERTIFICATE_RENEW,
  SCHEDULED_TASK_TRIGGER,
  HOST_TAG_ASSIGN, HOST_DISCOVER,
  SERVICE_MERGE,
  SERVICE_CERTIFICATE_ISSUE, SERVICE_CERTIFICATE_RENEW,
  SERVICE_ENROLLMENT_COMPLETED,
  SERVICE_CONFIG_DELIVER, SERVICE_CREDENTIALS_DELIVER,
  SERVICE_WORKLOAD_CLAIM, SERVICE_WORKLOAD_RELEASE,
  SURFACE_PROVIDER_REGISTER, SURFACE_ACTION_INVOKE,
  SOFTWARE_ITEM_MERGE, SOFTWARE_ITEM_BATCH, SOFTWARE_ITEM_ENRICH,
  SOFTWARE_VERSION_CHECK_TRIGGERED, SOFTWARE_VERSION_CHECK_COMPLETED,
  SOFTWARE_UPDATE_TRIGGERED, SOFTWARE_BATCH_UPDATE_TRIGGERED,
  SOFTWARE_UPDATE_STARTED, SOFTWARE_BATCH_UPDATE_STARTED,
  SOFTWARE_UPDATE_FINALIZED, SOFTWARE_BATCH_UPDATE_FINALIZED,
  SOFTWARE_UPDATE_STDIN_ATTENTION, SOFTWARE_UPDATE_INTERACTIVE_CONTROL,
  SYSTEM_SERVICE_UPDATE_GATE, SYSTEM_SERVICE_MACHINE_ID_VALIDATE,
  SYSTEM_SERVICE_UPDATE_FREEZE_APPLY, SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP
  ```

  Example one-line edit:

  ```rust
  // Before:
  pub const AUTH_LOGIN: RegisteredAuditAction = RegisteredAuditAction::new("auth.login");
  // After:
  pub const AUTH_LOGIN: RegisteredAuditAction =
      RegisteredAuditAction::new("auth.login", AuditActionKind::Event);
  ```

- [ ] **Step 5: Compile + run all action-type tests**

  Run: `cargo test -p uptrakit-audit-log action_type::tests`
  Expected: PASS.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/shared/audit-log/src/action_type.rs
  git commit -m "feat(audit-v2): classify every registered audit action as Stateful or Event"
  ```

---

## Task 6: Add the `AuditView` trait

**Files:**

- Modify: `crates/shared/audit-log/src/entry.rs`
- Modify: `crates/shared/audit-log/src/lib.rs`

- [ ] **Step 1: Write failing test**

  Append to `entry.rs` test module:

  ```rust
  #[test]
  fn audit_view_projects_struct_into_json() {
      struct Demo { id: Uuid, name: String, count: u32 }
      impl AuditView for Demo {
          const TARGET_TYPE: &'static str = "demo";
          fn audit_target_id(&self) -> String { self.id.to_string() }
          fn audit_target_display(&self) -> Option<String> { Some(self.name.clone()) }
          fn audit_view(&self) -> serde_json::Value {
              serde_json::json!({ "name": self.name, "count": self.count })
          }
      }

      let id = Uuid::now_v7();
      let v = Demo { id, name: "alpha".into(), count: 3 };
      assert_eq!(v.audit_target_id(), id.to_string());
      assert_eq!(v.audit_target_display(), Some("alpha".into()));
      let proj = v.audit_view();
      assert_eq!(proj["name"], "alpha");
      assert_eq!(proj["count"], 3);
  }
  ```

- [ ] **Step 2: Run test (expected fail)**

  Run: `cargo test -p uptrakit-audit-log entry::tests::audit_view_projects_struct_into_json`
  Expected: FAIL (`AuditView` undefined).

- [ ] **Step 3: Add the trait**

  Append to `entry.rs`:

  ```rust
  /// Deterministic, secret-safe projection of a persisted entity into snapshot JSON.
  ///
  /// Implementations are typically generated by the `#[derive(AuditView)]` proc-macro.
  pub trait AuditView {
      const TARGET_TYPE: &'static str;
      fn audit_target_id(&self) -> String;
      fn audit_target_display(&self) -> Option<String>;
      fn audit_view(&self) -> serde_json::Value;
  }
  ```

- [ ] **Step 4: Re-export from `lib.rs`**

  Add `pub use entry::AuditView;` next to the existing `AuditEntry` re-export.

- [ ] **Step 5: Run test**

  Run: `cargo test -p uptrakit-audit-log entry::tests::audit_view_projects_struct_into_json`
  Expected: PASS.

- [ ] **Step 6: Commit**

  ```bash
  git add crates/shared/audit-log/src/entry.rs crates/shared/audit-log/src/lib.rs
  git commit -m "feat(audit-v2): introduce AuditView trait"
  ```

---

## Task 7: Implement the `AuditView` derive macro

**Files:**

- Modify: `crates/shared/audit-log-derive/src/audit_view.rs`
- Create: `crates/shared/audit-log-derive/tests/audit_view_derive.rs`
- Modify: `crates/shared/audit-log/Cargo.toml` (depend on derive crate, add dev-dep `trybuild`)
- Modify: `crates/shared/audit-log/src/lib.rs` (re-export the derive)

- [ ] **Step 1: Add the derive crate as a dependency**

  In `crates/shared/audit-log/Cargo.toml` `[dependencies]`:

  ```toml
  uptrakit-audit-log-derive = { path = "../audit-log-derive" }
  ```

  In `[dev-dependencies]`:

  ```toml
  trybuild = "1"
  ```

- [ ] **Step 2: Re-export the derive**

  In `crates/shared/audit-log/src/lib.rs`:

  ```rust
  pub use uptrakit_audit_log_derive::AuditView;
  ```

- [ ] **Step 3: Write failing test for derive**

  Create `crates/shared/audit-log-derive/tests/audit_view_derive.rs`:

  ```rust
  use uptrakit_audit_log::AuditView;

  #[derive(uptrakit_audit_log::AuditView)]
  #[audit(target_type = "demo")]
  struct Demo {
      id: uuid::Uuid,
      name: String,
      count: u32,
      #[audit(skip)]
      internal: i64,
      // auto-skipped by name allowlist:
      created_at: time::OffsetDateTime,
      updated_at: time::OffsetDateTime,
  }

  #[test]
  fn derive_projects_only_audit_relevant_fields() {
      let id = uuid::Uuid::now_v7();
      let now = time::OffsetDateTime::now_utc();
      let demo = Demo { id, name: "alpha".into(), count: 3, internal: 99, created_at: now, updated_at: now };

      assert_eq!(<Demo as AuditView>::TARGET_TYPE, "demo");
      assert_eq!(demo.audit_target_id(), id.to_string());
      assert_eq!(demo.audit_target_display(), Some("alpha".into()));
      let v = demo.audit_view();
      let map = v.as_object().expect("object");
      assert!(map.contains_key("name"));
      assert!(map.contains_key("count"));
      assert!(!map.contains_key("internal"));
      assert!(!map.contains_key("created_at"));
      assert!(!map.contains_key("updated_at"));
      assert!(!map.contains_key("id"));
  }
  ```

  Workspace dev-deps must include `time` and `uuid`. Confirm via `grep '^uuid\|^time' Cargo.toml` — both already present.

- [ ] **Step 4: Run test (expected fail)**

  Run: `cargo test -p uptrakit-audit-log-derive --tests`
  Expected: FAIL (derive returns empty TokenStream).

- [ ] **Step 5: Implement the derive**

  Replace `crates/shared/audit-log-derive/src/audit_view.rs` with the full expansion. Key rules:
  - Required struct attribute: `#[audit(target_type = "<literal>")]`.
  - Field-level attribute `#[audit(skip)]`: exclude entirely.
  - Field-level attribute `#[audit(include)]`: override the auto-skip allowlist.
  - Field-level attribute `#[audit(project_with = "<fn>")]`: emit `serde_json::to_value(<fn>(&self.<field>))` instead of the field's default
    `Serialize`.
  - Optional struct attribute `#[audit(id_field = "<ident>")]`: default `id`.
  - Optional struct attribute `#[audit(display_field = "<ident>")]`: default `name` if a `name: String` field exists else `None`.
  - Auto-skip name allowlist (closed): `created_at`, `updated_at`, `deleted_at`, `deactivated_at`. Skipped unless `#[audit(include)]` is on
    the field.
  - Field projection iterates declaration order and emits a `serde_json::Map` (relies on workspace-enabled `preserve_order` feature).
  - Generated impl block goes to the same module the input struct is in. No `where` clause beyond what derive sees.

  ```rust
  use proc_macro::TokenStream;
  use proc_macro2::TokenStream as TokenStream2;
  use quote::{format_ident, quote};
  use syn::{parse_macro_input, Attribute, DeriveInput, Field, LitStr, Meta, Type};

  const AUTO_SKIP_FIELDS: &[&str] = &["created_at", "updated_at", "deleted_at", "deactivated_at"];

  pub fn expand(input: TokenStream) -> TokenStream {
      let input = parse_macro_input!(input as DeriveInput);
      match expand_inner(&input) {
          Ok(ts) => ts.into(),
          Err(e) => e.to_compile_error().into(),
      }
  }

  fn expand_inner(input: &DeriveInput) -> syn::Result<TokenStream2> {
      let struct_attrs = parse_struct_attrs(&input.attrs)?;
      let target_type = struct_attrs.target_type
          .ok_or_else(|| syn::Error::new_spanned(input, "missing #[audit(target_type = \"...\")]"))?;
      let id_field = struct_attrs.id_field.unwrap_or_else(|| format_ident!("id"));
      let display_field = struct_attrs.display_field;

      let fields = match &input.data {
          syn::Data::Struct(s) => match &s.fields {
              syn::Fields::Named(named) => named.named.iter().collect::<Vec<_>>(),
              _ => return Err(syn::Error::new_spanned(input, "AuditView requires a named-field struct")),
          },
          _ => return Err(syn::Error::new_spanned(input, "AuditView requires a struct")),
      };

      let projections = fields.iter()
          .filter_map(|f| field_projection(f).transpose())
          .collect::<syn::Result<Vec<_>>>()?;

      let display_expr = match &display_field {
          Some(name) => quote!(Some(::std::string::ToString::to_string(&self.#name))),
          None if fields.iter().any(|f| f.ident.as_ref().is_some_and(|i| i == "name")) =>
              quote!(Some(::std::string::ToString::to_string(&self.name))),
          None => quote!(None),
      };

      let name = &input.ident;
      let target_type_lit = LitStr::new(&target_type, proc_macro2::Span::call_site());

      Ok(quote! {
          impl ::uptrakit_audit_log::AuditView for #name {
              const TARGET_TYPE: &'static str = #target_type_lit;
              fn audit_target_id(&self) -> ::std::string::String {
                  ::std::string::ToString::to_string(&self.#id_field)
              }
              fn audit_target_display(&self) -> ::std::option::Option<::std::string::String> {
                  #display_expr
              }
              fn audit_view(&self) -> ::serde_json::Value {
                  let mut map = ::serde_json::Map::new();
                  #(#projections)*
                  ::serde_json::Value::Object(map)
              }
          }
      })
  }

  struct StructAttrs {
      target_type: Option<String>,
      id_field: Option<syn::Ident>,
      display_field: Option<syn::Ident>,
  }

  fn parse_struct_attrs(attrs: &[Attribute]) -> syn::Result<StructAttrs> {
      let mut out = StructAttrs { target_type: None, id_field: None, display_field: None };
      for attr in attrs.iter().filter(|a| a.path().is_ident("audit")) {
          attr.parse_nested_meta(|meta| {
              if meta.path.is_ident("target_type") {
                  out.target_type = Some(meta.value()?.parse::<LitStr>()?.value());
              } else if meta.path.is_ident("id_field") {
                  out.id_field = Some(format_ident!("{}", meta.value()?.parse::<LitStr>()?.value()));
              } else if meta.path.is_ident("display_field") {
                  out.display_field = Some(format_ident!("{}", meta.value()?.parse::<LitStr>()?.value()));
              } else {
                  return Err(meta.error("unknown audit attribute"));
              }
              Ok(())
          })?;
      }
      Ok(out)
  }

  enum FieldAction { Skip, Default, ProjectWith(syn::Ident) }

  fn field_projection(f: &Field) -> syn::Result<Option<TokenStream2>> {
      let ident = f.ident.as_ref().expect("named field");
      let name_str = ident.to_string();
      let mut force_include = false;
      let mut action = FieldAction::Default;

      for attr in f.attrs.iter().filter(|a| a.path().is_ident("audit")) {
          attr.parse_nested_meta(|meta| {
              if meta.path.is_ident("skip") {
                  action = FieldAction::Skip;
              } else if meta.path.is_ident("include") {
                  force_include = true;
              } else if meta.path.is_ident("project_with") {
                  let s: LitStr = meta.value()?.parse()?;
                  action = FieldAction::ProjectWith(format_ident!("{}", s.value()));
              } else {
                  return Err(meta.error("unknown audit attribute"));
              }
              Ok(())
          })?;
      }

      if matches!(action, FieldAction::Skip) { return Ok(None); }
      if AUTO_SKIP_FIELDS.contains(&name_str.as_str()) && !force_include { return Ok(None); }
      if name_str == "id" { return Ok(None); }

      let key_lit = LitStr::new(&name_str, proc_macro2::Span::call_site());
      Ok(Some(match action {
          FieldAction::ProjectWith(func) => quote! {
              map.insert(#key_lit.into(), #func(&self.#ident));
          },
          FieldAction::Default => quote! {
              map.insert(#key_lit.into(), ::serde_json::to_value(&self.#ident).unwrap_or(::serde_json::Value::Null));
          },
          FieldAction::Skip => unreachable!(),
      }))
  }
  ```

  > Note: the `serde_json::to_value(...).unwrap_or(Null)` pattern is the only place this crate touches `unwrap_or` and is **not** an
  > `unwrap()` per snapshot rule "no unwrap in production." `serde_json::to_value` only errors on non-self-describing types, which our
  > `Serialize`-bound fields never are; the `Null` fallback preserves total-function semantics for the projection.

- [ ] **Step 6: Run test**

  Run: `cargo test -p uptrakit-audit-log-derive --tests`
  Expected: PASS.

- [ ] **Step 7: Add a determinism test**

  Append to `tests/audit_view_derive.rs`:

  ```rust
  #[test]
  fn derive_projection_is_byte_equal_across_invocations() {
      let id = uuid::Uuid::now_v7();
      let now = time::OffsetDateTime::now_utc();
      let demo = Demo { id, name: "alpha".into(), count: 3, internal: 99, created_at: now, updated_at: now };
      let first = serde_json::to_vec(&demo.audit_view()).expect("serialize");
      let second = serde_json::to_vec(&demo.audit_view()).expect("serialize");
      assert_eq!(first, second);
  }
  ```

  Run: `cargo test -p uptrakit-audit-log-derive --tests`. Expected: PASS.

- [ ] **Step 8: Commit**

  ```bash
  git add crates/shared/audit-log-derive/ crates/shared/audit-log/Cargo.toml crates/shared/audit-log/src/lib.rs
  git commit -m "feat(audit-v2): implement AuditView derive macro"
  ```

---

## Task 8: Replace `AuditEntry` with `AuditEntry<K>` + typestate builder

**Files:**

- Modify: `crates/shared/audit-log/src/entry.rs`
- Modify: `crates/shared/audit-log/src/lib.rs`

This is a sizable single-file rewrite. The shape is:

- `pub struct Stateful;` and `pub struct Event;` zero-size marker types.
- `pub struct HasBefore;` `pub struct HasAfter;` `pub struct NeedsBefore;` `pub struct NeedsAfter;` intermediate markers for the builder.
- `pub struct AuditEntry<K> { fields…, _kind: PhantomData<K> }`.
- `pub struct AuditEntryBuilder<K, B = NeedsBefore, A = NeedsAfter> { … }`.
- `Builder<Event>::build()` produces `AuditEntry<Event>` without snapshot fields.
- `Builder<Stateful, HasBefore, HasAfter>::build()` produces `AuditEntry<Stateful>` with `before` + `after` populated; no other state
  combination has `.build()`.
- `.before(&impl AuditView)` is implemented only on `Builder<Stateful, NeedsBefore, A>` and returns `Builder<Stateful, HasBefore, A>`.
  Same for `.after`.

- [ ] **Step 1: Write the new types**

  Replace `pub struct AuditEntry { … }` (and its V1 builder) with the V2 shape. Keep `AuditActorType` and `AuditOutcome` unchanged.

  ```rust
  use std::marker::PhantomData;
  use serde_json::Value;
  use time::OffsetDateTime;
  use uuid::Uuid;

  use crate::action_type::AuditActionType;
  use crate::error::{AuditLogError, Result};

  pub struct Stateful;
  pub struct Event;

  pub struct NeedsBefore; pub struct HasBefore;
  pub struct NeedsAfter;  pub struct HasAfter;

  #[derive(Clone, Debug)]
  pub struct AuditEntry<K> {
      pub id: Uuid,
      pub tenant_id: Option<Uuid>,
      pub occurred_at: OffsetDateTime,
      pub actor_type: AuditActorType,
      pub actor_id: Option<Uuid>,
      pub actor_display: Option<String>,
      pub action_type: AuditActionType,
      pub target_type: Option<String>,
      pub target_id: Option<String>,
      pub target_display: Option<String>,
      pub outcome: AuditOutcome,
      pub details_json: Option<Value>,
      pub before_snapshot: Option<Value>,
      pub after_snapshot: Option<Value>,
      pub correlation_id: Option<Uuid>,
      pub request_id: Option<String>,
      _kind: PhantomData<K>,
  }

  pub struct AuditEntryBuilder<K, B = NeedsBefore, A = NeedsAfter> {
      entry: AuditEntry<K>,
      _state: PhantomData<(B, A)>,
  }
  ```

- [ ] **Step 2: Implement `AuditEntry::builder_event` and `builder_stateful` constructors**

  These are the only two entry points. The macro-generated per-action constructors call them.

  ```rust
  impl AuditEntry<Event> {
      /// Construct an Event-class builder for the supplied action type.
      pub fn builder_event(action: AuditActionType) -> AuditEntryBuilder<Event> {
          AuditEntryBuilder {
              entry: empty_entry(action),
              _state: PhantomData,
          }
      }
  }

  impl AuditEntry<Stateful> {
      /// Construct a Stateful-class builder. Callers must subsequently set `.before()` and `.after()`.
      pub fn builder_stateful(action: AuditActionType) -> AuditEntryBuilder<Stateful, NeedsBefore, NeedsAfter> {
          AuditEntryBuilder { entry: empty_entry(action), _state: PhantomData }
      }
  }

  fn empty_entry<K>(action: AuditActionType) -> AuditEntry<K> {
      AuditEntry {
          id: Uuid::now_v7(),
          tenant_id: None,
          occurred_at: OffsetDateTime::now_utc(),
          actor_type: AuditActorType::System,
          actor_id: None,
          actor_display: None,
          action_type: action,
          target_type: None,
          target_id: None,
          target_display: None,
          outcome: AuditOutcome::Success,
          details_json: None,
          before_snapshot: None,
          after_snapshot: None,
          correlation_id: None,
          request_id: None,
          _kind: PhantomData,
      }
  }
  ```

- [ ] **Step 3: Implement common builder methods on `AuditEntryBuilder<K, B, A>` (parameterised over all states)**

  ```rust
  impl<K, B, A> AuditEntryBuilder<K, B, A> {
      pub fn tenant_scope(mut self, tenant_id: Uuid) -> Self { self.entry.tenant_id = Some(tenant_id); self }
      pub fn system_scope(mut self) -> Self { self.entry.tenant_id = None; self }
      pub fn actor(mut self, actor_type: AuditActorType, actor_id: Option<Uuid>) -> Self {
          self.entry.actor_type = actor_type; self.entry.actor_id = actor_id; self
      }
      pub fn actor_user(self, actor_id: Uuid, display: impl Into<String>) -> Self {
          self.actor(AuditActorType::User, Some(actor_id)).actor_display_opt(Some(display.into()))
      }
      pub fn actor_service(self, actor_id: Uuid) -> Self { self.actor(AuditActorType::Service, Some(actor_id)) }
      pub fn actor_system(self) -> Self { self.actor(AuditActorType::System, None) }
      pub fn actor_display_opt(mut self, display: Option<String>) -> Self { self.entry.actor_display = display; self }
      pub fn outcome(mut self, outcome: AuditOutcome) -> Self { self.entry.outcome = outcome; self }
      pub fn details(mut self, details: Value) -> Self { self.entry.details_json = Some(details); self }
      pub fn request_id_opt(mut self, req: Option<String>) -> Self { self.entry.request_id = req; self }
      pub fn correlation_id(mut self, id: Uuid) -> Self { self.entry.correlation_id = Some(id); self }
      pub fn correlation_id_opt(mut self, id: Option<Uuid>) -> Self { self.entry.correlation_id = id; self }
      pub fn target(mut self, target_type: &str, target_id: String, display: Option<String>) -> Self {
          self.entry.target_type = Some(target_type.to_string());
          self.entry.target_id = Some(target_id);
          self.entry.target_display = display;
          self
      }
  }
  ```

- [ ] **Step 4: Implement `.before()` and `.after()` — Stateful only, state-transition methods**

  ```rust
  impl<A> AuditEntryBuilder<Stateful, NeedsBefore, A> {
      pub fn before<V: AuditView>(mut self, view: &V) -> AuditEntryBuilder<Stateful, HasBefore, A> {
          self.entry.before_snapshot = Some(view.audit_view());
          self.entry.target_type = Some(V::TARGET_TYPE.to_string());
          self.entry.target_id = Some(view.audit_target_id());
          if self.entry.target_display.is_none() { self.entry.target_display = view.audit_target_display(); }
          AuditEntryBuilder { entry: self.entry, _state: PhantomData }
      }
  }

  impl<B> AuditEntryBuilder<Stateful, B, NeedsAfter> {
      pub fn after<V: AuditView>(mut self, view: &V) -> AuditEntryBuilder<Stateful, B, HasAfter> {
          self.entry.after_snapshot = Some(view.audit_view());
          AuditEntryBuilder { entry: self.entry, _state: PhantomData }
      }
  }
  ```

- [ ] **Step 5: Implement `.build()` only on the terminal builder states**

  ```rust
  impl AuditEntryBuilder<Event> {
      pub fn build(self) -> Result<AuditEntry<Event>> {
          validate(&self.entry)?;
          Ok(self.entry)
      }
  }

  impl AuditEntryBuilder<Stateful, HasBefore, HasAfter> {
      pub fn build(self) -> Result<AuditEntry<Stateful>> {
          validate(&self.entry)?;
          Ok(self.entry)
      }
  }

  fn validate<K>(e: &AuditEntry<K>) -> Result<()> { /* identical to V1 validate; plus enforce that
      Stateful entries have both snapshots and total snapshot bytes per-column <= 16 KB; Event entries
      have both snapshots = None */
      const MAX_SNAPSHOT_BYTES: usize = 16 * 1024;
      const MAX_DETAILS_BYTES: usize = 4 * 1024;
      // existing V1 checks (UTC offset, action_type length, target_id requires target_type, actor_display
      // length, target_type length, target_display length, target_id length, request_id length,
      // details_json size, system actor must not have actor_id) preserved verbatim from V1 entry.rs
      // …
      if let Some(s) = &e.before_snapshot {
          let bytes = serde_json::to_vec(s).map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
          if bytes.len() > MAX_SNAPSHOT_BYTES {
              return Err(rootcause::report!(AuditLogError::Validation("before_snapshot exceeds 16 KB".into())));
          }
      }
      if let Some(s) = &e.after_snapshot {
          let bytes = serde_json::to_vec(s).map_err(|err| rootcause::report!(AuditLogError::Serialization(err)))?;
          if bytes.len() > MAX_SNAPSHOT_BYTES {
              return Err(rootcause::report!(AuditLogError::Validation("after_snapshot exceeds 16 KB".into())));
          }
      }
      Ok(())
  }
  ```

- [ ] **Step 6: Migrate V1 in-file unit tests to the new API**

  Update each existing test in `entry.rs#[cfg(test)] mod tests` that called `AuditEntry::builder(...)` or `AuditEntry::test_stub(...)`. Replace
  with `AuditEntry::builder_event(...)` / `builder_stateful(...)` where applicable. Remove `test_stub` until Task 11.

- [ ] **Step 7: Re-export new types from `lib.rs`**

  ```rust
  pub use entry::{AuditEntry, AuditEntryBuilder, AuditView, Event, HasAfter, HasBefore, NeedsAfter, NeedsBefore, Stateful};
  ```

- [ ] **Step 8: Build + test**

  Run: `cargo test -p uptrakit-audit-log`
  Expected: PASS.

- [ ] **Step 9: Commit**

  ```bash
  git add crates/shared/audit-log/src/entry.rs crates/shared/audit-log/src/lib.rs
  git commit -m "feat(audit-v2): typestate AuditEntry<K> + builder with HasBefore/HasAfter markers"
  ```

---

## Task 9: Implement the `audit_actions!` registry macro

**Files:**

- Modify: `crates/shared/audit-log-derive/src/audit_actions.rs`
- Modify: `crates/shared/audit-log/src/action_type.rs` (invoke the macro at the end of file)

The macro takes a flat list of `(ident, "literal", kind, optional target type binding)` and emits per-action constructor methods. For
Stateful actions it generates a method that takes `(&impl AuditView, &impl AuditView)` and stamps target metadata. For Event actions it
generates a no-argument method.

- [ ] **Step 1: Write failing test (in audit-log crate)**

  Append to `entry.rs` tests:

  ```rust
  #[test]
  fn audit_actions_macro_generates_event_constructor() {
      // Tests against the macro-generated constructor wired in action_type.rs.
      let b = AuditEntry::auth_login();
      // builder<Event>; ensure build() compiles and produces an event entry.
      let entry = b.actor_system().build().expect("event builds");
      assert_eq!(entry.action_type.as_str(), "auth.login");
      assert!(entry.before_snapshot.is_none());
  }
  ```

- [ ] **Step 2: Implement the macro**

  In `crates/shared/audit-log-derive/src/audit_actions.rs`:

  ```rust
  use proc_macro::TokenStream;
  use proc_macro2::TokenStream as TokenStream2;
  use quote::{format_ident, quote};
  use syn::{parse::{Parse, ParseStream}, parse_macro_input, Ident, LitStr, Token};

  struct ActionList(Vec<Action>);
  struct Action { method: Ident, action_const: Ident, kind: Ident }

  impl Parse for ActionList {
      fn parse(input: ParseStream) -> syn::Result<Self> {
          let mut out = Vec::new();
          while !input.is_empty() {
              let method: Ident = input.parse()?;
              input.parse::<Token![=>]>()?;
              let action_const: Ident = input.parse()?;
              input.parse::<Token![,]>()?;
              let kind: Ident = input.parse()?;
              if !input.is_empty() { input.parse::<Token![;]>()?; }
              out.push(Action { method, action_const, kind });
          }
          Ok(ActionList(out))
      }
  }

  pub fn expand(input: TokenStream) -> TokenStream {
      let list = parse_macro_input!(input as ActionList);
      let mut items = Vec::new();
      for a in list.0 {
          let m = &a.method;
          let c = &a.action_const;
          let item = match a.kind.to_string().as_str() {
              "Event" => quote! {
                  impl crate::entry::AuditEntry<crate::entry::Event> {
                      pub fn #m() -> crate::entry::AuditEntryBuilder<crate::entry::Event> {
                          Self::builder_event(crate::action_type::AuditActionType::from(crate::action_type::AuditActionType::#c))
                      }
                  }
              },
              "Stateful" => quote! {
                  impl crate::entry::AuditEntry<crate::entry::Stateful> {
                      pub fn #m<V: crate::entry::AuditView>(
                          before: &V,
                          after: &V,
                      ) -> crate::entry::AuditEntryBuilder<crate::entry::Stateful, crate::entry::HasBefore, crate::entry::HasAfter> {
                          Self::builder_stateful(crate::action_type::AuditActionType::from(crate::action_type::AuditActionType::#c))
                              .before(before)
                              .after(after)
                      }
                  }
              },
              other => panic!("audit_actions!: unknown kind `{other}`"),
          };
          items.push(item);
      }
      let out: TokenStream2 = quote! { #(#items)* };
      out.into()
  }
  ```

- [ ] **Step 3: Invoke the macro at the bottom of `action_type.rs`**

  Append:

  ```rust
  uptrakit_audit_log_derive::audit_actions! {
      // auth — Event
      auth_login => AUTH_LOGIN, Event;
      auth_logout => AUTH_LOGOUT, Event;
      auth_api_token_authenticate => AUTH_API_TOKEN_AUTHENTICATE, Event;
      auth_jwt_authenticate => AUTH_JWT_AUTHENTICATE, Event;
      auth_service_authenticate => AUTH_SERVICE_AUTHENTICATE, Event;
      auth_token_refresh => AUTH_TOKEN_REFRESH, Event;
      auth_device_start => AUTH_DEVICE_START, Event;
      auth_device_poll => AUTH_DEVICE_POLL, Event;
      auth_device_approve => AUTH_DEVICE_APPROVE, Event;
      auth_device_deny => AUTH_DEVICE_DENY, Event;
      auth_oidc_authorize => AUTH_OIDC_AUTHORIZE, Event;
      auth_oidc_callback => AUTH_OIDC_CALLBACK, Event;
      auth_oidc_exchange => AUTH_OIDC_EXCHANGE, Event;
      auth_oidc_link => AUTH_OIDC_LINK, Event;

      // tokens — Stateful
      api_token_create => API_TOKEN_CREATE, Stateful;
      api_token_revoke => API_TOKEN_REVOKE, Stateful;
      enrollment_token_create => ENROLLMENT_TOKEN_CREATE, Stateful;
      enrollment_token_revoke => ENROLLMENT_TOKEN_REVOKE, Stateful;

      // users — Stateful
      user_create => USER_CREATE, Stateful;
      user_update => USER_UPDATE, Stateful;
      user_delete => USER_DELETE, Stateful;

      // oidc providers — Stateful
      oidc_provider_create => OIDC_PROVIDER_CREATE, Stateful;
      oidc_provider_update => OIDC_PROVIDER_UPDATE, Stateful;
      oidc_provider_delete => OIDC_PROVIDER_DELETE, Stateful;

      // plugin config — Stateful
      plugin_config_create => PLUGIN_CONFIG_CREATE, Stateful;
      plugin_config_update => PLUGIN_CONFIG_UPDATE, Stateful;
      plugin_config_delete => PLUGIN_CONFIG_DELETE, Stateful;
      plugin_type_settings_upsert => PLUGIN_TYPE_SETTINGS_UPSERT, Stateful;
      plugin_type_settings_delete => PLUGIN_TYPE_SETTINGS_DELETE, Stateful;
      instance_plugin_toggled => INSTANCE_PLUGIN_TOGGLED, Stateful;
      instance_plugin_config_upserted => INSTANCE_PLUGIN_CONFIG_UPSERTED, Stateful;

      // notifications — channel/rule mutations Stateful; test/callback Event
      notification_channel_create => NOTIFICATION_CHANNEL_CREATE, Stateful;
      notification_channel_update => NOTIFICATION_CHANNEL_UPDATE, Stateful;
      notification_channel_delete => NOTIFICATION_CHANNEL_DELETE, Stateful;
      notification_channel_test => NOTIFICATION_CHANNEL_TEST, Event;
      notification_rule_create => NOTIFICATION_RULE_CREATE, Stateful;
      notification_rule_update => NOTIFICATION_RULE_UPDATE, Stateful;
      notification_rule_delete => NOTIFICATION_RULE_DELETE, Stateful;
      notification_rule_test => NOTIFICATION_RULE_TEST, Event;
      notification_callback => NOTIFICATION_CALLBACK, Event;

      // settings — Stateful; tenant.data.reset Event
      global_setting_update => GLOBAL_SETTING_UPDATE, Stateful;
      tenant_setting_update => TENANT_SETTING_UPDATE, Stateful;
      tenant_data_reset => TENANT_DATA_RESET, Event;

      // CA + server certificate — Event (point-in-time rotation acts)
      system_ca_rotate => SYSTEM_CA_ROTATE, Event;
      system_server_certificate_renew => SYSTEM_SERVER_CERTIFICATE_RENEW, Event;

      // scheduled tasks — update Stateful; trigger Event
      scheduled_task_update => SCHEDULED_TASK_UPDATE, Stateful;
      scheduled_task_trigger => SCHEDULED_TASK_TRIGGER, Event;

      // hosts + tags
      host_tag_create => HOST_TAG_CREATE, Stateful;
      host_tag_update => HOST_TAG_UPDATE, Stateful;
      host_tag_delete => HOST_TAG_DELETE, Stateful;
      host_tag_assign => HOST_TAG_ASSIGN, Event;
      host_update => HOST_UPDATE, Stateful;
      host_deactivate => HOST_DEACTIVATE, Stateful;
      host_discover => HOST_DISCOVER, Event;

      // services
      service_update => SERVICE_UPDATE, Stateful;
      service_approve => SERVICE_APPROVE, Stateful;
      service_reject => SERVICE_REJECT, Stateful;
      service_merge => SERVICE_MERGE, Event;
      service_update_freeze_enable => SERVICE_UPDATE_FREEZE_ENABLE, Stateful;
      service_update_freeze_disable => SERVICE_UPDATE_FREEZE_DISABLE, Stateful;
      service_deactivate => SERVICE_DEACTIVATE, Stateful;
      service_config_store => SERVICE_CONFIG_STORE, Stateful;
      service_config_delete => SERVICE_CONFIG_DELETE, Stateful;
      service_config_deliver => SERVICE_CONFIG_DELIVER, Event;
      service_certificate_issue => SERVICE_CERTIFICATE_ISSUE, Event;
      service_certificate_renew => SERVICE_CERTIFICATE_RENEW, Event;
      service_enrollment_completed => SERVICE_ENROLLMENT_COMPLETED, Event;
      service_credentials_deliver => SERVICE_CREDENTIALS_DELIVER, Event;
      service_workload_claim => SERVICE_WORKLOAD_CLAIM, Event;
      service_workload_release => SERVICE_WORKLOAD_RELEASE, Event;

      // surfaces — Event
      surface_provider_register => SURFACE_PROVIDER_REGISTER, Event;
      surface_action_invoke => SURFACE_ACTION_INVOKE, Event;

      // software item registry
      software_item_create => SOFTWARE_ITEM_CREATE, Stateful;
      software_item_update => SOFTWARE_ITEM_UPDATE, Stateful;
      software_item_delete => SOFTWARE_ITEM_DELETE, Stateful;
      software_item_approve => SOFTWARE_ITEM_APPROVE, Stateful;
      software_item_assign_hosts => SOFTWARE_ITEM_ASSIGN_HOSTS, Stateful;
      software_item_unassign_host => SOFTWARE_ITEM_UNASSIGN_HOST, Stateful;
      software_item_update_host_assignment => SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT, Stateful;
      software_item_delete_plugin_assignment => SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT, Stateful;
      software_item_merge => SOFTWARE_ITEM_MERGE, Event;
      software_item_batch => SOFTWARE_ITEM_BATCH, Event;
      software_item_enrich => SOFTWARE_ITEM_ENRICH, Event;

      software_ignore_create => SOFTWARE_IGNORE_CREATE, Stateful;
      software_ignore_delete => SOFTWARE_IGNORE_DELETE, Stateful;
      discovery_allowlist_create => DISCOVERY_ALLOWLIST_CREATE, Stateful;
      discovery_allowlist_delete => DISCOVERY_ALLOWLIST_DELETE, Stateful;

      // software workflow — all Event (correlation_id ties them)
      software_version_check_triggered => SOFTWARE_VERSION_CHECK_TRIGGERED, Event;
      software_version_check_completed => SOFTWARE_VERSION_CHECK_COMPLETED, Event;
      software_update_triggered => SOFTWARE_UPDATE_TRIGGERED, Event;
      software_batch_update_triggered => SOFTWARE_BATCH_UPDATE_TRIGGERED, Event;
      software_update_started => SOFTWARE_UPDATE_STARTED, Event;
      software_batch_update_started => SOFTWARE_BATCH_UPDATE_STARTED, Event;
      software_update_finalized => SOFTWARE_UPDATE_FINALIZED, Event;
      software_batch_update_finalized => SOFTWARE_BATCH_UPDATE_FINALIZED, Event;
      software_update_stdin_attention => SOFTWARE_UPDATE_STDIN_ATTENTION, Event;
      software_update_interactive_control => SOFTWARE_UPDATE_INTERACTIVE_CONTROL, Event;

      // system service runtime events — Event
      system_service_update_gate => SYSTEM_SERVICE_UPDATE_GATE, Event;
      system_service_machine_id_validate => SYSTEM_SERVICE_MACHINE_ID_VALIDATE, Event;
      system_service_update_freeze_apply => SYSTEM_SERVICE_UPDATE_FREEZE_APPLY, Event;
      system_scheduler_audit_log_cleanup => SYSTEM_SCHEDULER_AUDIT_LOG_CLEANUP, Event;
  }
  ```

- [ ] **Step 4: Run test**

  Run: `cargo test -p uptrakit-audit-log entry::tests::audit_actions_macro_generates_event_constructor`
  Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/audit-log-derive/src/audit_actions.rs crates/shared/audit-log/src/action_type.rs
  git commit -m "feat(audit-v2): audit_actions! macro generates per-action constructors"
  ```

---

## Task 10: Compile-fail tests for the typestate

**Files:**

- Create: `crates/shared/audit-log/tests/typestate.rs`
- Create: `crates/shared/audit-log/tests/typestate_compile_fail/missing_after.rs`
- Create: `crates/shared/audit-log/tests/typestate_compile_fail/event_with_before.rs`
- Create: `crates/shared/audit-log/tests/typestate_compile_fail/wrong_kind_to_emit.rs`

- [ ] **Step 1: Add the trybuild runner**

  Create `crates/shared/audit-log/tests/typestate.rs`:

  ```rust
  #[test]
  fn typestate_compile_failures() {
      let t = trybuild::TestCases::new();
      t.compile_fail("tests/typestate_compile_fail/*.rs");
  }
  ```

- [ ] **Step 2: Add the three compile-fail cases**

  `tests/typestate_compile_fail/missing_after.rs`:

  ```rust
  use uptrakit_audit_log::{AuditEntry, AuditView};

  struct Demo;
  impl AuditView for Demo {
      const TARGET_TYPE: &'static str = "demo";
      fn audit_target_id(&self) -> String { "x".into() }
      fn audit_target_display(&self) -> Option<String> { None }
      fn audit_view(&self) -> serde_json::Value { serde_json::json!({}) }
  }

  fn main() {
      // Stateful builder needs both .before() and .after() before .build() is callable.
      let _ = AuditEntry::plugin_config_update(&Demo, &Demo);
      // Intentionally call .build() on a partially-populated builder constructed manually:
      let partial = AuditEntry::<uptrakit_audit_log::Stateful>::builder_stateful(
          uptrakit_audit_log::AuditActionType::from(uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE),
      ).before(&Demo);
      let _ = partial.build();
  }
  ```

  `tests/typestate_compile_fail/event_with_before.rs`:

  ```rust
  use uptrakit_audit_log::AuditEntry;

  struct Demo;
  impl uptrakit_audit_log::AuditView for Demo {
      const TARGET_TYPE: &'static str = "demo";
      fn audit_target_id(&self) -> String { "x".into() }
      fn audit_target_display(&self) -> Option<String> { None }
      fn audit_view(&self) -> serde_json::Value { serde_json::json!({}) }
  }

  fn main() {
      let builder = AuditEntry::auth_login();
      // Event builder has no .before() method.
      let _ = builder.before(&Demo);
  }
  ```

  `tests/typestate_compile_fail/wrong_kind_to_emit.rs`:

  ```rust
  use uptrakit_audit_log::AuditEntry;

  fn main() {
      let event = AuditEntry::auth_login().actor_system().build().expect("event");
      // Stub emit_stateful signature — should not accept AuditEntry<Event>.
      fn emit_stateful(_e: AuditEntry<uptrakit_audit_log::Stateful>) {}
      emit_stateful(event);
  }
  ```

- [ ] **Step 3: Run**

  Run: `cargo test -p uptrakit-audit-log --test typestate`
  Expected: PASS (trybuild compiles cases and confirms they fail as expected).

- [ ] **Step 4: Commit**

  ```bash
  git add crates/shared/audit-log/tests/
  git commit -m "test(audit-v2): trybuild compile-fail cases for AuditEntry typestate"
  ```

---

## Task 11: V2 schema migration

**Files:**

- Create: `crates/shared/db/src/migration/m20260512_000001_audit_logs_v2.rs`
- Modify: `crates/shared/db/src/migration/mod.rs`

- [ ] **Step 1: Write the migration**

  Use `ColumnDef::json_binary()` for snapshot columns. Drop V1 tables then create V2 tables with `CHECK` constraints + indexes. Pattern
  exactly matches `m20260417_000001_semantic_audit_logs.rs` — same `helpers::set_foreign_keys` discipline.

  ```rust
  use sea_orm_migration::prelude::*;

  use crate::migration::helpers;

  #[derive(DeriveMigrationName)]
  pub(super) struct Migration;

  #[async_trait::async_trait]
  impl MigrationTrait for Migration {
      async fn up(&self, m: &SchemaManager) -> Result<(), DbErr> {
          helpers::set_foreign_keys(m, false).await?;
          m.drop_table(Table::drop().table(Alias::new("system_audit_logs")).if_exists().to_owned()).await?;
          m.drop_table(Table::drop().table(Alias::new("audit_logs")).if_exists().to_owned()).await?;
          m.create_table(build_audit_logs("audit_logs")).await?;
          m.create_table(build_system_audit_logs("system_audit_logs")).await?;
          create_indexes(m).await?;
          helpers::set_foreign_keys(m, true).await?;
          Ok(())
      }
      async fn down(&self, m: &SchemaManager) -> Result<(), DbErr> {
          m.drop_table(Table::drop().table(Alias::new("system_audit_logs")).if_exists().to_owned()).await?;
          m.drop_table(Table::drop().table(Alias::new("audit_logs")).if_exists().to_owned()).await
      }
  }

  fn audit_kind_check(table: &str) -> Expr {
      // CHECK ((action_kind = 'event' AND before_snapshot IS NULL AND after_snapshot IS NULL)
      //       OR (action_kind = 'stateful' AND before_snapshot IS NOT NULL AND after_snapshot IS NOT NULL))
      Expr::cust(&format!(
          "((action_kind = 'event' AND before_snapshot IS NULL AND after_snapshot IS NULL) \
           OR (action_kind = 'stateful' AND before_snapshot IS NOT NULL AND after_snapshot IS NOT NULL))",
      ))
  }

  fn build_audit_logs(name: &str) -> TableCreateStatement {
      Table::create()
          .table(Alias::new(name))
          .col(ColumnDef::new(Alias::new("id")).uuid().not_null().primary_key())
          .col(ColumnDef::new(Alias::new("tenant_id")).uuid().not_null())
          .col(ColumnDef::new(Alias::new("occurred_at")).timestamp_with_time_zone().not_null())
          .col(ColumnDef::new(Alias::new("actor_type")).string_len(32).not_null())
          .col(ColumnDef::new(Alias::new("actor_id")).uuid())
          .col(ColumnDef::new(Alias::new("actor_display")).string_len(255))
          .col(ColumnDef::new(Alias::new("action_type")).string_len(128).not_null())
          .col(ColumnDef::new(Alias::new("action_kind")).string_len(16).not_null())
          .col(ColumnDef::new(Alias::new("target_type")).string_len(128))
          .col(ColumnDef::new(Alias::new("target_id")).string_len(255))
          .col(ColumnDef::new(Alias::new("target_display")).string_len(255))
          .col(ColumnDef::new(Alias::new("outcome")).string_len(32).not_null())
          .col(ColumnDef::new(Alias::new("details_json")).json_binary())
          .col(ColumnDef::new(Alias::new("before_snapshot")).json_binary())
          .col(ColumnDef::new(Alias::new("after_snapshot")).json_binary())
          .col(ColumnDef::new(Alias::new("correlation_id")).uuid())
          .col(ColumnDef::new(Alias::new("request_id")).string_len(255))
          .check(audit_kind_check(name))
          .to_owned()
  }

  fn build_system_audit_logs(name: &str) -> TableCreateStatement {
      // identical to build_audit_logs minus tenant_id
      // …
  }

  async fn create_indexes(m: &SchemaManager) -> Result<(), DbErr> {
      // tenant table V1 indexes
      for (cols, name) in [
          (vec!["tenant_id", "occurred_at"], "idx_audit_tenant_time"),
          (vec!["tenant_id", "action_type", "occurred_at"], "idx_audit_tenant_action_time"),
          (vec!["tenant_id", "actor_type", "occurred_at"], "idx_audit_tenant_actor_time"),
          (vec!["tenant_id", "outcome", "occurred_at"], "idx_audit_tenant_outcome_time"),
          (vec!["tenant_id", "target_type", "occurred_at"], "idx_audit_tenant_target_type_time"),
          (vec!["tenant_id", "target_id", "occurred_at"], "idx_audit_tenant_target_id_time"),
          (vec!["tenant_id", "actor_id", "occurred_at"], "idx_audit_tenant_actor_id_time"),
          // V2 additions
          (vec!["tenant_id", "correlation_id"], "idx_audit_tenant_correlation"),
          (vec!["tenant_id", "action_kind", "occurred_at"], "idx_audit_tenant_kind_time"),
      ] { create_index(m, "audit_logs", &cols, name).await?; }

      // system table indexes (mirrors above without tenant_id)
      for (cols, name) in [
          (vec!["occurred_at"], "idx_system_audit_time"),
          (vec!["action_type", "occurred_at"], "idx_system_audit_action_time"),
          (vec!["actor_type", "occurred_at"], "idx_system_audit_actor_time"),
          (vec!["outcome", "occurred_at"], "idx_system_audit_outcome_time"),
          (vec!["target_type", "occurred_at"], "idx_system_audit_target_type_time"),
          (vec!["target_id", "occurred_at"], "idx_system_audit_target_id_time"),
          (vec!["actor_id", "occurred_at"], "idx_system_audit_actor_id_time"),
          (vec!["correlation_id"], "idx_system_audit_correlation"),
          (vec!["action_kind", "occurred_at"], "idx_system_audit_kind_time"),
      ] { create_index(m, "system_audit_logs", &cols, name).await?; }

      Ok(())
  }

  async fn create_index(m: &SchemaManager, table: &str, cols: &[&str], name: &str) -> Result<(), DbErr> {
      let mut idx = Index::create().name(name).table(Alias::new(table)).to_owned();
      for c in cols { idx = idx.col(Alias::new(*c)).to_owned(); }
      m.create_index(idx).await
  }
  ```

- [ ] **Step 2: Register the migration**

  In `crates/shared/db/src/migration/mod.rs`, add `mod m20260512_000001_audit_logs_v2;` and append
  `Box::new(m20260512_000001_audit_logs_v2::Migration)` to the `Migrator::migrations()` vector.

- [ ] **Step 3: Run migration against in-memory SQLite**

  Run: `cargo test -p uptrakit-shared-db --features db-sqlite`
  Expected: PASS (existing migration test harness applies the new migration without errors).

- [ ] **Step 4: Add a CHECK-constraint regression test**

  Append to existing migration tests:

  ```rust
  #[tokio::test]
  async fn audit_v2_check_rejects_event_with_snapshots() {
      let db = setup_test_db().await;
      // Attempt to insert action_kind = 'event' with before_snapshot set; expect DB error.
      let res = db
          .execute_unprepared(
              "INSERT INTO audit_logs (id, tenant_id, occurred_at, actor_type, action_type, action_kind, outcome, before_snapshot) \
               VALUES ('00000000-0000-0000-0000-000000000001', '00000000-0000-0000-0000-000000000002', \
               CURRENT_TIMESTAMP, 'system', 'auth.login', 'event', 'success', '{}')"
          ).await;
      assert!(res.is_err(), "CHECK should reject event+before_snapshot");
  }
  ```

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/db/src/migration/
  git commit -m "feat(audit-v2): migration drops V1 audit tables and creates V2 schema"
  ```

---

## Task 12: Backend writes accept `AuditEntry<K>`

**Files:**

- Modify: `crates/shared/audit-log/src/backend.rs`

The V1 `AuditLogBackend::write(&AuditEntry)` becomes generic over the kind. Easiest path: introduce an erased value
`AuditEntryErased` produced from `AuditEntry<K>` via a `From<AuditEntry<Stateful>>` and `From<AuditEntry<Event>>`. Backends operate on the
erased type. The erased type also carries `action_kind: AuditActionKind`.

- [ ] **Step 1: Add `AuditEntryErased` to `entry.rs`**

  ```rust
  /// Type-erased entry used at backend boundaries. Includes `action_kind` so the
  /// DB backend can persist the `action_kind` column and so the journald backend
  /// can emit the right field set.
  #[derive(Clone, Debug)]
  pub struct AuditEntryErased {
      pub id: Uuid,
      pub tenant_id: Option<Uuid>,
      pub occurred_at: OffsetDateTime,
      pub actor_type: AuditActorType,
      pub actor_id: Option<Uuid>,
      pub actor_display: Option<String>,
      pub action_type: AuditActionType,
      pub action_kind: AuditActionKind,
      pub target_type: Option<String>,
      pub target_id: Option<String>,
      pub target_display: Option<String>,
      pub outcome: AuditOutcome,
      pub details_json: Option<Value>,
      pub before_snapshot: Option<Value>,
      pub after_snapshot: Option<Value>,
      pub correlation_id: Option<Uuid>,
      pub request_id: Option<String>,
  }

  impl From<AuditEntry<Event>> for AuditEntryErased { /* maps fields; sets action_kind = Event */ }
  impl From<AuditEntry<Stateful>> for AuditEntryErased { /* maps fields; sets action_kind = Stateful */ }
  ```

- [ ] **Step 2: Update the backend trait + implementations**

  Replace `AuditLogBackend::write(&AuditEntry)` with `write(&AuditEntryErased)`. Add a separate trait method
  `write_in_tx(&AuditEntryErased, &DatabaseTransaction)` defaulted to delegate to `write` for non-DB backends.

- [ ] **Step 3: Update Database/Journald/Noop/Multiplex backends**

  Database backend: persist `action_kind`, `before_snapshot`, `after_snapshot`, `correlation_id` columns. Journald backend: emit
  `audit_id`, `action_kind`, snapshot bytes counts (not the snapshots themselves), correlation_id, plus V1 fields.

- [ ] **Step 4: Compile + test**

  Run: `cargo test -p uptrakit-audit-log --all-features`
  Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/audit-log/src/
  git commit -m "feat(audit-v2): backends accept AuditEntryErased; persist V2 columns"
  ```

---

## Task 13: `AuditCommitHook` accumulator

**Files:**

- Create: `crates/shared/audit-log/src/commit_hook.rs`
- Modify: `crates/shared/audit-log/src/lib.rs`

- [ ] **Step 1: Write failing test**

  In `commit_hook.rs`:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use crate::backend::{AuditLogBackend, NoopBackend};

      #[tokio::test]
      async fn commit_hook_flushes_to_journald_only_on_caller_flush() {
          let count = std::sync::Arc::new(parking_lot::Mutex::new(0_usize));
          struct Counting(std::sync::Arc<parking_lot::Mutex<usize>>);
          #[async_trait::async_trait]
          impl AuditLogBackend for Counting {
              async fn write(&self, _e: &AuditEntryErased) -> Result<(), AuditLogError> { *self.0.lock() += 1; Ok(()) }
          }
          let mirror = std::sync::Arc::new(Counting(count.clone()));
          let mut hook = AuditCommitHook::new(mirror.clone());
          hook.enqueue(make_stub_erased());
          assert_eq!(*count.lock(), 0);
          hook.flush_after_commit().await;
          assert_eq!(*count.lock(), 1);
      }

      #[tokio::test]
      async fn commit_hook_drops_when_not_flushed() {
          let count = std::sync::Arc::new(parking_lot::Mutex::new(0_usize));
          // Drop without flush; count must remain 0.
          {
              let _hook = AuditCommitHook::new(std::sync::Arc::new(NoopBackend));
              // never call flush_after_commit
          }
          assert_eq!(*count.lock(), 0);
      }
  }
  ```

- [ ] **Step 2: Run test (expected fail)**

  Run: `cargo test -p uptrakit-audit-log commit_hook`
  Expected: FAIL.

- [ ] **Step 3: Implement `AuditCommitHook`**

  ```rust
  use std::sync::Arc;

  use crate::backend::AuditLogBackend;
  use crate::entry::AuditEntryErased;

  /// Buffers stateful audit entries that have been written to the DB transaction
  /// but not yet mirrored to non-DB backends (e.g. journald). The caller flushes
  /// via [`flush_after_commit`] immediately after `tx.commit().await` succeeds.
  /// Dropping without flushing discards the entries (e.g. on rollback).
  pub struct AuditCommitHook {
      mirror: Arc<dyn AuditLogBackend>,
      pending: parking_lot::Mutex<Vec<AuditEntryErased>>,
  }

  impl AuditCommitHook {
      pub fn new(mirror: Arc<dyn AuditLogBackend>) -> Self {
          Self { mirror, pending: parking_lot::Mutex::new(Vec::new()) }
      }

      pub fn enqueue(&self, entry: AuditEntryErased) {
          self.pending.lock().push(entry);
      }

      /// Flush all enqueued entries to the mirror backend.
      /// Failures are logged at `error!` and do not propagate.
      pub async fn flush_after_commit(self) {
          let pending = std::mem::take(&mut *self.pending.lock());
          for entry in pending {
              if let Err(error) = self.mirror.write(&entry).await {
                  tracing::error!(error = %error, action_type = %entry.action_type, "audit commit-hook flush failed");
              }
          }
      }
  }
  ```

  Re-export in `lib.rs`: `pub use commit_hook::AuditCommitHook;` and `mod commit_hook;`.

- [ ] **Step 4: Run test**

  Run: `cargo test -p uptrakit-audit-log commit_hook`
  Expected: PASS.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/audit-log/src/
  git commit -m "feat(audit-v2): AuditCommitHook accumulator for post-commit journald flush"
  ```

---

## Task 14: `AuditEmitter::emit_stateful` and `emit_event`

**Files:**

- Modify: `crates/shared/audit-log/src/emitter.rs`

`emit_event` is the renamed/typed variant of V1 `emit_best_effort`. `emit_stateful` writes inline to the supplied `&DatabaseTransaction`
via the DB backend; enqueues the same erased entry on a returned `AuditCommitHook` (or one passed in — see signature).

- [ ] **Step 1: Update the emitter API**

  ```rust
  use std::sync::Arc;

  use crate::backend::{AuditLogBackend, MultiplexBackend};
  use crate::commit_hook::AuditCommitHook;
  use crate::dispatcher::AuditLogDispatcher;
  use crate::entry::{AuditEntry, AuditEntryErased, Event, Stateful};
  use crate::error::AuditLogError;

  pub struct AuditEmitter {
      dispatcher: AuditLogDispatcher,
      // The DB backend used for the synchronous stateful write path.
      db_backend: Arc<dyn AuditLogBackend>,
      // The mirror (typically journald + any other non-DB backend) used for post-commit flush.
      mirror_backend: Arc<dyn AuditLogBackend>,
      correlation_id: Option<uuid::Uuid>,
  }

  impl AuditEmitter {
      pub fn new(
          dispatcher: AuditLogDispatcher,
          db_backend: Arc<dyn AuditLogBackend>,
          mirror_backend: Arc<dyn AuditLogBackend>,
      ) -> Self {
          Self { dispatcher, db_backend, mirror_backend, correlation_id: None }
      }

      #[must_use]
      pub fn with_correlation(&self, correlation_id: uuid::Uuid) -> Self {
          Self {
              dispatcher: self.dispatcher.clone(),
              db_backend: self.db_backend.clone(),
              mirror_backend: self.mirror_backend.clone(),
              correlation_id: Some(correlation_id),
          }
      }

      pub fn commit_hook(&self) -> AuditCommitHook {
          AuditCommitHook::new(self.mirror_backend.clone())
      }

      /// Fire-and-forget event emission. Mirrors V1 `emit_best_effort` semantics.
      pub fn emit_event(&self, mut entry: AuditEntry<Event>) {
          if entry.correlation_id.is_none() { entry.correlation_id = self.correlation_id; }
          self.dispatcher.enqueue(entry.into());
      }

      /// Stateful emission. Writes the audit row through `db_backend.write_in_tx`
      /// against the supplied transaction. Enqueues the same entry on `hook`
      /// for post-commit journald flush. Failure rolls back the caller's
      /// transaction (propagated via `?`).
      pub async fn emit_stateful(
          &self,
          tx: &sea_orm::DatabaseTransaction,
          hook: &AuditCommitHook,
          mut entry: AuditEntry<Stateful>,
      ) -> rootcause::Result<(), AuditLogError> {
          if entry.correlation_id.is_none() { entry.correlation_id = self.correlation_id; }
          let erased: AuditEntryErased = entry.into();
          self.db_backend.write_in_tx(&erased, tx).await?;
          hook.enqueue(erased);
          Ok(())
      }
  }
  ```

- [ ] **Step 2: Keep `emit_best_effort` as a `#[deprecated]` shim**

  ```rust
  impl AuditEmitter {
      #[deprecated(since = "0.1.0", note = "use emit_event for Event entries; emit_stateful for Stateful entries")]
      #[doc(hidden)]
      pub fn emit_best_effort(&self, entry: AuditEntry<Event>) { self.emit_event(entry); }
  }
  ```

  This preserves V1 producer call sites compile-correctness until Plan B removes them.

- [ ] **Step 3: Compile + run existing emitter tests**

  Run: `cargo test -p uptrakit-audit-log --all-features`
  Expected: PASS (the existing dispatcher round-trip tests still pass; new emitter shape is additive).

- [ ] **Step 4: Add a round-trip test for `emit_stateful`**

  Use a SQLite in-memory pool. Project a `Demo` entity via `AuditView`, build an `AuditEntry<Stateful>`, open a transaction, call
  `emit_stateful`, assert exactly one row exists. Force a `tx.rollback()` in a second scenario and assert zero rows.

- [ ] **Step 5: Commit**

  ```bash
  git add crates/shared/audit-log/src/emitter.rs
  git commit -m "feat(audit-v2): AuditEmitter.emit_stateful/emit_event + with_correlation"
  ```

---

## Task 15: Test ergonomics — `event_test_stub` + `stateful_test_stub`

**Files:**

- Modify: `crates/shared/audit-log/src/entry.rs`

- [ ] **Step 1: Replace V1 `test_stub` with two helpers**

  ```rust
  impl AuditEntry<Event> {
      pub fn event_test_stub(action: impl AsRef<str>) -> Self {
          Self::builder_event(action.as_ref().parse().expect("registered action"))
              .actor_system()
              .build()
              .expect("event stub builds")
      }
  }
  impl AuditEntry<Stateful> {
      pub fn stateful_test_stub(action: &str, target_type: &str, target_id: String, before: Value, after: Value) -> Self {
          struct Stub<'a> { target_type: &'a str, target_id: String, value: Value }
          impl<'a> AuditView for Stub<'a> {
              const TARGET_TYPE: &'static str = ""; // overridden via target() below
              fn audit_target_id(&self) -> String { self.target_id.clone() }
              fn audit_target_display(&self) -> Option<String> { None }
              fn audit_view(&self) -> Value { self.value.clone() }
          }
          let b = Stub { target_type, target_id: target_id.clone(), value: before };
          let a = Stub { target_type, target_id, value: after };
          Self::builder_stateful(action.parse().expect("registered action"))
              .target(target_type, b.target_id.clone(), None)
              .before(&b)
              .after(&a)
              .build()
              .expect("stateful stub builds")
      }
  }
  ```

- [ ] **Step 2: Test the helpers**

  Append `entry.rs` tests:

  ```rust
  #[test]
  fn event_test_stub_builds_event_entry() {
      let e = AuditEntry::event_test_stub("auth.login");
      assert_eq!(e.action_type.as_str(), "auth.login");
      assert!(e.before_snapshot.is_none() && e.after_snapshot.is_none());
  }

  #[test]
  fn stateful_test_stub_builds_stateful_entry() {
      let e = AuditEntry::stateful_test_stub(
          "plugin_config.update", "plugin_config",
          uuid::Uuid::now_v7().to_string(),
          serde_json::json!({"enabled": false}),
          serde_json::json!({"enabled": true}),
      );
      assert_eq!(e.action_type.as_str(), "plugin_config.update");
      assert_eq!(e.before_snapshot.as_ref().unwrap()["enabled"], false);
      assert_eq!(e.after_snapshot.as_ref().unwrap()["enabled"], true);
  }
  ```

  Run: `cargo test -p uptrakit-audit-log entry::tests`. Expected: PASS.

- [ ] **Step 3: Commit**

  ```bash
  git add crates/shared/audit-log/src/entry.rs
  git commit -m "test(audit-v2): event_test_stub + stateful_test_stub helpers"
  ```

---

## Task 16: Quality gates + push

- [ ] **Step 1: Run the full quality gate suite**

  ```bash
  cargo fmt --all
  cargo check --no-default-features --features db-sqlite
  cargo check --all-features
  cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --all-features
  cargo deny check
  markdownlint --config .markdownlint.json '**/*.md'
  ```

  Expected: all green. Fix any findings root-cause rather than suppressing.

- [ ] **Step 2: Push branch**

  ```bash
  git push -u origin feat/audit-v2-foundation
  ```

---

## Spec coverage check (Plan A scope)

This plan delivers:

- Spec §"Data model" — schema columns, `CHECK` constraints, indexes (Task 11).
- Spec §"`AuditView` trait and derive macro" (Tasks 6, 7).
- Spec §"Action kind classification and typestate builder" (Tasks 4, 5, 8, 9, 10).
- Spec §"Emitter API" — `emit_stateful`, `emit_event`, `AuditCommitHook`, scoped `with_correlation` (Tasks 13, 14).
- Spec §"Initial catalog classification" — full sweep applied via the `audit_actions!` invocation (Task 9).
- Workspace `serde_json/preserve_order` enabling (Task 2).
- Test ergonomics replacement helpers (Task 15).

Deferred to Plan B: every producer call-site migration (~100 sites), the `AuditView` impls on real domain entities, correlation_id
threading at workflow heads, wire payload extension and ingress validation, and the `emit_best_effort` removal.

Deferred to Plan C: catalog file + static-analysis CI gate.

Deferred to Plan D: frontend State tab and CLI rendering.

Deferred to Plan E: documentation deliverables and the new ADR.
