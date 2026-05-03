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
// after
PluginSurfaceLocalExecutor::new(Arc::new(db_conn.clone()), Arc::clone(&plugin_ops))
```

**Test constructor** (unchanged shape, kept `#[cfg(test)]`):

```rust
pub fn new_without_database(plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>) -> Self
```

Sets `action_context_db` and `plugin_ops` to `None`. The test suite uses mock `plugin_invoker`
implementations for the generic invoke path; DB-backed allowlisted paths are tested separately
via the `controller_owned` test suite.

### Three-tier dispatch in `execute()`

`PluginSurfaceLocalExecutor::execute()` dispatches across three tiers:

**Tier 1 — custom executor (db + plugin_ops required):**

| Family                    | Module                                   | Function                                          |
| ------------------------- | ---------------------------------------- | ------------------------------------------------- |
| Notification channel CRUD | `controller_local/notifications.rs`      | `execute_allowlisted_notification_channel_action` |
| Proxmox add-config        | `controller_local/proxmox_add_config.rs` | `execute_allowlisted_proxmox_add_config_action`   |

These call `controller_local` functions directly via `self.action_context_db` + `self.plugin_ops`.
Both must be `Some` or the function returns `SurfaceProxyError::SchemaValidationFailed` with an
internal-error message.

**Tier 2 — generic invoke + audit:**

| Family                    | Allowlist check fn                                              | New module                     | Audit fn                                     |
| ------------------------- | --------------------------------------------------------------- | ------------------------------ | -------------------------------------------- |
| Notification settings     | `allowlisted_notification_settings_controller_local_action`     | `notification_settings.rs`     | `emit_notification_settings_audit_event`     |
| Docker switch-tag         | `allowlisted_docker_switch_tag_controller_local_action`         | `docker.rs`                    | `emit_docker_switch_tag_audit_event`         |
| Proxmox update-protection | `allowlisted_proxmox_update_protection_controller_local_action` | `proxmox_update_protection.rs` | `emit_proxmox_update_protection_audit_event` |

These call `self.plugin_invoker.invoke(...)` (the standard plugin surface-action path) and then
emit an audit event. The allowlist check determines which audit emitter to call. Error branches
also emit audit events with the appropriate failure outcome.

**Tier 3 — generic invoke (no audit):**

Everything else: `self.plugin_invoker.invoke(...)` with `map_surface_action_error`.

### Three new `controller_local/` submodules

#### `notification_settings.rs`

Migrated from `proxy.rs` inline code. Contains:

- `allowlisted_notification_settings_controller_local_action(provider_id, surface_id, interaction_id) -> Option<NotificationSettingsAction>`
- `NotificationSettingsAction` enum (`ConfigureSmtp`, `SaveGlobalSmtp`, `SaveGlobalTelegram`)
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
- `ProxmoxUpdateProtectionAction` enum (`SaveGlobalDefaults`, `SaveItemOverrides`)
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
| `controller_local.rs`   | `#[expect(dead_code)]` on `map_surface_action_error`     | Remove                       |
| `notifications.rs`      | `#![expect(dead_code)]`                                  | Remove                       |
| `notifications.rs`      | `#![expect(unreachable_pub)]`                            | Remove                       |
| `proxmox_add_config.rs` | `#![expect(dead_code)]`                                  | Remove                       |
| `proxmox_add_config.rs` | `#![expect(unreachable_pub)]`                            | Remove                       |

---

## Inline test migration

The inline `#[cfg(test)] mod tests { ... }` block in `proxy.rs` contains ~39 tests. These
must be audited against the external `proxy/tests/` suite before deletion. The process:

1. List all inline test function names
2. Map each to the equivalent external test (or flag as missing)
3. Port missing coverage to the appropriate external file:
   - Non-DB executor behavior → `tests/controller_local.rs`
   - DB-backed notification tests → `tests/controller_owned/notifications.rs`
   - DB-backed proxmox tests → `tests/controller_owned/proxmox.rs`
   - Rollout / provider-proxied behavior → `tests/provider_proxied/rollout.rs`
   - General proxied flow (timeout, rate limiting, idempotency, budget) → new files if needed
4. Delete inline block once coverage is verified

For the three new action families (notification settings, docker, proxmox update-protection),
new test coverage must be written in the external suite before the inline tests for those
families are deleted:

- `tests/controller_owned/notifications.rs` — add tests for `configure_smtp`,
  `save_global_smtp`, `save_global_telegram` paths
- New `tests/controller_owned/docker.rs` — add tests for switch-tag success + error paths
- New `tests/controller_owned/proxmox_update_protection.rs` — add tests for
  save-global-defaults and save-item-overrides paths

---

## External caller impact

`controller-runtime/src/lib.rs` (lines 440–450): constructor signature changes from
`new(db, invoker)` to `new(db, plugin_ops)`. No other external callers of
`PluginSurfaceLocalExecutor` or `PluginOpsSurfaceActionInvoker` exist.

---

## Sequence of changes

The implementation should proceed in this order to keep the build green at each step:

1. **Add new submodules** — create `notification_settings.rs`, `docker.rs`,
   `proxmox_update_protection.rs` in `controller_local/`; add `mod` declarations in
   `controller_local.rs`; add re-exports; clean suppression attributes from `notifications.rs`
   and `proxmox_add_config.rs`
2. **Refactor `local_executor.rs`** — remove `invoke_allowlisted_*` from trait; make
   `PluginOpsSurfaceActionInvoker` `pub(super)`; add `plugin_ops` field; update `new`
   constructor; update `execute()` for all 5 families; add `pub use map_surface_action_error`
3. **Wire into `proxy.rs`** — add `mod local_executor;` and re-exports; add
   `#[cfg(test)] mod tests;`; remove inline duplicates (keeping `SurfaceProxy` struct,
   `SurfaceProxyError`, `SurfaceCallerOrigin`, `SurfaceInvokeRequest`, and all non-executor
   logic intact); update `lib.rs` exports
4. **Update `controller-runtime`** — update `PluginSurfaceLocalExecutor::new` call site
5. **Audit and port inline tests** — for each of the 39 inline tests, confirm coverage in
   external suite or port it; add missing DB-backed tests for the 3 new families; delete
   inline test block

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
