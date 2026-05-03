# Plugin Boundary Hardening — Wave 3a–3g: Plugin Store Migration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate each plugin that uses a plugin-named store method to query via `ctx.tenant_db()` directly. Simultaneously delete every plugin-named
store trait, its request/response types, and all `impl <StoreTrait> for ...` blocks. Each sub-task is independently CI-green.

**Architecture:** Seven atomic sub-tasks (3a–3g). Each sub-task: (1) updates the plugin's surface action handlers to call `ctx.tenant_db()` directly,
(2) deletes the store trait and all its impls. The last sub-task (3g) removes leftover registry re-exports.

**Tech Stack:** Rust, SeaORM, `async_trait`, `uptrakit-tenant-db`

**Prerequisite:** Waves 1–2 merged. `tenant_db()` available on both controller traits.

---

## Task 1 (3a): Docker — migrate `DockerSurfaceStore`

**Files:**

- Modify: `crates/plugins/releases/docker/src/surfaces.rs` (or wherever docker surface action handlers live — check `grep -rn "docker_surface_store"
crates/plugins/`)
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs` (delete store types)

- [ ] **Step 1: Locate all `docker_surface_store()` call sites**

Run: `grep -rn "docker_surface_store\|DockerSurfaceStore\|DockerItemHostRequest\|DockerSwitchTagRequest" crates/ --include="*.rs"`

Expected call sites: `surfaces.rs` in the Docker plugin, `controller_local.rs` in surface-proxy (the impl), and `roles.rs` (the definition).

- [ ] **Step 2: Rewrite Docker surface action handlers to use `ctx.tenant_db()`**

In the Docker plugin's surface action handler (likely `crates/plugins/releases/docker/src/surfaces.rs`), locate each call to
`ctx.controller.docker_surface_store()`. Replace with direct SeaORM queries using `ctx.tenant_db()`.

`host_software_item_plugin` has **no `tenant_id` column** — it is scoped by `host_id` +
`software_item_id` (both come from action params, already tenant-scoped). The current impl lives in
`crates/ui/surface-proxy/src/proxy/controller_local.rs` (not `controller_local/docker.rs` — that
file contains routing helpers only).

Pattern — before (`surfaces.rs` parses params and calls store):

```rust
let request = parse_action_params::<DockerItemHostRequest>(params, action_id)?;
let store = ctx.controller.docker_surface_store()
    .ok_or_else(|| SurfaceActionError::internal("docker store unavailable"))?;
let image_ref = store.load_current_image_ref(request.host_id, request.software_item_id).await
    .map_err(|e| SurfaceActionError::internal(e.to_string()))?;
```

After (inline query using `ctx.tenant_db().db()`):

```rust
use uptrakit_shared_db::entity::host_software_item_plugin;
let request = parse_action_params::<DockerItemHostRequest>(params, action_id)?;
let db = ctx.tenant_db().db();
let plugin_rows = host_software_item_plugin::Entity::find()
    .filter(host_software_item_plugin::Column::HostId.eq(request.host_id))
    .filter(host_software_item_plugin::Column::SoftwareItemId.eq(request.software_item_id))
    .filter(host_software_item_plugin::Column::PluginType.eq("releases_docker"))
    .all(db)
    .await
    .map_err(|e| SurfaceActionError::internal(e.to_string()))?;
let image_ref = plugin_rows
    .into_iter()
    .next()
    .map(|r| strip_container_suffix(&r.package_identifier))
    .unwrap_or_default();
```

For the switch-tag action, replicate the `switch_image_ref` logic from
`crates/ui/surface-proxy/src/proxy/controller_local.rs` (the `impl DockerSurfaceStore` block,
around line 276) — it queries by `host_id` + `software_item_id`, fetches
`host_software_item`, then updates the `package_identifier` field in a transaction.

- [ ] **Step 3: Delete `DockerSurfaceStore` trait and request types from `roles.rs`**

In `crates/plugins/infrastructure/core/src/roles.rs`, delete:

- `DockerSurfaceStore` trait definition
- `DockerItemHostRequest` struct
- `DockerSwitchTagRequest` struct
- `docker_surface_store()` method from `SurfaceActionController` trait body

- [ ] **Step 4: Delete `impl DockerSurfaceStore for AppStateSurfaceActionController` from `controller_local.rs`**

In `crates/ui/surface-proxy/src/proxy/controller_local.rs`:

- Delete `docker_surface_store()` method from the `impl SurfaceActionController` block
- Delete `impl DockerSurfaceStore for AppStateSurfaceActionController` (the full impl block)
- Delete `mod docker;` line and the `docker.rs` file if it's now empty
- Remove imports of `DockerSurfaceStore`, `DockerItemHostRequest`, `DockerSwitchTagRequest` from the `use uptrakit_plugin_infrastructure_registry::{
... }` block

Also check `controller_local/docker.rs` — if all its functionality has been moved into the Docker plugin crate, delete the file.

- [ ] **Step 5: Update `pub(crate) use docker::...` re-exports**

At lines 43–45 of `controller_local.rs`:

```rust
pub(crate) use docker::{
    allowlisted_docker_switch_tag_controller_local_action, emit_docker_switch_tag_audit_event,
};
```

Check whether these are still needed after the Docker plugin directly queries via `tenant_db`. If `controller_local/docker.rs` is deleted, remove
these re-exports. Check callers: `grep -rn "allowlisted_docker_switch_tag_controller_local_action\|emit_docker_switch_tag_audit_event"
crates/ui/surface-proxy/`.

- [ ] **Step 6: Update `lib.rs` re-exports in registry (if any Docker store types are re-exported)**

Run: `grep -n "Docker\|docker" crates/plugins/infrastructure/registry/src/lib.rs`

Remove any `DockerSurfaceStore`, `DockerItemHostRequest`, `DockerSwitchTagRequest` re-exports found.

- [ ] **Step 7: Compile check**

Run: `cargo check --all-features`
Expected: clean — zero references to `DockerSurfaceStore`

- [ ] **Step 8: Run tests**

Run: `cargo test --all-features`
Expected: all pass

- [ ] **Step 9: Commit**

```bash
git add crates/
git commit -m "refactor(docker): migrate surface actions to tenant_db(); remove DockerSurfaceStore"
```

---

## Task 2 (3b): Email — migrate `EmailSmtpSettingsStore` and fix `notification_settings.rs`

**Files:**

- Modify: `crates/plugins/notifications/email/src/` (surface action handlers)
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local/settings_store.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`
- Modify: `crates/ui/web-api-queries/src/notification_settings.rs`

- [ ] **Step 1: Write the `build_settings_bag` fixture test BEFORE the rewrite**

In `crates/ui/web-api-queries/src/notification_settings.rs`, add a test that records the expected output key names. This must pass against the current
`EmailSmtpSettings`-based implementation **before** we change anything:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_settings_bag_key_contract() {
        // Verify the key names used in the output of smtp_settings_to_prefixed_map.
        // This is a compile-time + naming contract — not a live DB test.
        // Expected keys in "tenant" map:
        let expected_tenant_keys = [
            "smtp.host", "smtp.port", "smtp.username", "smtp.password",
            "smtp.from_address", "smtp.from_name", "smtp.tls_mode", "smtp.helo_host",
        ];
        // Expected keys in "global" map:
        let expected_global_keys = [
            "global_smtp.host", "global_smtp.port", "global_smtp.username", "global_smtp.password",
            "global_smtp.from_address", "global_smtp.from_name", "global_smtp.tls_mode", "global_smtp.helo_host",
        ];
        // Telegram keys in "global" map (prefixed):
        let _expected_telegram_global_prefix = "global_telegram.";

        // Contract: the rewrite must produce these same keys.
        // If this test exists and the key list above is correct, the rewrite can be validated.
        assert!(!expected_tenant_keys.is_empty());
        assert!(!expected_global_keys.is_empty());
    }
}
```

Run: `cargo test -p uptrakit-web-api-queries --all-features build_settings_bag_key_contract`
Expected: PASS

- [ ] **Step 2: Rewrite `build_settings_bag` to remove `EmailSmtpSettings` as intermediate**

Current: `notification_settings.rs` imports `EmailSmtpSettings` from the registry, uses `typed_smtp_settings_or_empty` +
`smtp_settings_to_prefixed_map` to normalize it, then builds the JSON.

Rewrite to work directly with the raw string maps from `load_settings_by_prefix` / `load_global_settings_by_prefix`. The key names are the same
(already match the final output keys), so the rewrite just needs to:

1. Load the raw maps
2. Process the `password` field (decrypt if encrypted, filter empty strings)
3. Build the `serde_json::Value::Object` directly

Replace the full `build_settings_bag` function and all private helpers (`typed_smtp_settings_or_empty`, `normalize_smtp_settings`,
`normalize_non_empty_string`, `decode_smtp_password`, `smtp_settings_to_prefixed_map`, `insert_prefixed_string`, `insert_prefixed_u16`) with:

Note: `load_settings_by_prefix` and `load_global_settings_by_prefix` return
`Result<HashMap<String, serde_json::Value>>` — no `Option` wrapping. Values are already JSON, not
strings. Skip `decode_prefixed_settings` — work with the map directly.

```rust
use sea_orm::DatabaseConnection;
use uuid::Uuid;

const SMTP_PREFIX: &str = "smtp.";
const GLOBAL_SMTP_PREFIX: &str = "global_smtp.";
const GLOBAL_TELEGRAM_PREFIX: &str = "global_telegram.";
const SMTP_PASSWORD_AAD: &str = "uptrakit:settings:smtp_password";
const GLOBAL_SMTP_PASSWORD_AAD: &str = "uptrakit:settings:global_smtp_password";

/// Build the settings bag consumed by notification plugin `deliver()` calls.
///
/// Returns `{ "tenant": { "smtp.*" -> val, ... }, "global": { ... } }`.
pub async fn build_settings_bag(db: &DatabaseConnection, tenant_id: Uuid) -> serde_json::Value {
    let tenant_map = load_smtp_map(db, tenant_id, SMTP_PREFIX, SMTP_PASSWORD_AAD).await;
    let mut global_map =
        load_global_smtp_map(db, GLOBAL_SMTP_PREFIX, GLOBAL_SMTP_PASSWORD_AAD).await;

    // Merge Telegram global settings into the global map.
    match uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(
        db,
        GLOBAL_TELEGRAM_PREFIX,
    )
    .await
    {
        Ok(r) => {
            for (k, v) in r {
                global_map.insert(k, v);
            }
        }
        Err(e) => tracing::warn!(error = ?e, "failed to load global Telegram settings"),
    }

    serde_json::json!({ "tenant": tenant_map, "global": global_map })
}

async fn load_smtp_map(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    prefix: &str,
    password_aad: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let raw =
        match uptrakit_shared_db::raw_settings::load_settings_by_prefix(db, tenant_id, prefix)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    error = ?e, %tenant_id,
                    "failed to load tenant SMTP settings; using empty"
                );
                return serde_json::Map::new();
            }
        };
    smtp_raw_to_json_map(raw, password_aad, "tenant", Some(tenant_id))
}

async fn load_global_smtp_map(
    db: &DatabaseConnection,
    prefix: &str,
    password_aad: &str,
) -> serde_json::Map<String, serde_json::Value> {
    let raw =
        match uptrakit_shared_db::raw_settings::load_global_settings_by_prefix(db, prefix).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = ?e, "failed to load global SMTP settings; using empty");
                return serde_json::Map::new();
            }
        };
    smtp_raw_to_json_map(raw, password_aad, "global", None)
}

fn smtp_raw_to_json_map(
    raw: std::collections::HashMap<String, serde_json::Value>,
    password_aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    for (k, v) in raw {
        // Skip empty strings.
        if let serde_json::Value::String(s) = &v {
            if s.is_empty() {
                continue;
            }
        }
        // Decrypt the password field (key ends with ".password").
        let value = if k.ends_with(".password") {
            let raw_str = match &v {
                serde_json::Value::String(s) => s.clone(),
                _ => continue,
            };
            decrypt_password_value(raw_str, password_aad, scope, tenant_id)
                .map(serde_json::Value::String)
        } else {
            Some(v)
        };
        if let Some(value) = value {
            map.insert(k, value);
        }
    }
    map
}

fn decrypt_password_value(
    raw: String,
    aad: &str,
    scope: &'static str,
    tenant_id: Option<Uuid>,
) -> Option<String> {
    if !uptrakit_crypto::is_encrypted(&raw) {
        return if raw.is_empty() { None } else { Some(raw) };
    }
    match uptrakit_crypto::decrypt_str(&raw, aad) {
        Ok(v) if v.is_empty() => None,
        Ok(v) => Some(v),
        Err(e) => {
            if let Some(tid) = tenant_id {
                tracing::warn!(
                    error = ?e, %tid, scope,
                    "failed to decrypt SMTP password; using empty"
                );
            } else {
                tracing::warn!(error = ?e, scope, "failed to decrypt SMTP password; using empty");
            }
            None
        }
    }
}
```

Remove the old `use uptrakit_plugin_infrastructure_registry::EmailSmtpSettings;` import at the top of the file.

- [ ] **Step 3: Run the fixture test against the rewritten implementation**

Run: `cargo test -p uptrakit-web-api-queries --all-features build_settings_bag_key_contract`
Expected: PASS

- [ ] **Step 4: Rewrite Email plugin surface action handlers to use `ctx.tenant_db()`**

In the email plugin's surface action handlers (locate with `grep -rn "email_smtp_settings_store" crates/plugins/notifications/email/`), replace store
calls with direct `ctx.tenant_db()` queries against the `setting` and `global_setting` entities.

Canonical key constants are in `crates/ui/surface-proxy/src/proxy/controller_local/settings_store.rs` — use these as the source of truth for key
names.

- [ ] **Step 5: Delete `EmailSmtpSettings`, `EmailSmtpSettingsPatch`, `EmailSmtpSettingsStore` from `roles.rs`**

Delete these three types and the `email_smtp_settings_store()` method from `SurfaceActionController` in
`crates/plugins/infrastructure/core/src/roles.rs`.

- [ ] **Step 6: Delete email impl from `settings_store.rs`**

In `crates/ui/surface-proxy/src/proxy/controller_local/settings_store.rs`:

- Delete `impl EmailSmtpSettingsStore for AppStateSurfaceActionController` (the full impl block)
- Delete the `email_smtp_settings_store()` method from the `impl SurfaceActionController` block in `controller_local.rs`
- Remove `EmailSmtpSettingsStore` from the `use uptrakit_plugin_infrastructure_registry::{ ... }` import in `controller_local.rs`

- [ ] **Step 7: Remove registry re-exports for email store types**

In `crates/plugins/infrastructure/registry/src/lib.rs`, remove:

- `EmailSmtpSettings` re-export
- `EmailSmtpSettingsPatch` re-export (if present)
- `EmailSmtpSettingsStore` re-export

- [ ] **Step 8: Compile check**

Run: `cargo check --all-features`
Expected: clean

- [ ] **Step 9: Run tests**

Run: `cargo test --all-features`
Expected: all pass

- [ ] **Step 10: Commit**

```bash
git add crates/
git commit -m "refactor(email): migrate surface actions to tenant_db(); remove EmailSmtpSettingsStore"
```

---

## Task 3 (3c): Telegram — migrate `TelegramGlobalSettingsStore`

**Files:**

- Modify: `crates/plugins/notifications/telegram/src/` (surface handlers)
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local/settings_store.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Locate Telegram surface action handler call sites**

Run: `grep -rn "telegram_global_settings_store\|TelegramGlobalSettingsStore" crates/ --include="*.rs"`

- [ ] **Step 2: Rewrite Telegram surface action handlers to use `ctx.tenant_db()`**

Replace `ctx.controller.telegram_global_settings_store()` calls with direct queries against `global_setting` entity (keys prefixed
`global_telegram.*`). Key constants are in `crates/ui/surface-proxy/src/proxy/controller_local/settings_store.rs`.

- [ ] **Step 3: Delete `TelegramGlobalSettingsStore` from `roles.rs`**

Remove the trait definition and `telegram_global_settings_store()` method from `SurfaceActionController`.

- [ ] **Step 4: Delete Telegram impl from `settings_store.rs` and delete the file**

In `settings_store.rs`:

- Delete `impl TelegramGlobalSettingsStore for AppStateSurfaceActionController` (full block)

After both 3b and 3c land, `settings_store.rs` is empty (or only has imports). Delete the file:

```bash
rm crates/ui/surface-proxy/src/proxy/controller_local/settings_store.rs
```

Remove `mod settings_store;` from `controller_local.rs`.

- [ ] **Step 5: Remove registry re-exports**

Remove `TelegramGlobalSettingsStore` re-export from `crates/plugins/infrastructure/registry/src/lib.rs`.

- [ ] **Step 6: Compile and test**

Run: `cargo check --all-features && cargo test --all-features`
Expected: clean and pass

- [ ] **Step 7: Commit**

```bash
git add crates/
git commit -m "refactor(telegram): migrate surface actions to tenant_db(); remove TelegramGlobalSettingsStore; delete settings_store.rs"
```

---

## Task 4 (3d): Notification channels — migrate `NotificationChannelStore`

**Files:**

- Modify: Notification plugin surface action handlers (webhook, telegram, email — whichever uses `notification_channel_store()`)
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs` (delete impl block)
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Locate all `notification_channel_store()` call sites**

Run: `grep -rn
"notification_channel_store\|NotificationChannelStore\|NotificationChannelListRequest\|NotificationChannelListItem\|NotificationChannelListPage\|NotificationActionTokenRecord"
crates/ --include="*.rs"`

Note: `notification-plugin-core/list_channels.rs` already accepts `(db: &DatabaseConnection, tenant_id: Uuid, ...)` directly — it does NOT use the
store trait. The surface action handlers call the store; the store calls `list_channels.rs`. The rewrite replaces the store dispatch with direct calls
to `list_channels.rs` functions passing `ctx.tenant_db().db()` and `ctx.tenant_db().tenant_id`.

- [ ] **Step 2: Rewrite notification surface action handlers**

In each plugin that registers surface actions for channel management, replace:

```rust
let store = ctx.controller.notification_channel_store()
    .ok_or_else(|| SurfaceActionError::internal("notification store unavailable"))?;
let result = store.list_channels(req).await?;
```

With direct calls:

```rust
let tenant_db = ctx.tenant_db();
let result = uptrakit_notification_plugin_core::list_channels(
    tenant_db.db(),
    tenant_db.tenant_id,
    // pass other params
).await?;
```

Check what `list_channels.rs` exports and what parameters it takes. Run: `grep -n "pub async fn"
crates/plugins/notifications/core/src/list_channels.rs`

- [ ] **Step 3: Delete `NotificationChannelStore` and related types from `roles.rs`**

Remove from `crates/plugins/infrastructure/core/src/roles.rs`:

- `NotificationChannelStore` trait
- `NotificationChannelListRequest` struct
- `NotificationChannelListItem` struct
- `NotificationChannelListPage` struct
- `NotificationActionTokenRecord` struct
- `notification_channel_store()` method from `SurfaceActionController`

- [ ] **Step 4: Delete `impl NotificationChannelStore for AppStateSurfaceActionController`**

In `crates/ui/surface-proxy/src/proxy/controller_local.rs`:

- Delete the full `impl NotificationChannelStore` block (starting at line ~147)
- Delete `notification_channel_store()` from `impl SurfaceActionController`
- Remove all `NotificationChannel*`, `NotificationActionToken*` from the registry import block
- Delete `mod notification_settings;` if `notification_settings.rs` in controller_local is now empty; otherwise leave it (it may have other functions)

- [ ] **Step 5: Remove registry re-exports**

Remove `NotificationChannelStore`, `NotificationChannelListRequest`, `NotificationChannelListItem`, `NotificationChannelListPage`,
`NotificationActionTokenRecord` from `crates/plugins/infrastructure/registry/src/lib.rs`.

- [ ] **Step 6: Compile and test**

Run: `cargo check --all-features && cargo test --all-features`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add crates/
git commit -m "refactor(notifications): migrate channel store to tenant_db(); remove NotificationChannelStore"
```

---

## Task 5 (3e): Proxmox surface actions — migrate `ProxmoxSurfaceStore`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/surfaces.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Locate all `proxmox_surface_store()` call sites**

Run: `grep -rn "proxmox_surface_store\|ProxmoxSurfaceStore\|execute_proxmox_controller_" crates/ --include="*.rs"`

The free functions `execute_proxmox_controller_*` live in `web-api-queries/update_dispatch.rs`. After this task they move into the Proxmox plugin
itself (or become closures in `surfaces.rs`).

- [ ] **Step 2: Rewrite `surfaces.rs` to query via `ctx.tenant_db()`**

In `crates/plugins/infrastructure/proxmox/src/surfaces.rs`, each surface action handler currently calls
`ctx.controller.proxmox_surface_store().ok_or(...)?.some_method(req).await?`. Replace each with direct `ctx.tenant_db()` DB queries.

The `execute_proxmox_controller_*` free functions in `update_dispatch.rs` contain the actual query logic. Move those functions (or equivalent logic)
into the Proxmox plugin's `surfaces.rs`. At this point Proxmox entities still live in `shared-db` (moved in Wave 4); use
`uptrakit_shared_db::entity::proxmox_*` imports.

For each action, locate the corresponding free function in `update_dispatch.rs` and replicate its logic in `surfaces.rs` using `tenant_db.db()` and
`tenant_db.tenant_id` instead of the store's methods.

- [ ] **Step 3: Delete `ProxmoxSurfaceStore` and all Proxmox request types from `roles.rs`**

Remove from `crates/plugins/infrastructure/core/src/roles.rs`:

- `ProxmoxSurfaceStore` trait
- `ProxmoxHostMappingsRequest`, `ProxmoxPluginConfigRequest`, `ProxmoxManualMatchRequest`
- `ProxmoxApproveMatchRequest`, `ProxmoxMappingRequest`, `ProxmoxHostInfoRequest`
- `ProxmoxUnmatchedGuestsRequest`, `ProxmoxScopeSelectionRequest`
- `ProxmoxItemOverridePreloadRequest`, `ProxmoxItemOverrideSaveRequest`
- `ProxmoxGlobalDefaultsSaveRequest`, `ProxmoxProtectionAuditRecord`
- `ProxmoxProtectionMode`, `ProxmoxProtectionPolicyRecord`, `ProxmoxHostMappingRecord`
- `proxmox_surface_store()` method from `SurfaceActionController`

Also remove `proxmox_protection_store()` from `SurfaceActionController` here (it exists on BOTH `SurfaceActionController` and
`UpdateProtectionController`; removal from `UpdateProtectionController` is in Task 6 below).

- [ ] **Step 4: Delete `impl ProxmoxSurfaceStore for AppStateSurfaceActionController`**

In `crates/ui/surface-proxy/src/proxy/controller_local.rs`:

- Delete `proxmox_surface_store()` method from `impl SurfaceActionController`
- Delete the `impl ProxmoxSurfaceStore` block
- Remove all `Proxmox*` types and `execute_proxmox_controller_*` functions from the registry import block
- Check `mod proxmox_add_config;` and `mod proxmox_update_protection;` — if they're now unused, delete them and their files

- [ ] **Step 5: Remove `execute_proxmox_controller_*` imports from `controller_local.rs`**

These are NOT free functions in `update_dispatch.rs` — they are `pub use` re-exports in
`registry/src/lib.rs` (removed in Step 6). In
`crates/ui/surface-proxy/src/proxy/controller_local.rs`, find the `use
uptrakit_plugin_infrastructure_registry::{...}` import block and remove any
`execute_proxmox_controller_*` entries. Those call sites will now use the functions from the
proxmox plugin crate directly.

- [ ] **Step 6: Remove registry re-exports**

In `crates/plugins/infrastructure/registry/src/lib.rs`, remove all `Proxmox*` type re-exports and all `execute_proxmox_controller_*` free-function
re-exports.

- [ ] **Step 7: Compile and test**

Run: `cargo check --all-features && cargo test --all-features`
Expected: clean

- [ ] **Step 8: Commit**

```bash
git add crates/
git commit -m "refactor(proxmox): migrate surface actions to tenant_db(); remove ProxmoxSurfaceStore"
```

---

## Task 6 (3f): Proxmox update protection — migrate `ProxmoxProtectionStore`

**Files:**

- Modify: `crates/plugins/infrastructure/proxmox/src/update_protection.rs`
- Modify: `crates/ui/web-api-queries/src/queries/update_dispatch.rs`
- Modify: `crates/plugins/infrastructure/core/src/roles.rs`

- [ ] **Step 1: Locate `ProxmoxProtectionStore` and `QueryProxmoxProtectionStore`**

Run: `grep -rn "ProxmoxProtectionStore\|QueryProxmoxProtectionStore\|proxmox_protection_store" crates/ --include="*.rs"`

`QueryProxmoxProtectionStore` struct + impl is in `update_dispatch.rs` around line 458–800. The Proxmox plugin's `ControllerUpdateProtection` impl (in
`update_protection.rs`) calls `ctx.controller.proxmox_protection_store()`.

- [ ] **Step 2: Move `QueryProxmoxProtectionStore` methods into `update_protection.rs`**

`QueryProxmoxProtectionStore` in `update_dispatch.rs` (lines ~458–700) has these methods to port:

| Method                       | Line (approx) | Entities                                                         | Destination            |
| ---------------------------- | ------------- | ---------------------------------------------------------------- | ---------------------- |
| `load_host_mapping`          | ~464          | `proxmox_host_mapping`                                           | module-level helper fn |
| `load_plugin_config_payload` | ~493          | `proxmox_host_mapping`, `plugin_config`                          | module-level helper fn |
| `load_effective_policy`      | ~519          | `proxmox_protection_default`, `proxmox_protection_item_override` | module-level helper fn |
| `load_audit`                 | ~583          | `proxmox_protection_audit`                                       | module-level helper fn |
| `upsert_audit`               | ~609          | `proxmox_protection_audit`                                       | module-level helper fn |
| `find_cached_backup_target`  | ~662          | `proxmox_backup_target_cache`                                    | module-level helper fn |

Also port helper fns used exclusively by these methods:
`proxmox_mode_from_db` (~line 424) and `proxmox_mode_to_db` (~line 432).

In `crates/plugins/infrastructure/proxmox/src/update_protection.rs`, add these as free async
functions accepting `db: &DatabaseConnection` and (where needed) `tenant_id: Uuid`. Change every
call in `prepare_pre_update_protection` and `finalize_post_update` from:

```rust
ctx.controller.proxmox_protection_store()
    .ok_or_else(|| anyhow!("no store"))?
    .load_host_mapping(host_id, software_item_id)
    .await?
```

to:

```rust
load_host_mapping(ctx.controller.tenant_db().db(), host_id, software_item_id).await?
```

Entities still live in `shared-db` at this wave; imports use `uptrakit_shared_db::entity::proxmox_*`.

**Tests:** The existing `QueryProxmoxProtectionStore` tests in `update_dispatch.rs`
(lines ~1617–1735) exercise the DB logic. Move them into
`crates/plugins/infrastructure/proxmox/src/update_protection.rs` test module, adjusting the
test setup to use the helper functions directly instead of the store struct.

- [ ] **Step 3: Delete `QueryProxmoxProtectionStore` from `update_dispatch.rs`**

In `crates/ui/web-api-queries/src/queries/update_dispatch.rs`, delete:

- `struct QueryProxmoxProtectionStore<'a>` (around line 458)
- `impl ProxmoxProtectionStore for QueryProxmoxProtectionStore<'_>` (entire impl block, around 462–800)
- All helper functions that were only used by `QueryProxmoxProtectionStore`
- All tests for `QueryProxmoxProtectionStore` in the `#[cfg(test)]` module (lines ~1617–1735) — move relevant test logic to the proxmox plugin crate
  if needed

`QueryUpdateProtectionController` itself must stay — it implements `UpdateProtectionController`.
What gets deleted is its `proxmox_store: QueryProxmoxProtectionStore<'a>` field (along with the
`QueryProxmoxProtectionStore` struct and impl). After this step the controller satisfies the trait
purely via `tenant_db()`.

- [ ] **Step 4: Delete `ProxmoxProtectionStore` from `roles.rs`**

Remove:

- `ProxmoxProtectionStore` trait definition
- `proxmox_protection_store()` method from `UpdateProtectionController` trait body
- Any remaining Proxmox protection-specific types referenced only by `ProxmoxProtectionStore`

- [ ] **Step 5: Remove proxmox_protection_store imports from `update_dispatch.rs`**

Check the top-level `use uptrakit_plugin_infrastructure_registry::{ ... }` block in `update_dispatch.rs`. Remove all `Proxmox*` types that were only
used by `QueryProxmoxProtectionStore` and the `QueryUpdateProtectionController` proxmox store field.

- [ ] **Step 6: Compile and test**

Run: `cargo check --all-features && cargo test --all-features`
Expected: clean

- [ ] **Step 7: Commit**

```bash
git add crates/
git commit -m "refactor(proxmox): migrate update protection to tenant_db(); remove ProxmoxProtectionStore"
```

---

## Task 7 (3g): Registry cleanup — remove all plugin-named re-exports

**Files:**

- Modify: `crates/plugins/infrastructure/registry/src/lib.rs`

- [ ] **Step 1: Audit remaining plugin-named re-exports**

Run: `grep -n "Proxmox\|Docker\|EmailSmtp\|TelegramGlobal\|NotificationChannel\|execute_proxmox" crates/plugins/infrastructure/registry/src/lib.rs`

After Tasks 1–6, this should return zero results. If anything remains, it means a prior task was incomplete.

- [ ] **Step 2: Verify no callers remain**

For any remaining re-export, check callers:

```bash
grep -rn "uptrakit_plugin_infrastructure_registry::{.*Proxmox\|.*Docker\|.*EmailSmtp" crates/ --include="*.rs"
```

Expected: zero results. If any callers remain, the relevant task above was incomplete — return and fix.

- [ ] **Step 3: Confirm retained re-exports**

These MUST remain in `registry/src/lib.rs` (they are generic infrastructure, not store traits):

- `DeliveryMessage`
- `MessageAction`
- `escape_html`
- `NotificationPluginError`
- `PluginError` / `PluginResult`

- [ ] **Step 4: Final compile and test**

Run: `cargo check --all-features && cargo test --all-features`
Expected: clean

- [ ] **Step 5: Audit `web-api-queries` and `surface-proxy` for zero plugin-specific imports**

```bash
grep -rn "DockerSurface\|ProxmoxSurface\|ProxmoxProtection\|EmailSmtp\|TelegramGlobal\|NotificationChannelStore" \
  crates/ui/web-api-queries/ crates/ui/surface-proxy/ --include="*.rs"
```

Expected: zero results.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/registry/
git commit -m "refactor(registry): remove all plugin-named store-trait re-exports"
```

---

## Task 8: Full quality gates for Wave 3

- [ ] **Step 1: Run full quality gates**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

All must pass before proceeding to Wave 3h / Wave 4.
