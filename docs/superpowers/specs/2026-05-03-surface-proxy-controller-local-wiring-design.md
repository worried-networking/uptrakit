# Surface Proxy: Wire `controller_local.rs` into the Module Tree

**Date:** 2026-05-03  
**Crate:** `uptrakit-surface-proxy`

## Background

`proxy.rs` is a 5,923-line file that contains everything for the surface proxy: the core
`SurfaceProxy` struct, trait definitions, all allowlist/dispatch logic for five controller-local
action families, all audit emission functions, and a ~3,200-line inline test module. The
extraction is already partially done:

- `proxy/controller_local.rs` and its submodules contain the clean business logic
- `proxy/local_executor.rs` contains clean typed trait + struct definitions
- `proxy/tests.rs` and subdirectories contain the replacement test suite

None of these files are in the module tree yet. `proxy.rs` has `mod controller_local;` (business
logic is live) but no `mod local_executor;` and no external `mod tests;`. The clean code exists
but has no compiled callers, causing suppression annotations throughout.

This spec covers completing the wiring: connecting the new files, migrating the three missing
action families, and deleting the old inline code.

## Goals

1. Add `mod local_executor;` and `#[cfg(test)] mod tests;` to `proxy.rs`
2. Delete all old inline code from `proxy.rs` (traits, impls, allowlist/dispatch fns, audit fns,
   inline test block) — ~3,200 lines removed
3. Migrate three missing action families from the inline code to new `controller_local/` submodules
4. Clean up all suppression annotations that exist solely because the new code has no callers
5. Simplify the public API: `PluginOpsSurfaceActionInvoker` becomes an internal detail

## Non-Goals

- Changes to the surface registry, routing, or any layer above `SurfaceProxy`
- Changes to the Proxmox, Docker, or notification plugin implementations themselves
- Any behavioural changes — this is a structural migration only

---

## Design

### Trait refactor: single-method `PluginSurfaceActionInvoker`

The current `local_executor.rs` trait has three methods: `invoke`, plus two optional
`invoke_allowlisted_*` methods. These exist because the old `proxy.rs` inline code
delegated the allowlisted paths through the invoker trait (to keep the struct testable).

With the clean architecture in place, this delegation is unnecessary. The two allowlisted
families that have custom execution logic (notification channel CRUD, proxmox add-config)
call `controller_local` functions directly from `PluginSurfaceLocalExecutor::execute()` using
the stored `db` and `plugin_ops`. The trait reduces to a single method:

```rust
#[async_trait]
pub trait PluginSurfaceActionInvoker: Send + Sync {
    async fn invoke(
        &self,
        db: Option<&DatabaseConnection>,
        tenant_id: Option<Uuid>,
        caller_user_id: Option<Uuid>,
        surface_id: &str,
        interaction_id: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError>;
}
```

`#[async_trait]` is retained — the workspace has not dropped it, and `dyn PluginSurfaceActionInvoker`
requires object-safe async dispatch.

`PluginOpsSurfaceActionInvoker` implements this trait (just `invoke`). It is no longer part
of the public crate API — it becomes `pub(super)` inside `local_executor.rs` and is
constructed internally by `PluginSurfaceLocalExecutor::new`.

### `PluginSurfaceLocalExecutor` struct

Gains a `plugin_ops` field alongside the existing `action_context_db` and `plugin_invoker`:

```rust
pub struct PluginSurfaceLocalExecutor {
    action_context_db: Option<Arc<DatabaseConnection>>,
    plugin_ops: Option<Arc<dyn PluginOps>>,
    plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>,
    audit_emitter: Option<uptrakit_audit_log::AuditEmitter>,
}
```

**Production constructor** (used by controller-runtime):

```rust
pub fn new(db: Arc<DatabaseConnection>, plugin_ops: Arc<dyn PluginOps>) -> Self
```

Constructs `PluginOpsSurfaceActionInvoker` internally from `plugin_ops`. The call site in
`controller-runtime/src/lib.rs` simplifies from:

```rust
// before
PluginSurfaceLocalExecutor::new(
    Arc::new(db_conn.clone()),
    Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
)
.with_audit_emitter(audit_emitter.clone())
// after
PluginSurfaceLocalExecutor::new(Arc::new(db_conn.clone()), Arc::clone(&plugin_ops))
.with_audit_emitter(audit_emitter.clone())
```

The `.with_audit_emitter(...)` builder method is retained unchanged — the struct gains a
`plugin_ops` field but the builder chain is unaffected.

**Test constructor** (unchanged shape, kept `#[cfg(test)]`):

```rust
pub fn new_without_database(plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>) -> Self
```

Sets `action_context_db` and `plugin_ops` to `None`. Valid only for Tier 2 and Tier 3 paths —
any Tier 1 path reached with this constructor will return an internal-error
`SchemaValidationFailed`. DB-backed Tier 1 paths are tested via the `controller_owned` suite
using the production constructor.

### Three-tier dispatch in `execute()`

`PluginSurfaceLocalExecutor::execute()` dispatches across three tiers:

**Tier 1 — custom executor (db + plugin_ops required):**

| Family                    | Module                                   | Function                                          |
| ------------------------- | ---------------------------------------- | ------------------------------------------------- |
| Notification channel CRUD | `controller_local/notifications.rs`      | `execute_allowlisted_notification_channel_action` |
| Proxmox add-config        | `controller_local/proxmox_add_config.rs` | `execute_allowlisted_proxmox_add_config_action`   |

These call `controller_local` functions directly via `self.action_context_db` + `self.plugin_ops`.
Both must be `Some` or the function returns `SurfaceProxyError::SchemaValidationFailed` with an
internal-error message (matching the existing guard pattern in `local_executor.rs`).

Audit emission is required for both families in the Tier 1 path:

- **Proxmox add-config**: `local_executor.rs` already emits `emit_proxmox_add_config_audit_event`
  on success only (success path, lines 282–288). Retain this behavior.
- **Notification channel CRUD**: the current `local_executor.rs` does NOT emit audit (it was
  never wired). The existing `proxy.rs` inline code emits `emit_notification_channel_audit_event`
  on **both success and failure**. After migration, `execute()` must emit on both outcomes to
  match the inline behavior — this is distinct from the proxmox add-config pattern which emits
  on success only. The audit assertion test (see Inline test migration section) enforces this.

Additionally, the call site for `execute_allowlisted_proxmox_add_config_action` in
`local_executor.rs` (line 152) is missing the required `plugin_type: PluginTypeId` argument —
the function signature takes four arguments but the current call passes only three. This is a
latent build error that will surface the moment `local_executor.rs` is wired in. The correct
`plugin_type` to pass is `uptrakit_shared_types::plugin_ids::INFRASTRUCTURE_PROXMOX.clone()`,
which is consistent with the allowlist guard (`surface_id == "proxmox.hosts"`). Fix this as
part of Step 3 when updating `execute()`.

**Tier 2 — generic invoke + audit:**

| Family                    | Allowlist check fn                                              | New module                     | Audit fn                                     |
| ------------------------- | --------------------------------------------------------------- | ------------------------------ | -------------------------------------------- |
| Notification settings     | `allowlisted_notification_settings_controller_local_action`     | `notification_settings.rs`     | `emit_notification_settings_audit_event`     |
| Docker switch-tag         | `allowlisted_docker_switch_tag_controller_local_action`         | `docker.rs`                    | `emit_docker_switch_tag_audit_event`         |
| Proxmox update-protection | `allowlisted_proxmox_update_protection_controller_local_action` | `proxmox_update_protection.rs` | `emit_proxmox_update_protection_audit_event` |

These call `self.plugin_invoker.invoke(...)` (the standard plugin surface-action path) and then
emit an audit event. The allowlist check determines which audit emitter to call. Error branches
also emit audit events with the appropriate failure outcome.

Error mapping for Tier 2 uses `map_surface_action_error`, which maps `InvalidInput` →
`SchemaValidationFailed` and `ControllerIntegration`/`PluginInternal` → `SendFailed`. The old
`proxy.rs` inline code collapsed all errors to `SchemaValidationFailed(error.to_string())`.
This is an intentional behavioral correction — `SendFailed` is the semantically correct
response for internal plugin errors, not a validation failure.

**Tier 3 — generic invoke (no audit):**

Everything else: `self.plugin_invoker.invoke(...)` with `map_surface_action_error`.

### Three new `controller_local/` submodules

#### `notification_settings.rs`

Migrated from `proxy.rs` inline code. Contains:

- `allowlisted_notification_settings_controller_local_action(provider_id, surface_id, interaction_id) -> Option<NotificationSettingsAction>`
- `NotificationSettingsAction` enum (`ConfigureSmtp`, `SaveGlobalSmtp`, `SaveGlobalTelegram`) — must carry
  `#[non_exhaustive]` per project standard; external match sites require a wildcard arm with `tracing::warn!`
- `emit_notification_settings_audit_event(emitter, caller_user_id, tenant_id, action, params, result)`

Note: notification settings actions use the existing `plugin_ops.handle_surface_action()` path
(Tier 2 invoke) — no custom execution function is needed in this module.

#### `docker.rs`

Migrated from `proxy.rs` inline code. Contains:

- `allowlisted_docker_switch_tag_controller_local_action(provider_id, surface_id, interaction_id) -> bool`
- `emit_docker_switch_tag_audit_event(emitter, caller_user_id, tenant_id, params, result)`
- `classify_docker_switch_tag_error(error) -> (AuditOutcome, &'static str)`

Note: docker switch-tag also uses the Tier 2 invoke path — the `DockerSurfaceStore` impl on
`AppStateSurfaceActionController` handles the actual DB work inside `handle_surface_action`.

#### `proxmox_update_protection.rs`

Migrated from `proxy.rs` inline code. Contains:

- `allowlisted_proxmox_update_protection_controller_local_action(surface_id, interaction_id) -> Option<ProxmoxUpdateProtectionAction>`
- `ProxmoxUpdateProtectionAction` enum (`SaveGlobalDefaults`, `SaveItemOverrides`) — must carry
  `#[non_exhaustive]` per project standard; external match sites require a wildcard arm with `tracing::warn!`
- `emit_proxmox_update_protection_audit_event(emitter, caller_user_id, tenant_id, action, params, result)`
- `classify_proxmox_update_protection_error(error) -> (AuditOutcome, &'static str)`
- Helper fns: `proxmox_update_protection_action_type`, `proxmox_update_protection_mutation_source`

Note: proxmox update-protection uses the Tier 2 invoke path — the `ProxmoxSurfaceStore` impl
handles the DB work inside `handle_surface_action`.

### Re-export surgery in `controller_local.rs`

New module declarations added:

```rust
mod notification_settings;
mod docker;
mod proxmox_update_protection;
```

Re-export changes:

- Remove `#![expect(unreachable_pub)]` crate-level attribute (items now have live callers)
- Remove `#[expect(unused_imports)]` from all non-test re-export blocks
- Keep `#[cfg(test)]` gate on `build_notification_channel_create_request` /
  `build_notification_channel_update_request` re-exports (test-only), but remove
  `#[expect(unused_imports)]` from that block (tests are live once `mod tests;` is wired)
- Add re-exports for new modules: allowlist fns, emit fns, action enums

### `proxy.rs` changes

**Additions:**

```rust
mod local_executor;
pub use local_executor::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor,
    SurfaceLocalActionExecutor,
};
pub use controller_local::map_surface_action_error;

#[cfg(test)]
mod tests;
```

`NoopSurfaceLocalExecutor` is `pub(super)` in `local_executor.rs` and stays internal — it is
only used by `SurfaceProxy::new()` inside `proxy.rs`. It does not appear in the public API.

`map_surface_action_error` lives in `controller_local.rs` (already `mod controller_local;` in
`proxy.rs`); re-exporting it directly from there avoids threading it through `local_executor.rs`.

**Deletions (all inline):**

- `PluginSurfaceActionInvoker` trait definition
- `PluginOpsSurfaceActionInvoker` struct + impl
- `PluginSurfaceLocalExecutor` struct + impl
- All 5 allowlist functions
- All audit emission functions and classify helpers
- Helper functions duplicated from `controller_local/` (`notification_channel_type_from_surface`,
  `allowlisted_proxmox_provider`, `execute_allowlisted_*`, `build_*`, `required_uuid_param`,
  `require_notification_channel_type`)
- `NoopSurfaceLocalExecutor` struct + impl (moves to `local_executor.rs`)
- `#[cfg(test)] mod tests { ... }` inline block (~3,200 lines)

`AppStateSurfaceActionController` re-export from `controller_local` remains unchanged.

### `lib.rs` public API changes

Remove `PluginOpsSurfaceActionInvoker` from the `pub use` block — it is no longer a public
type. All other current exports remain.

### Attribute cleanup

| File                    | Attribute                                                | Action                       |
| ----------------------- | -------------------------------------------------------- | ---------------------------- |
| `controller_local.rs`   | `#![expect(unreachable_pub)]`                            | Remove                       |
| `controller_local.rs`   | `#[expect(unused_imports)]` on non-test re-exports       | Remove                       |
| `controller_local.rs`   | `#[expect(unused_imports)]` on `#[cfg(test)]` re-exports | Remove (keep `#[cfg(test)]`) |
| `controller_local.rs`   | `#[expect(dead_code)]` on `map_surface_action_error`     | Remove (keep `pub fn`)       |
| `notifications.rs`      | `#![expect(dead_code)]`                                  | Remove                       |
| `notifications.rs`      | `#![expect(unreachable_pub)]`                            | Remove                       |
| `proxmox_add_config.rs` | `#![expect(dead_code)]`                                  | Remove                       |
| `proxmox_add_config.rs` | `#![expect(unreachable_pub)]`                            | Remove                       |

---

## Inline test migration

The inline `#[cfg(test)] mod tests { ... }` block in `proxy.rs` contains approximately 55 tests. These
must be audited against the external `proxy/tests/` suite before deletion. The process:

1. List all inline test function names
2. Map each to the equivalent external test (or flag as missing)
3. Port missing coverage to the appropriate external file:
   - Non-DB executor behavior → `tests/controller_local.rs`
   - DB-backed notification tests → `tests/controller_owned/notifications.rs`
   - DB-backed proxmox tests → `tests/controller_owned/proxmox.rs`
   - Rollout / provider-proxied behavior → `tests/provider_proxied/rollout.rs`
   - General proxied flow (timeout, rate limiting, idempotency, budget) → new files if needed
4. Delete **entire** inline block once coverage is verified

**Trait signature incompatibility — mocks must be rewritten, not moved.** The inline test
mocks in `proxy.rs` implement the old `PluginSurfaceActionInvoker` with signature
`db: &(dyn Any + Send + Sync)`. The external `tests.rs` `TestPluginInvoker` implements the
new signature `db: Option<&DatabaseConnection>`. These are distinct incompatible traits. Every
mock `invoke` implementation ported from the inline block must be rewritten to match the new
signature — verbatim copy-paste will not compile.

**Sequencing constraint: the inline block must be fully deleted (not just partially ported)
before Step 3 begins.** The inline block directly references `PluginOpsSurfaceActionInvoker::new(...)`
in ~14 call sites. Step 3 makes `PluginOpsSurfaceActionInvoker` `pub(super)`, which immediately
breaks compilation for any inline test still referencing the type. Port all missing coverage
first, then delete the entire block in one operation, before starting Step 3.

For the three new action families (notification settings, docker, proxmox update-protection),
new test coverage must be written in the external suite before the inline tests for those
families are deleted:

- `tests/controller_owned/notifications.rs` — add tests for `configure_smtp`,
  `save_global_smtp`, `save_global_telegram` paths
- New `tests/controller_owned/docker.rs` — add tests for switch-tag success + error paths
- New `tests/controller_owned/proxmox_update_protection.rs` — add tests for
  save-global-defaults and save-item-overrides paths

**Audit assertion test as forcing function.** Before implementing the notification channel CRUD
audit emission in Step 3, write an audit assertion test in `tests/controller_owned/notifications.rs`
that verifies `emit_notification_channel_audit_event` is called on a successful create. Writing
this test first ensures the audit path cannot be silently omitted during the Step 3
implementation.

**Submodule test duplication.** `controller_local/notifications.rs` contains an inline
`#[cfg(test)] mod tests` with builder-function unit tests that are also covered (as integration
tests with richer assertions) in `tests/controller_owned/notifications.rs`. The submodule-level
tests are lower-value duplicates. Remove the submodule-level unit tests for
`build_notification_channel_create_request` and `build_notification_channel_update_request` from
`notifications.rs` during Step 2, retaining the integration-level equivalents.

---

## External caller impact

`controller-runtime/src/lib.rs` (lines 440–450): constructor signature changes from
`new(db, invoker)` to `new(db, plugin_ops)`. No other external callers of
`PluginSurfaceLocalExecutor` or `PluginOpsSurfaceActionInvoker` exist outside the crate.

Within the crate, `tests/controller_owned/notifications.rs` (lines 13–14, 102–104, 170–172)
currently imports and calls `PluginOpsSurfaceActionInvoker::new(...)` directly to construct
the executor for integration tests. These call sites must be updated during Step 5 to use
`PluginSurfaceLocalExecutor::new(db, plugin_ops)` instead — the new production constructor
produces the same wired-up executor without exposing the internal invoker type.

---

## Sequence of changes

The implementation should proceed in this order to keep the build green at each step:

1. **Add new submodules** — create `notification_settings.rs`, `docker.rs`,
   `proxmox_update_protection.rs` in `controller_local/`; add `mod` declarations in
   `controller_local.rs`; add re-exports; clean suppression attributes from `notifications.rs`
   and `proxmox_add_config.rs`
2. **Audit and port inline tests** — audit the ~55 inline tests against the external
   `proxy/tests/` suite; port missing coverage; update `tests/controller_owned/notifications.rs`
   to use `PluginSurfaceLocalExecutor::new(db, plugin_ops)` instead of constructing
   `PluginOpsSurfaceActionInvoker` directly; add missing DB-backed tests for the 3 new families
3. **Refactor `local_executor.rs`** — remove `invoke_allowlisted_*` from trait; make
   `PluginOpsSurfaceActionInvoker` `pub(super)` (safe now that tests no longer reference it);
   add `plugin_ops` field; update `new` constructor; update `execute()` for all 5 families
   including audit emission for notification channel CRUD
4. **Wire into `proxy.rs`** — add `mod local_executor;` and re-exports; add
   `#[cfg(test)] mod tests;`; remove inline duplicates (keeping `SurfaceProxy` struct,
   `SurfaceProxyError`, `SurfaceCallerOrigin`, `SurfaceInvokeRequest`, and all non-executor
   logic intact); update `lib.rs` exports; delete inline test block
5. **Update `controller-runtime`** — update `PluginSurfaceLocalExecutor::new` call site

---

## Success criteria

- `cargo check --all-features` passes with no new `#[expect]` annotations added
- `cargo clippy --all-targets --all-features` clean
- `cargo test --all-features` passes
- `proxy.rs` no longer contains any trait or executor definitions — only `SurfaceProxy` struct,
  `SurfaceProxyError`, `SurfaceCallerOrigin`, `SurfaceInvokeRequest`, module declarations,
  and re-exports
- Zero `#[expect(dead_code)]`, `#[expect(unused_imports)]`, or `#[expect(unreachable_pub)]`
  annotations remain in `controller_local.rs` or its submodules
- `PluginOpsSurfaceActionInvoker` is not present in `lib.rs` public exports
