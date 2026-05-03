# Surface Proxy: Wire `controller_local.rs` Into the Module Tree

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task.
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire `local_executor.rs` and `tests/` into `proxy.rs`, migrate three missing action
families to `controller_local/` submodules, and delete all ~3,200 lines of inline duplicate code.

**Architecture:** Add `mod local_executor;` and `#[cfg(test)] mod tests;` to `proxy.rs`. Extract
notification-settings, docker switch-tag, and proxmox update-protection audit logic from the
inline block into new `controller_local/` submodules. Simplify `PluginSurfaceActionInvoker` to
a single `invoke` method; internalize `PluginOpsSurfaceActionInvoker` as `pub(super)`. Deletion
and wiring are performed atomically in a single commit (Task 11) after all tests are pre-staged.

**Tech Stack:** Rust / async-trait / SeaORM / uptrakit-audit-log / parking_lot::Mutex

---

## File structure

**Create:**

- `crates/ui/surface-proxy/src/proxy/controller_local/notification_settings.rs` — allowlist fn,
  `NotificationSettingsAction` enum, classify helper, emit fn
- `crates/ui/surface-proxy/src/proxy/controller_local/docker.rs` — allowlist fn, classify helper,
  emit fn
- `crates/ui/surface-proxy/src/proxy/controller_local/proxmox_update_protection.rs` — allowlist
  fn, `ProxmoxUpdateProtectionAction` enum, classify helper, emit fn, two action helpers
- `crates/ui/surface-proxy/src/proxy/tests/controller_owned/notification_settings.rs` — DB-backed
  audit tests for save_global_smtp, save_global_telegram
- `crates/ui/surface-proxy/src/proxy/tests/controller_owned/docker.rs` — DB-backed audit tests
  for switch-tag (success + error paths)
- `crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox_update_protection.rs` —
  DB-backed audit tests for save-global-defaults, save-item-overrides

**Modify:**

- `crates/ui/surface-proxy/src/proxy/controller_local/notifications.rs` — add three audit fns
  (`notification_channel_action_type`, `classify_notification_channel_error`,
  `emit_notification_channel_audit_event`); remove `#![expect(dead_code)]` and
  `#![expect(unreachable_pub)]`; remove submodule-level builder unit tests
- `crates/ui/surface-proxy/src/proxy/controller_local/proxmox_add_config.rs` — expand
  `emit_proxmox_add_config_audit_event` to handle both success and failure (add `request_params`
  - `Result` params); remove `#![expect(dead_code)]` and `#![expect(unreachable_pub)]`
- `crates/ui/surface-proxy/src/proxy/controller_local.rs` — add three `mod` declarations; add
  re-exports for new modules; remove ALL `#[expect(unused_imports)]`; remove
  `#![expect(unreachable_pub)]`; remove `#[expect(dead_code)]` from `map_surface_action_error`
- `crates/ui/surface-proxy/src/proxy/tests/controller_owned/mod.rs` — add audit helper fns
  (`test_audit_emitter`, `latest_tenant_audit_row_for_action`,
  `latest_tenant_audit_row_for_action_and_outcome`); add `mod notification_settings;`, `mod docker;`,
  `mod proxmox_update_protection;`
- `crates/ui/surface-proxy/src/proxy/tests/controller_owned/notifications.rs` — add forcing-function
  audit assertion test for `create`; add `build_notification_channel_requests_pass_config_through`
  (ported builder test); update `PluginSurfaceLocalExecutor::new` call sites (Task 11)
- `crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox.rs` — add three audit tests;
  update `PluginSurfaceLocalExecutor::new` call sites (Task 11)
- `crates/ui/surface-proxy/src/proxy/local_executor.rs` — simplify trait to single `invoke` method;
  make `PluginOpsSurfaceActionInvoker` `pub(super)`; add `plugin_ops` field; change `new(db,
invoker)` → `new(db, plugin_ops)`; implement full five-family dispatch with audit in `execute()`
- `crates/ui/surface-proxy/src/proxy.rs` — add `mod local_executor;`, re-exports, `#[cfg(test)]
mod tests;`; delete ~3,200 lines of inline duplicates (atomic with lib.rs update, Task 11)
- `crates/ui/surface-proxy/src/lib.rs` — remove `PluginOpsSurfaceActionInvoker` from pub use
- `crates/core/controller-runtime/src/lib.rs` — update constructor call site

---

## Task 1: Create `notification_settings.rs` and add audit fns to `notifications.rs`

**Files:**

- Create: `crates/ui/surface-proxy/src/proxy/controller_local/notification_settings.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local/notifications.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local/proxmox_add_config.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`

- [ ] **Step 1: Create `notification_settings.rs`**

```rust
// crates/ui/surface-proxy/src/proxy/controller_local/notification_settings.rs
use uuid::Uuid;

use super::SurfaceProxyError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum NotificationSettingsAction {
    ConfigureSmtp,
    SaveGlobalSmtp,
    SaveGlobalTelegram,
}

pub(crate) fn allowlisted_notification_settings_controller_local_action(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> Option<NotificationSettingsAction> {
    let channel_type = surface_id
        .strip_prefix("notifications.")
        .and_then(|rest| rest.split('.').next())?;
    if provider_id.strip_prefix("plugin.") != Some(channel_type) {
        return None;
    }
    match (surface_id, interaction_id) {
        ("notifications.email", "configure_smtp") => {
            Some(NotificationSettingsAction::ConfigureSmtp)
        }
        ("notifications.email.global_smtp", "save_global_smtp") => {
            Some(NotificationSettingsAction::SaveGlobalSmtp)
        }
        ("notifications.telegram.global_settings", "save_global_telegram") => {
            Some(NotificationSettingsAction::SaveGlobalTelegram)
        }
        _ => None,
    }
}

fn notification_settings_audit_action_type(
    action: NotificationSettingsAction,
) -> uptrakit_audit_log::RegisteredAuditAction {
    match action {
        NotificationSettingsAction::ConfigureSmtp => {
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE
        }
        NotificationSettingsAction::SaveGlobalSmtp
        | NotificationSettingsAction::SaveGlobalTelegram => {
            uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE
        }
    }
}

fn notification_settings_target(
    action: NotificationSettingsAction,
) -> (&'static str, &'static str) {
    match action {
        NotificationSettingsAction::ConfigureSmtp => ("tenant_setting", "smtp"),
        NotificationSettingsAction::SaveGlobalSmtp => ("global_setting", "global_smtp"),
        NotificationSettingsAction::SaveGlobalTelegram => ("global_setting", "global_telegram"),
    }
}

fn notification_settings_scope(action: NotificationSettingsAction) -> &'static str {
    match action {
        NotificationSettingsAction::ConfigureSmtp => "tenant",
        NotificationSettingsAction::SaveGlobalSmtp
        | NotificationSettingsAction::SaveGlobalTelegram => "global",
    }
}

fn notification_settings_mutation_source(action: NotificationSettingsAction) -> &'static str {
    match action {
        NotificationSettingsAction::ConfigureSmtp => {
            "surface_proxy.notification_settings.configure_smtp"
        }
        NotificationSettingsAction::SaveGlobalSmtp => {
            "surface_proxy.notification_settings.save_global_smtp"
        }
        NotificationSettingsAction::SaveGlobalTelegram => {
            "surface_proxy.notification_settings.save_global_telegram"
        }
    }
}

fn classify_notification_settings_error(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match error {
        SurfaceProxyError::SensitiveFieldRejected(_) => {
            return (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "invalid_request",
            );
        }
        SurfaceProxyError::PermissionDenied(_) => {
            return (
                uptrakit_audit_log::AuditOutcome::Denied,
                "permission_denied",
            );
        }
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        SurfaceProxyError::SchemaValidationFailed(message) => {
            let lowered = message.to_ascii_lowercase();
            if lowered.contains("required")
                || lowered.contains("invalid")
                || lowered.contains("must be")
                || lowered.contains("unknown action")
            {
                return (
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    "invalid_request",
                );
            }
            if lowered.contains("forbidden")
                || lowered.contains("not authorized")
                || lowered.contains("permission")
            {
                return (
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "permission_denied",
                );
            }
            if lowered.contains("internal server error")
                || lowered.contains("failed to")
                || lowered.contains("database")
            {
                return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
            }
        }
        _ => {}
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_notification_settings_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    action: NotificationSettingsAction,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };

    let (outcome, reason_code) = match result {
        Ok(_) => (uptrakit_audit_log::AuditOutcome::Success, None),
        Err(error) => {
            let (outcome, reason_code) = classify_notification_settings_error(error);
            (outcome, Some(reason_code))
        }
    };

    let mut requested_keys = request_params.keys().cloned().collect::<Vec<_>>();
    requested_keys.sort();

    let mut details = serde_json::json!({
        "setting_area": notification_settings_target(action).1,
        "setting_scope": notification_settings_scope(action),
        "mutation_source": notification_settings_mutation_source(action),
        "requested_keys": requested_keys,
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    let (target_type, target_id) = notification_settings_target(action);
    let builder =
        uptrakit_audit_log::AuditEntry::builder(notification_settings_audit_action_type(action))
            .tenant_scope(tenant_id)
            .actor(
                uptrakit_audit_log::AuditActorType::User,
                Some(caller_user_id),
            )
            .target(
                target_type,
                target_id.to_string(),
                Some(target_id.to_string()),
            );

    if let Ok(entry) = builder.outcome(outcome).details(details).build() {
        audit_emitter.emit_best_effort(entry);
    }
}
```

- [ ] **Step 2: Add notification channel audit fns to `notifications.rs` and remove expect attributes**

Append before the final `#[cfg(test)]` block in
`crates/ui/surface-proxy/src/proxy/controller_local/notifications.rs`:

```rust
use super::SurfaceProxyError;

pub(crate) fn notification_channel_action_type(
    interaction_id: &str,
) -> Option<uptrakit_audit_log::RegisteredAuditAction> {
    match interaction_id {
        "create" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE),
        "edit" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE),
        "delete" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE),
        "test" => Some(uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST),
        _ => None,
    }
}

fn classify_notification_channel_error(
    interaction_id: &str,
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    let message = match error {
        SurfaceProxyError::SchemaValidationFailed(message)
        | SurfaceProxyError::SensitiveFieldRejected(message)
        | SurfaceProxyError::PermissionDenied(message) => message.as_str(),
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        _ => "",
    };

    if message.contains("Channel not found") {
        return if interaction_id == "test" {
            (uptrakit_audit_log::AuditOutcome::Failed, "channel_not_found")
        } else {
            (uptrakit_audit_log::AuditOutcome::Denied, "channel_not_found")
        };
    }
    if message.contains("Channel type mismatch") {
        return (uptrakit_audit_log::AuditOutcome::Denied, "channel_type_mismatch");
    }
    if message.contains("Unsupported channel type") {
        return (uptrakit_audit_log::AuditOutcome::Failed, "unsupported_channel_type");
    }
    if message.contains("Failed to parse channel config") {
        return (uptrakit_audit_log::AuditOutcome::Failed, "channel_config_parse_failed");
    }
    if message.contains("field `")
        || message.contains("invalid")
        || message.contains("must be")
        || matches!(error, SurfaceProxyError::SensitiveFieldRejected(_))
    {
        return (uptrakit_audit_log::AuditOutcome::ValidationFailed, "invalid_request");
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_notification_channel_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    interaction_id: &str,
    channel_type: &str,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };
    let Some(action_type) = notification_channel_action_type(interaction_id) else {
        return;
    };

    let requested_id = request_params
        .get("id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_name = request_params
        .get("name")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, target_id, target_display) = match result {
        Ok(response) => {
            let target_id = response
                .get("id")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| requested_id.clone())
                .or_else(|| (interaction_id == "create").then(|| "pending".to_string()));
            let target_display = response
                .get("name")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| requested_name.clone());
            (uptrakit_audit_log::AuditOutcome::Success, None, target_id, target_display)
        }
        Err(error) => {
            let (outcome, reason_code) =
                classify_notification_channel_error(interaction_id, error);
            let target_id = requested_id
                .clone()
                .or_else(|| (interaction_id == "create").then(|| "pending".to_string()));
            (outcome, Some(reason_code), target_id, requested_name.clone())
        }
    };

    let mut details = serde_json::json!({
        "channel_type": channel_type,
        "create_source": format!("surface_proxy.notification_channel.{interaction_id}"),
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(tenant_id)
        .actor(uptrakit_audit_log::AuditActorType::User, Some(caller_user_id))
        .target_opt(Some("notification_channel".to_string()), target_id, target_display)
        .outcome(outcome)
        .details(details)
        .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}
```

Remove both `#![expect(dead_code, ...)]` and `#![expect(unreachable_pub, ...)]` from
`notifications.rs`, and change all `pub fn` / `pub async fn` declarations to `pub(crate) fn` /
`pub(crate) async fn`. Using `pub(crate)` is the correct visibility for items accessed only within
the crate — it satisfies `unreachable_pub = "deny"` without requiring a suppressor.

Remove the `#[cfg(test)] mod tests { ... }` block at the bottom of `notifications.rs` (the builder
unit tests; integration-level equivalents exist in `tests/controller_owned/notifications.rs`).

- [ ] **Step 3: Expand `emit_proxmox_add_config_audit_event` in `proxmox_add_config.rs`**

The current function takes 4 params (success-only). Replace it with the full Result-aware version
from `proxy.rs` inline. Remove both `#![expect(dead_code, ...)]` and `#![expect(unreachable_pub, ...)]`
from the top of `proxmox_add_config.rs`, and change all `pub fn` declarations to `pub(crate) fn`
(including `build_proxmox_add_config_create_request`).

```rust
// Remove both: #![expect(dead_code, ...)] and #![expect(unreachable_pub, ...)]
// Change all pub fn / pub async fn → pub(crate) fn / pub(crate) async fn
// (including build_proxmox_add_config_create_request)

// Replace the old emit_proxmox_add_config_audit_event with:
pub fn emit_proxmox_add_config_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else { return; };
    let Some(caller_user_id) = caller_user_id else { return; };
    let requested_name = request_params
        .get("name")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, error_kind, target_id, target_display, plugin_type) = match result {
        Ok(result) => {
            let Some(plugin_config_id) = result.get("id").and_then(|v| v.as_str()) else {
                return;
            };
            let config_name = result
                .get("name")
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string)
                .or_else(|| requested_name.clone());
            let plugin_type = result
                .get("plugin_type")
                .and_then(|v| v.as_str())
                .unwrap_or("infrastructure_proxmox");
            (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                None,
                Some(plugin_config_id.to_string()),
                config_name,
                plugin_type.to_string(),
            )
        }
        Err(error) => {
            let (outcome, reason_code, error_kind) = classify_proxmox_add_config_error(error);
            (
                outcome,
                Some(reason_code),
                error_kind,
                None,
                requested_name,
                "infrastructure_proxmox".to_string(),
            )
        }
    };

    let mut details = serde_json::json!({
        "plugin_type": plugin_type,
        "create_source": "surface_proxy.proxmox_add_config",
    });
    if let Some(config_name) = target_display.as_deref() {
        details["config_name"] = serde_json::json!(config_name);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }
    if let Some(error_kind) = error_kind {
        details["error_kind"] = serde_json::json!(error_kind);
    }

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE)
            .tenant_scope(tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::User, Some(caller_user_id))
            .target_opt(Some("plugin_config".to_string()), target_id, target_display)
            .outcome(outcome)
            .details(details)
            .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}

fn classify_proxmox_add_config_error(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str, Option<&'static str>) {
    match error {
        SurfaceProxyError::SchemaValidationFailed(_)
        | SurfaceProxyError::SensitiveFieldRejected(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "validation_failed",
            None,
        ),
        SurfaceProxyError::Conflict { code, .. } => {
            (uptrakit_audit_log::AuditOutcome::Failed, code, Some("conflict"))
        }
        _ => (uptrakit_audit_log::AuditOutcome::Failed, "failed", None),
    }
}
```

Add `use super::SurfaceProxyError;` to the imports in `proxmox_add_config.rs`.

- [ ] **Step 4: Update `controller_local.rs` (add mod + re-exports, remove all expect attrs)**

```rust
// Add this mod declaration alongside the existing ones:
mod notification_settings;

// Replace the three #[expect(unused_imports)] pub use blocks with pub(crate) use
// (drop #[expect(unused_imports)]; use pub(crate) so unreachable_pub lint is satisfied):
pub(crate) use notifications::{
    allowlisted_notification_channel_controller_local_action,
    emit_notification_channel_audit_event,
    execute_allowlisted_notification_channel_action,
    notification_channel_action_type,
    notification_channel_type_for_surface_id,
};
#[cfg(test)]
pub(crate) use notifications::{
    build_notification_channel_create_request, build_notification_channel_update_request,
};
pub(crate) use proxmox_add_config::{
    allowlisted_proxmox_add_config_controller_local_action,
    allowlisted_proxmox_provider,
    emit_proxmox_add_config_audit_event,
    execute_allowlisted_proxmox_add_config_action,
};
pub(crate) use notification_settings::{
    NotificationSettingsAction,
    allowlisted_notification_settings_controller_local_action,
    emit_notification_settings_audit_event,
};
// NOTE: `mod docker;` and `pub(crate) use docker::{...}` are added in Task 2.
// NOTE: `mod proxmox_update_protection;` and its re-exports are added in Task 3.

// Remove #![expect(unreachable_pub, ...)] from top of file — no longer needed since
// all re-exported items are pub(crate), not pub, so unreachable_pub won't fire.
// Remove #[expect(dead_code, ...)] from map_surface_action_error.
```

- [ ] **Step 5: Run check and commit**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --all-features
```

Expected: no errors, no new `#[expect]` annotations needed.

```bash
git add \
  crates/ui/surface-proxy/src/proxy/controller_local/notification_settings.rs \
  crates/ui/surface-proxy/src/proxy/controller_local/notifications.rs \
  crates/ui/surface-proxy/src/proxy/controller_local/proxmox_add_config.rs \
  crates/ui/surface-proxy/src/proxy/controller_local.rs
git commit -m "feat(surface-proxy): add notification_settings submodule and notification channel audit fns"
```

---

## Task 2: Create `docker.rs` submodule

**Files:**

- Create: `crates/ui/surface-proxy/src/proxy/controller_local/docker.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`

- [ ] **Step 1: Write `docker.rs`**

```rust
// crates/ui/surface-proxy/src/proxy/controller_local/docker.rs
use uuid::Uuid;

use super::SurfaceProxyError;

const PLUGIN_TYPE_RELEASES_DOCKER: &str = "releases_docker";

pub(crate) fn allowlisted_docker_switch_tag_controller_local_action(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> bool {
    matches!(provider_id, "plugin.releases_docker" | "releases_docker")
        && surface_id == "docker.item-host-actions"
        && interaction_id == "switch-tag"
}

fn classify_docker_switch_tag_error(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    let message = match error {
        SurfaceProxyError::SchemaValidationFailed(message)
        | SurfaceProxyError::SensitiveFieldRejected(message)
        | SurfaceProxyError::PermissionDenied(message) => message.as_str(),
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        _ => "",
    };

    if message.contains("missing required parameter")
        || message.contains("invalid UUID")
        || message.contains("invalid image reference")
    {
        return (uptrakit_audit_log::AuditOutcome::ValidationFailed, "invalid_request");
    }
    if message.contains("no plugin assignments found for this host")
        || message.contains("host_software_item not found for host")
    {
        return (uptrakit_audit_log::AuditOutcome::Denied, "host_assignment_not_found");
    }
    if message.contains("database error")
        || message.contains("failed to begin transaction")
        || message.contains("failed to update plugin row")
        || message.contains("failed to update host_software_item")
        || message.contains("failed to commit transaction")
    {
        return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_docker_switch_tag_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else { return; };
    let Some(caller_user_id) = caller_user_id else { return; };

    let requested_software_item_id = request_params
        .get("software_item_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_host_id = request_params
        .get("host_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_new_image_ref = request_params
        .get("new_image_ref")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code) = match result {
        Ok(_) => (uptrakit_audit_log::AuditOutcome::Success, None),
        Err(error) => {
            let (outcome, reason_code) = classify_docker_switch_tag_error(error);
            (outcome, Some(reason_code))
        }
    };

    let mut details = serde_json::json!({
        "plugin_type": PLUGIN_TYPE_RELEASES_DOCKER,
        "mutation_source": "surface_proxy.docker_switch_tag",
    });
    if let Some(host_id) = requested_host_id.as_deref() {
        details["host_id"] = serde_json::json!(host_id);
    }
    if let Some(new_image_ref) = requested_new_image_ref.as_deref() {
        details["new_image_ref"] = serde_json::json!(new_image_ref);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE)
            .tenant_scope(tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::User, Some(caller_user_id))
            .target_opt(
                Some("software_item".to_string()),
                requested_software_item_id,
                None,
            )
            .outcome(outcome)
            .details(details)
            .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}
```

- [ ] **Step 2: Update `controller_local.rs` (add mod + re-exports for docker)**

In `controller_local.rs`, add the module declaration and re-exports for docker alongside the
existing ones:

```rust
mod docker;

pub(crate) use docker::{
    allowlisted_docker_switch_tag_controller_local_action,
    emit_docker_switch_tag_audit_event,
};
```

- [ ] **Step 3: Run check and commit**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features
git add \
  crates/ui/surface-proxy/src/proxy/controller_local/docker.rs \
  crates/ui/surface-proxy/src/proxy/controller_local.rs
git commit -m "feat(surface-proxy): add docker controller_local submodule"
```

---

## Task 3: Create `proxmox_update_protection.rs` submodule

**Files:**

- Create: `crates/ui/surface-proxy/src/proxy/controller_local/proxmox_update_protection.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/controller_local.rs`

- [ ] **Step 1: Write `proxmox_update_protection.rs`**

```rust
// crates/ui/surface-proxy/src/proxy/controller_local/proxmox_update_protection.rs
use uuid::Uuid;

use super::SurfaceProxyError;

const PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX: &str = "infrastructure_proxmox";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub(crate) enum ProxmoxUpdateProtectionAction {
    SaveGlobalDefaults,
    SaveItemOverrides,
}

pub(crate) fn allowlisted_proxmox_update_protection_controller_local_action(
    surface_id: &str,
    interaction_id: &str,
) -> Option<ProxmoxUpdateProtectionAction> {
    match (surface_id, interaction_id) {
        ("proxmox.settings.update-protection", "save-global-defaults") => {
            Some(ProxmoxUpdateProtectionAction::SaveGlobalDefaults)
        }
        ("proxmox.software-item.update-protection", "save-item-overrides") => {
            Some(ProxmoxUpdateProtectionAction::SaveItemOverrides)
        }
        _ => None,
    }
}

fn proxmox_update_protection_action_type(
    action: ProxmoxUpdateProtectionAction,
) -> uptrakit_audit_log::RegisteredAuditAction {
    match action {
        ProxmoxUpdateProtectionAction::SaveGlobalDefaults => {
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE
        }
        ProxmoxUpdateProtectionAction::SaveItemOverrides => {
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE
        }
    }
}

fn proxmox_update_protection_mutation_source(
    action: ProxmoxUpdateProtectionAction,
) -> &'static str {
    match action {
        ProxmoxUpdateProtectionAction::SaveGlobalDefaults => {
            "surface_proxy.proxmox_update_protection.save_global_defaults"
        }
        ProxmoxUpdateProtectionAction::SaveItemOverrides => {
            "surface_proxy.proxmox_update_protection.save_item_overrides"
        }
    }
}

fn classify_proxmox_update_protection_error(
    error: &SurfaceProxyError,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    let message = match error {
        SurfaceProxyError::SchemaValidationFailed(message)
        | SurfaceProxyError::SensitiveFieldRejected(message)
        | SurfaceProxyError::PermissionDenied(message) => message.as_str(),
        SurfaceProxyError::Conflict { code, .. } => {
            return (uptrakit_audit_log::AuditOutcome::Failed, code);
        }
        _ => "",
    };

    if message.contains("missing required parameter")
        || message.contains("invalid UUID")
        || message.contains("invalid protection mode")
        || message.contains("invalid backup target selection")
        || message.contains("missing target key")
        || message.contains("belongs to a different Proxmox configuration")
    {
        return (uptrakit_audit_log::AuditOutcome::ValidationFailed, "invalid_request");
    }
    if message.contains("not found in tenant scope")
        || message.contains("not assigned to software item")
        || message.contains("not present in cache")
    {
        return (uptrakit_audit_log::AuditOutcome::Denied, "resource_not_available");
    }
    if message.contains("failed to save")
        || message.contains("failed to clear")
        || message.contains("database error")
    {
        return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
    }
    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

pub(crate) fn emit_proxmox_update_protection_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    action: ProxmoxUpdateProtectionAction,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else { return; };
    let Some(caller_user_id) = caller_user_id else { return; };

    let requested_plugin_config_id = request_params
        .get("plugin_config_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_software_item_id = request_params
        .get("software_item_id")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);
    let requested_mode = request_params
        .get("mode")
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, target_type, target_id, details_target_plugin_config_id) =
        match (action, result) {
            (ProxmoxUpdateProtectionAction::SaveGlobalDefaults, Ok(response)) => (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some("plugin_config".to_string()),
                response
                    .get("plugin_config_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_plugin_config_id.clone()),
                response
                    .get("plugin_config_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_plugin_config_id.clone()),
            ),
            (ProxmoxUpdateProtectionAction::SaveGlobalDefaults, Err(error)) => {
                let (outcome, reason_code) = classify_proxmox_update_protection_error(error);
                (
                    outcome,
                    Some(reason_code),
                    Some("plugin_config".to_string()),
                    requested_plugin_config_id.clone(),
                    requested_plugin_config_id.clone(),
                )
            }
            (ProxmoxUpdateProtectionAction::SaveItemOverrides, Ok(response)) => (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some("software_item".to_string()),
                response
                    .get("software_item_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_software_item_id.clone()),
                response
                    .get("plugin_config_id")
                    .and_then(|v| v.as_str())
                    .map(std::string::ToString::to_string)
                    .or_else(|| requested_plugin_config_id.clone()),
            ),
            (ProxmoxUpdateProtectionAction::SaveItemOverrides, Err(error)) => {
                let (outcome, reason_code) = classify_proxmox_update_protection_error(error);
                (
                    outcome,
                    Some(reason_code),
                    Some("software_item".to_string()),
                    requested_software_item_id.clone(),
                    requested_plugin_config_id.clone(),
                )
            }
        };

    let mut details = serde_json::json!({
        "plugin_type": PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX,
        "mutation_source": proxmox_update_protection_mutation_source(action),
    });
    if let Some(mode) = requested_mode.as_deref() {
        details["mode"] = serde_json::json!(mode);
    }
    if let Some(plugin_config_id) = details_target_plugin_config_id.as_deref() {
        details["plugin_config_id"] = serde_json::json!(plugin_config_id);
    }
    if let Ok(response) = result
        && let Some(cleared) = response.get("cleared").and_then(|v| v.as_bool())
    {
        details["cleared"] = serde_json::json!(cleared);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(proxmox_update_protection_action_type(action))
            .tenant_scope(tenant_id)
            .actor(uptrakit_audit_log::AuditActorType::User, Some(caller_user_id))
            .target_opt(target_type, target_id, None)
            .outcome(outcome)
            .details(details)
            .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}
```

- [ ] **Step 2: Update `controller_local.rs` (add mod + re-exports for proxmox_update_protection)**

In `controller_local.rs`, add the module declaration and re-exports alongside the existing ones:

```rust
mod proxmox_update_protection;

pub(crate) use proxmox_update_protection::{
    ProxmoxUpdateProtectionAction,
    allowlisted_proxmox_update_protection_controller_local_action,
    emit_proxmox_update_protection_audit_event,
};
```

- [ ] **Step 3: Run check and commit**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features
git add \
  crates/ui/surface-proxy/src/proxy/controller_local/proxmox_update_protection.rs \
  crates/ui/surface-proxy/src/proxy/controller_local.rs
git commit -m "feat(surface-proxy): add proxmox_update_protection controller_local submodule"
```

---

## Task 4: Add audit helpers to `controller_owned/mod.rs`

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/mod.rs`

These helpers are only compiled when `#[cfg(test)] mod tests;` is active in `proxy.rs` (Task 11),
but writing them now keeps Task 11 atomic.

- [ ] **Step 1: Add imports and helpers to `mod.rs`**

Add to the top of the file (alongside existing imports):

```rust
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use uptrakit_shared_db::entity::audit_log;
```

Append after the existing `insert_tenant` function:

```rust
pub(super) fn test_audit_emitter(
    db: sea_orm::DatabaseConnection,
) -> uptrakit_audit_log::AuditEmitter {
    use std::sync::Arc as StdArc;
    let backend = StdArc::new(uptrakit_audit_log::DatabaseBackend::new(db));
    let dispatcher = uptrakit_audit_log::AuditLogDispatcher::new(backend);
    uptrakit_audit_log::AuditEmitter::new(dispatcher)
}

pub(super) async fn latest_tenant_audit_row_for_action(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query tenant audit rows")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("expected tenant audit row for action {action_type}");
}

pub(super) async fn latest_tenant_audit_row_for_action_and_outcome(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .filter(audit_log::Column::Outcome.eq(outcome.as_str()))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query tenant audit rows by outcome")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("expected tenant audit row for action {action_type} with outcome {outcome}");
}
```

Add also `mod notification_settings;`, `mod docker;`, `mod proxmox_update_protection;` declarations
to `mod.rs`. These declarations reference files created in Tasks 6–8. They are safe here because
`controller_owned/mod.rs` is only compiled once `proxy.rs` declares `#[cfg(test)] mod tests;`,
which does not happen until Task 11. **Do NOT run `cargo test` between Tasks 4 and 11** — it
would attempt to compile the test tree, and the missing files would cause a hard error.

- [ ] **Step 2: Commit**

```bash
git add crates/ui/surface-proxy/src/proxy/tests/controller_owned/mod.rs
git commit -m "test(surface-proxy): add audit query helpers to controller_owned test module"
```

---

## Task 5: Add forcing-function audit test + port builder test to `notifications.rs`

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/notifications.rs`

This test will fail until Task 10 wires `emit_notification_channel_audit_event` into `execute()`.
It will not compile until proxy.rs has `#[cfg(test)] mod tests;` (Task 11). That is the forcing
function: when Task 11 runs, if the audit path is missing, this test immediately fails.

- [ ] **Step 1: Add forcing-function audit test**

Append to `crates/ui/surface-proxy/src/proxy/tests/controller_owned/notifications.rs`:

```rust
#[tokio::test]
async fn invoke_allowlisted_notification_create_emits_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_channel_registration(
            "plugin.webhook",
            "notifications.webhook",
            "create",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert("name".to_string(), serde_json::json!("Ops Hook"));
    params.insert("channel_type".to_string(), serde_json::json!("webhook"));
    params.insert("config".to_string(), serde_json::json!({"url": "https://example.invalid/hook"}));
    params.insert("enabled".to_string(), serde_json::json!(true));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.webhook".to_string(),
                interaction_id: "create".to_string(),
                idempotency_key: "idem-notification-create-audit".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(std::time::Duration::from_secs(5)),
        )
        .await
        .expect("notification create should succeed");

    assert!(response.success);

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
    )
    .await;
    assert_eq!(row.tenant_id, tenant_id());
    assert_eq!(row.outcome, uptrakit_audit_log::AuditOutcome::Success.as_str());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
    let details = row.details_json.expect("audit details");
    assert_eq!(details["channel_type"], serde_json::json!("webhook"));
    assert_eq!(
        details["create_source"],
        serde_json::json!("surface_proxy.notification_channel.create")
    );
}
```

- [ ] **Step 2: Port `build_notification_channel_requests_pass_config_through` inline test**

Append to `notifications.rs`:

```rust
#[test]
fn build_notification_channel_requests_pass_config_through() {
    let create_params = serde_json::json!({
        "name": "Email Alerts",
        "channel_type": "email",
        "config": {
            "to_addresses": ["alice@example.com", "bob@example.com"]
        },
        "enabled": true
    });
    let create_params = create_params.as_object().expect("create params should be an object");
    let create_request = build_notification_channel_create_request("email", create_params)
        .expect("create request should build");
    assert_eq!(
        create_request.config,
        serde_json::from_value::<
            uptrakit_web_api_types::notifications::channels::JsonObjectInput,
        >(serde_json::json!({
            "to_addresses": ["alice@example.com", "bob@example.com"]
        }))
        .expect("valid JsonObjectInput"),
        "config JSON object must be passed through unchanged for create"
    );

    let update_params = serde_json::json!({
        "id": uuid::Uuid::now_v7().to_string(),
        "config": {
            "to_addresses": ["carol@example.com", "dave@example.com"]
        }
    });
    let update_params = update_params.as_object().expect("update params should be an object");
    let update_request = build_notification_channel_update_request("email", update_params)
        .expect("update request should build");
    assert_eq!(
        update_request.config,
        Some(
            serde_json::from_value::<
                uptrakit_web_api_types::notifications::channels::JsonObjectInput,
            >(serde_json::json!({
                "to_addresses": ["carol@example.com", "dave@example.com"]
            }))
            .expect("valid JsonObjectInput")
        ),
        "config JSON object must be passed through unchanged for update"
    );
}
```

- [ ] **Step 3: Commit**

> **Do NOT run `cargo test` here** — the test tree is not compiled until Task 11; running it now
> produces missing-file errors.

```bash
git add crates/ui/surface-proxy/src/proxy/tests/controller_owned/notifications.rs
git commit -m "test(surface-proxy): add notification channel create audit assertion test and port builder test"
```

---

## Task 6: Port notification settings tests

**Files:**

- Create: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/notification_settings.rs`

- [ ] **Step 1: Write `notification_settings.rs`**

The inline tests use `TestPluginInvoker` and `ErrorPluginInvoker`. The external `tests.rs` exposes
`TestPluginInvoker`. For `ErrorPluginInvoker`, declare a local one in this file (the signature
matches the new `Option<&DatabaseConnection>` trait, unlike the old inline version).

````rust
// crates/ui/surface-proxy/src/proxy/tests/controller_owned/notification_settings.rs
use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::{PluginOps, SurfaceActionError};
use uptrakit_wire::surfaces;

use super::super::super::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, ServiceConnectionRegistry,
    SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError,
};
use super::super::{TestPluginInvoker, tenant_id, user_id};
use super::{ensure_master_key, setup_notification_db};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

struct ErrorPluginInvoker {
    error_message: String,
}

#[async_trait]
impl PluginSurfaceActionInvoker for ErrorPluginInvoker {
    async fn invoke(
        &self,
        _db: Option<&sea_orm::DatabaseConnection>,
        _tenant_id: Option<uuid::Uuid>,
        _caller_user_id: Option<uuid::Uuid>,
        _surface_id: &str,
        _interaction_id: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        Err(SurfaceActionError::InvalidInput(self.error_message.clone()))
    }
}

fn notification_settings_registration(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
            surfaces::Capability::MutationAction,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new(surface_id).unwrap(),
                label: "Settings".to_string(),
                priority: 100,
                slot: surfaces::SLOT_SETTINGS_BELOW_GLOBAL.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock {
                    text: "ok".to_string(),
                },
            },
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new(interaction_id).unwrap(),
                kind: surfaces::InteractionKind::FormSubmit,
                label: None,
                required_permission: None,
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            }],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

```rust
#[cfg(feature = "notifications-email")]
#[tokio::test]
async fn invoke_notifications_email_save_global_smtp_emits_global_setting_update_audit() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let seen = StdArc::new(Mutex::new(Vec::new()));
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::clone(&seen),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_settings_registration(
            "plugin.email",
            "notifications.email.global_smtp",
            "save_global_smtp",
        ))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert("host".to_string(), serde_json::json!("smtp.global.example"));
    params.insert("smtp_password".to_string(), serde_json::json!("secret-value"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.email.global_smtp".to_string(),
                interaction_id: "save_global_smtp".to_string(),
                idempotency_key: "idem-global-smtp-audit".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("save_global_smtp should succeed");
    assert!(response.success);

    {
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "notifications.email.global_smtp");
        assert_eq!(seen[0].1, "save_global_smtp");
    }

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
    )
    .await;
    assert_eq!(row.tenant_id, tenant_id());
    assert_eq!(row.outcome, uptrakit_audit_log::AuditOutcome::Success.as_str());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("global_setting"));
    assert_eq!(row.target_id.as_deref(), Some("global_smtp"));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.notification_settings.save_global_smtp")
    );
    assert_eq!(details["setting_scope"], serde_json::json!("global"));
    assert_eq!(details["setting_area"], serde_json::json!("global_smtp"));
    assert!(
        !details.to_string().contains("secret-value"),
        "audit details must never include raw secret values"
    );
}

#[cfg(feature = "notifications-telegram")]
#[tokio::test]
async fn invoke_notifications_telegram_save_global_telegram_failure_emits_failed_audit() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(ErrorPluginInvoker {
            error_message: "Internal server error".to_string(),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(notification_settings_registration(
            "plugin.telegram",
            "notifications.telegram.global_settings",
            "save_global_telegram",
        ))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert("bot_token".to_string(), serde_json::json!("123456:super-secret"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "notifications.telegram.global_settings".to_string(),
                interaction_id: "save_global_telegram".to_string(),
                idempotency_key: "idem-global-telegram-audit-failure".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("save_global_telegram should fail");
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

    let row = super::latest_tenant_audit_row_for_action_and_outcome(
        &db,
        uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
        uptrakit_audit_log::AuditOutcome::Failed,
    )
    .await;
    assert_eq!(row.tenant_id, tenant_id());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("global_setting"));
    assert_eq!(row.target_id.as_deref(), Some("global_telegram"));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.notification_settings.save_global_telegram")
    );
    assert_eq!(details["reason_code"], serde_json::json!("storage_error"));
    assert!(
        !details.to_string().contains("123456:super-secret"),
        "audit details must never include raw secret values"
    );
}
````

- [ ] **Step 2: Commit**

> **Do NOT run `cargo test` here** — the test tree is not compiled until Task 11; running it now
> produces missing-file errors.

```bash
git add crates/ui/surface-proxy/src/proxy/tests/controller_owned/notification_settings.rs
git commit -m "test(surface-proxy): port notification settings audit tests"
```

---

## Task 7: Port proxmox add-config audit tests

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox.rs`

The three inline proxmox audit tests will run against the new `execute()` path after Task 11.
They test a richer `emit_proxmox_add_config_audit_event` (success + failure), which requires
the expanded function from Task 1 Step 3.

Add this import if not already present in `proxmox.rs`:

```rust
use uptrakit_shared_db::entity::audit_log;
```

Append to `crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox.rs`
(do NOT change existing tests; these are additions):

```rust
#[tokio::test]
async fn invoke_proxmox_add_config_emits_audit_row_when_emitter_is_configured() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert("api_token".to_string(), json!("root@pam!uptrakit=secret-token"));
    params.insert("verify_tls".to_string(), json!(false));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("proxmox add-config should succeed");

    assert!(response.success);
    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(row.outcome, uptrakit_audit_log::AuditOutcome::Success.as_str());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    let details = row.details_json.expect("audit details");
    assert_eq!(details["create_source"], json!("surface_proxy.proxmox_add_config"));
    assert_eq!(details["plugin_type"], json!("infrastructure_proxmox"));
}

#[tokio::test]
async fn invoke_proxmox_add_config_validation_failure_emits_validation_failed_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert("api_token".to_string(), json!("root@pam!uptrakit=secret-token"));
    params.insert("verify_tls".to_string(), json!("definitely-not-bool"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit-validation-failed".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("invalid verify_tls should be rejected");
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .await;
    assert_eq!(row.outcome, uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str());
    let details = row.details_json.expect("audit details");
    assert_eq!(details["create_source"], json!("surface_proxy.proxmox_add_config"));
    assert_eq!(details["reason_code"], json!("validation_failed"));
}

#[tokio::test]
async fn invoke_proxmox_add_config_duplicate_conflict_emits_failed_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_hosts_registration("plugin.infrastructure_proxmox"))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = Map::new();
    params.insert("name".to_string(), json!("PVE Cluster"));
    params.insert("api_url".to_string(), json!("https://pve.local:8006"));
    params.insert("api_token".to_string(), json!("root@pam!uptrakit=secret-token"));

    proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit-conflict-first".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params: params.clone(),
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("initial create should succeed");

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.hosts".to_string(),
                interaction_id: "add-config".to_string(),
                idempotency_key: "idem-proxmox-add-config-audit-conflict-second".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("duplicate create should fail");
    assert!(matches!(err, SurfaceProxyError::Conflict { .. }));

    let row = super::latest_tenant_audit_row_for_action_and_outcome(
        &db,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        uptrakit_audit_log::AuditOutcome::Failed,
    )
    .await;
    let details = row.details_json.expect("audit details");
    assert_eq!(details["create_source"], json!("surface_proxy.proxmox_add_config"));
    assert_eq!(details["reason_code"], json!("duplicate_name"));
    assert_eq!(details["error_kind"], json!("conflict"));
}
```

- [ ] **Step 2: Commit**

> **Do NOT run `cargo test` here** — the test tree is not compiled until Task 11; running it now
> produces missing-file errors.

```bash
git add crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox.rs
git commit -m "test(surface-proxy): port proxmox add-config audit assertion tests"
```

---

## Task 8: Create `controller_owned/docker.rs`

**Files:**

- Create: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/docker.rs`

The tests use `TestPluginInvoker` (success path) and a local `ErrorPluginInvoker` (error paths).
The executor uses `new_without_database(invoker)` — docker switch-tag is Tier 2 (no direct DB
access in execute), so no database is needed for the proxy call. The DB is only needed for audit
row assertions.

- [ ] **Step 1: Write `docker.rs`**

```rust
// crates/ui/surface-proxy/src/proxy/tests/controller_owned/docker.rs
use std::sync::Arc;
use std::sync::Arc as StdArc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::SurfaceActionError;
use uptrakit_wire::surfaces;
use uuid::Uuid;

use super::super::super::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, ServiceConnectionRegistry,
    SurfaceCallerOrigin, SurfaceInvokeRequest, SurfaceProxy, SurfaceProxyError,
};
use super::super::{TestPluginInvoker, tenant_id, user_id};
use super::{ensure_master_key, setup_notification_db};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

struct ErrorPluginInvoker {
    error_message: String,
}

#[async_trait]
impl PluginSurfaceActionInvoker for ErrorPluginInvoker {
    async fn invoke(
        &self,
        _db: Option<&sea_orm::DatabaseConnection>,
        _tenant_id: Option<Uuid>,
        _caller_user_id: Option<Uuid>,
        _surface_id: &str,
        _interaction_id: &str,
        _params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        Err(SurfaceActionError::InvalidInput(self.error_message.clone()))
    }
}

fn docker_switch_tag_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
            surfaces::Capability::MutationAction,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new("docker.item-host-actions").unwrap(),
                label: "Docker Actions".to_string(),
                priority: 100,
                slot: "software.actions".to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock { text: "ok".to_string() },
            },
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new("switch-tag").unwrap(),
                kind: surfaces::InteractionKind::MutationAction,
                label: None,
                required_permission: None,
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            }],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

#[tokio::test]
async fn invoke_docker_switch_tag_success_emits_software_item_update_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let software_item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let seen = StdArc::new(Mutex::new(Vec::new()));
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::clone(&seen),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert("software_item_id".to_string(), serde_json::json!(software_item_id.to_string()));
    params.insert("host_id".to_string(), serde_json::json!(host_id.to_string()));
    params.insert("new_image_ref".to_string(), serde_json::json!("ghcr.io/example/app:26.2.6"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "docker.item-host-actions".to_string(),
                interaction_id: "switch-tag".to_string(),
                idempotency_key: "idem-docker-switch-tag-success".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("switch-tag should succeed");

    assert!(response.success);
    {
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "docker.item-host-actions");
        assert_eq!(seen[0].1, "switch-tag");
    }

    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
    )
    .await;
    assert_eq!(row.outcome, uptrakit_audit_log::AuditOutcome::Success.as_str());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(row.target_id.as_deref(), Some(software_item_id.to_string().as_str()));
    let details = row.details_json.expect("audit details");
    assert_eq!(details["mutation_source"], serde_json::json!("surface_proxy.docker_switch_tag"));
    assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
    assert_eq!(details["new_image_ref"], serde_json::json!("ghcr.io/example/app:26.2.6"));
}

#[tokio::test]
async fn invoke_docker_switch_tag_invalid_image_emits_validation_failed_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let software_item_id = Uuid::now_v7();
    let host_id = Uuid::now_v7();
    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new_without_database(Arc::new(ErrorPluginInvoker {
            error_message: "invalid image reference: bad tag".to_string(),
        }))
        .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();
    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
        .expect("plugin registration should succeed");

    let mut params = serde_json::Map::new();
    params.insert("software_item_id".to_string(), serde_json::json!(software_item_id.to_string()));
    params.insert("host_id".to_string(), serde_json::json!(host_id.to_string()));
    params.insert("new_image_ref".to_string(), serde_json::json!("bad ref"));

    let err = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "docker.item-host-actions".to_string(),
                interaction_id: "switch-tag".to_string(),
                idempotency_key: "idem-docker-switch-tag-invalid".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect_err("switch-tag should fail");
    assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

    let row = super::latest_tenant_audit_row_for_action_and_outcome(
        &db,
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
        uptrakit_audit_log::AuditOutcome::ValidationFailed,
    )
    .await;
    assert_eq!(row.target_type.as_deref(), Some("software_item"));
    assert_eq!(row.target_id.as_deref(), Some(software_item_id.to_string().as_str()));
    let details = row.details_json.expect("audit details");
    assert_eq!(details["mutation_source"], serde_json::json!("surface_proxy.docker_switch_tag"));
    assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
    assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
    assert_eq!(details["new_image_ref"], serde_json::json!("bad ref"));
}
```

- [ ] **Step 2: Commit**

> **Do NOT run `cargo test` here** — the test tree is not compiled until Task 11; running it now
> produces missing-file errors.

```bash
git add crates/ui/surface-proxy/src/proxy/tests/controller_owned/docker.rs \
        crates/ui/surface-proxy/src/proxy/tests/controller_owned/mod.rs
git commit -m "test(surface-proxy): add docker switch-tag audit tests"
```

---

## Task 9: Create `controller_owned/proxmox_update_protection.rs`

**Files:**

- Create: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox_update_protection.rs`

The proxmox update-protection tests require DB setup for the protection tables and a live
`plugin_ops` (real catalog). Use `PluginSurfaceLocalExecutor::new(db, plugin_ops)` since the
action goes through `plugin_invoker.invoke()` which needs `plugin_ops` to build the controller.
The `plugin_ops` stored in the executor is used internally by `PluginOpsSurfaceActionInvoker` to
call `handle_surface_action`.

- [ ] **Step 1: Write `proxmox_update_protection.rs`**

```rust
// crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox_update_protection.rs
use std::sync::Arc;
use std::time::Duration;

use uptrakit_plugin_infrastructure_registry::PluginOps;
use uptrakit_wire::surfaces;
use uuid::Uuid;

use super::super::super::{
    PluginSurfaceLocalExecutor, ServiceConnectionRegistry, SurfaceCallerOrigin, SurfaceInvokeRequest,
    SurfaceProxy,
};
use super::super::{tenant_id, user_id};
use super::{ensure_master_key, setup_notification_db};
use crate::registry::{SurfaceRegistry, SurfaceRegistryConfig};

fn proxmox_update_protection_registration(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> surfaces::SurfaceRegistration {
    surfaces::SurfaceRegistration {
        provider: surfaces::ProviderIdentity {
            provider_id: provider_id.to_string(),
            provider_kind: surfaces::ProviderKind::Plugin,
            provider_namespace: "plugin".to_string(),
        },
        framework_generation: surfaces::FrameworkGeneration::new(1, 0),
        capabilities: surfaces::CapabilitySet::from_capabilities([
            surfaces::Capability::TextBlockNode,
            surfaces::Capability::UniversalTargeting,
            surfaces::Capability::MutationAction,
        ]),
        effective_tenant_binding: surfaces::EffectiveTenantBinding {
            scope: surfaces::Scope::Global,
            tenant_id: None,
        },
        surfaces: vec![surfaces::RegisteredSurface {
            descriptor: surfaces::SurfaceDescriptor {
                surface_id: surfaces::SurfaceId::new(surface_id).unwrap(),
                label: "Update Protection".to_string(),
                priority: 100,
                slot: surfaces::SLOT_SETTINGS_TABS.to_string(),
                scope: surfaces::Scope::Global,
                targeting: surfaces::Targeting::Universal,
                required_permission: None,
                provider_kind: surfaces::ProviderKind::Plugin,
                required_capabilities: surfaces::CapabilitySet::from_capabilities([
                    surfaces::Capability::TextBlockNode,
                    surfaces::Capability::MutationAction,
                    surfaces::Capability::UniversalTargeting,
                ]),
                root_node: surfaces::SurfaceNode::TextBlock { text: "ok".to_string() },
            },
            interactions: vec![surfaces::InteractionDescriptor {
                interaction_id: surfaces::InteractionId::new(interaction_id).unwrap(),
                kind: surfaces::InteractionKind::FormSubmit,
                label: None,
                required_permission: None,
                input_schema: Some(surfaces::SchemaContract::Object),
                result_schema: Some(surfaces::SchemaContract::Any),
                sensitive_fields: vec![],
                timeout_seconds: Some(30),
                confirmation: None,
                transport: surfaces::InteractionTransport::ControllerLocal,
                workflow_steps: vec![],
                form_ui: None,
            }],
            data_sources: vec![],
        }],
        encryption_metadata: None,
    }
}

async fn insert_active_proxmox_plugin_config(db: &sea_orm::DatabaseConnection) -> Uuid {
    use sea_orm::{ActiveModelTrait, Set};
    use uptrakit_shared_db::entity::plugin_config;

    let id = Uuid::now_v7();
    let now = time::OffsetDateTime::now_utc();
    plugin_config::ActiveModel {
        id: Set(id),
        tenant_id: Set(tenant_id()),
        name: Set("test-proxmox".to_string()),
        plugin_type: Set("infrastructure_proxmox".to_string()),
        config: Set(uptrakit_crypto::EncryptedString::plaintext_for_test(
            r#"{"api_url":"https://pve.test:8006","api_token":"tok","verify_tls":true,"node_filter":[]}"#,
        )),
        enabled: Set(true),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(db)
    .await
    .expect("insert proxmox plugin_config");
    id
}

#[tokio::test]
async fn invoke_proxmox_save_global_defaults_emits_success_audit_row() {
    ensure_master_key();
    let db = setup_notification_db().await;
    let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
    let plugin_ops: Arc<dyn PluginOps> = Arc::new(
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
        )
        .expect("catalog should build"),
    );

    let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
    registry
        .bootstrap_plugin(proxmox_update_protection_registration(
            "plugin.infrastructure_proxmox",
            "proxmox.settings.update-protection",
            "save-global-defaults",
        ))
        .expect("plugin registration should succeed");

    let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
        PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
            .with_audit_emitter(super::test_audit_emitter(db.clone())),
    ));
    let service_connections = ServiceConnectionRegistry::new();

    let mut params = serde_json::Map::new();
    params.insert("plugin_config_id".to_string(), serde_json::json!(plugin_config_id.to_string()));
    params.insert("mode".to_string(), serde_json::json!("do_nothing"));

    let response = proxy
        .invoke(
            &service_connections,
            &registry,
            SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "proxmox.settings.update-protection".to_string(),
                interaction_id: "save-global-defaults".to_string(),
                idempotency_key: "idem-proxmox-save-global-defaults-success".to_string(),
                target_provider_id: None,
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params,
                encrypted_sensitive_params: None,
            },
            Some(Duration::from_secs(5)),
        )
        .await
        .expect("save-global-defaults should succeed");

    assert!(response.success);
    let row = super::latest_tenant_audit_row_for_action(
        &db,
        uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
    )
    .await;
    assert_eq!(row.outcome, uptrakit_audit_log::AuditOutcome::Success.as_str());
    assert_eq!(row.actor_id, Some(user_id()));
    assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
    assert_eq!(row.target_id.as_deref(), Some(plugin_config_id.to_string().as_str()));
    let details = row.details_json.expect("audit details");
    assert_eq!(
        details["mutation_source"],
        serde_json::json!("surface_proxy.proxmox_update_protection.save_global_defaults")
    );
    assert_eq!(details["plugin_type"], serde_json::json!("infrastructure_proxmox"));
}
```

- [ ] **Step 2: Commit**

> **Do NOT run `cargo test` here** — the test tree is not compiled until Task 11; running it now
> produces missing-file errors.

```bash
git add crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox_update_protection.rs \
        crates/ui/surface-proxy/src/proxy/tests/controller_owned/mod.rs
git commit -m "test(surface-proxy): add proxmox update-protection audit tests"
```

---

## Task 10: Refactor `local_executor.rs`

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy/local_executor.rs`

This file is NOT yet in the module tree — changes do not affect the compiled build until Task 11.
Make all changes here in one commit so Task 11 can be a clean wiring-only step.

- [ ] **Step 1: Update imports at the top of `local_executor.rs`**

Replace the existing `use super::controller_local::{...}` import block with:

```rust
use super::controller_local::{
    allowlisted_docker_switch_tag_controller_local_action,
    allowlisted_notification_channel_controller_local_action,
    allowlisted_notification_settings_controller_local_action,
    allowlisted_proxmox_add_config_controller_local_action,
    allowlisted_proxmox_provider,
    allowlisted_proxmox_update_protection_controller_local_action,
    emit_docker_switch_tag_audit_event,
    emit_notification_channel_audit_event,
    emit_notification_settings_audit_event,
    emit_proxmox_add_config_audit_event,
    emit_proxmox_update_protection_audit_event,
    execute_allowlisted_notification_channel_action,
    execute_allowlisted_proxmox_add_config_action,
    map_surface_action_error,
    notification_channel_type_for_surface_id,
};
```

- [ ] **Step 2: Simplify `PluginSurfaceActionInvoker` to a single `invoke` method**

Remove the two `invoke_allowlisted_*` default methods. The trait becomes:

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

- [ ] **Step 3: Make `PluginOpsSurfaceActionInvoker` `pub(super)`**

Change `pub struct PluginOpsSurfaceActionInvoker` and `impl PluginOpsSurfaceActionInvoker` to
`pub(super)`. Remove the `invoke_allowlisted_*` impl methods from `PluginOpsSurfaceActionInvoker`
(only `invoke` remains).

- [ ] **Step 4: Update `PluginSurfaceLocalExecutor` struct and constructors**

```rust
pub struct PluginSurfaceLocalExecutor {
    action_context_db: Option<Arc<DatabaseConnection>>,
    plugin_ops: Option<Arc<dyn PluginOps>>,
    plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>,
    audit_emitter: Option<uptrakit_audit_log::AuditEmitter>,
}

impl PluginSurfaceLocalExecutor {
    pub fn new(db: Arc<DatabaseConnection>, plugin_ops: Arc<dyn PluginOps>) -> Self {
        let plugin_invoker = Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops)));
        Self {
            action_context_db: Some(db),
            plugin_ops: Some(plugin_ops),
            plugin_invoker,
            audit_emitter: None,
        }
    }

    pub fn with_audit_emitter(mut self, audit_emitter: uptrakit_audit_log::AuditEmitter) -> Self {
        self.audit_emitter = Some(audit_emitter);
        self
    }

    #[cfg(test)]
    pub fn new_without_database(plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>) -> Self {
        Self {
            action_context_db: None,
            plugin_ops: None,
            plugin_invoker,
            audit_emitter: None,
        }
    }
}
```

Add `use uptrakit_plugin_infrastructure_registry::PluginOps;` to the imports.

- [ ] **Step 5: Replace `execute()` with the full five-family dispatch**

```rust
#[async_trait]
impl SurfaceLocalActionExecutor for PluginSurfaceLocalExecutor {
    async fn execute(
        &self,
        resolved: &crate::registry::ResolvedSurfaceAction,
        request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError> {
        if resolved.provider_kind != surfaces::ProviderKind::Plugin {
            return Err(SurfaceProxyError::SchemaValidationFailed(format!(
                "local surface transport is only implemented for plugin providers (got `{}`)",
                resolved.provider_id
            )));
        }
        if resolved.interaction.transport != surfaces::InteractionTransport::ControllerLocal {
            return Err(SurfaceProxyError::SchemaValidationFailed(format!(
                "plugin local executor only supports controller_local transport for interaction `{}`",
                resolved.interaction.interaction_id
            )));
        }

        let tenant_id = Uuid::parse_str(request.tenant_id.as_str()).map_err(|e| {
            SurfaceProxyError::SchemaValidationFailed(format!(
                "invalid tenant_id in surface action request: {e}"
            ))
        })?;
        let caller_user_id = match &request.caller_origin {
            surfaces::CallerOrigin::UserSession { user_id, .. } => {
                Some(Uuid::parse_str(user_id.as_str()).map_err(|e| {
                    SurfaceProxyError::SchemaValidationFailed(format!(
                        "invalid caller user_id in surface action request: {e}"
                    ))
                })?)
            }
            _ => None,
        };

        // --- Tier 1: Notification channel CRUD (db + plugin_ops required) ---
        if allowlisted_notification_channel_controller_local_action(
            resolved.provider_id.as_str(),
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        )
        .is_some()
        {
            let db = self.action_context_db.as_deref().ok_or_else(|| {
                SurfaceProxyError::SchemaValidationFailed(
                    "internal error: expected DatabaseConnection".to_string(),
                )
            })?;
            let plugin_ops = self.plugin_ops.as_deref().ok_or_else(|| {
                SurfaceProxyError::SchemaValidationFailed(
                    "internal error: expected PluginOps".to_string(),
                )
            })?;
            let channel_type = notification_channel_type_for_surface_id(
                resolved.descriptor.surface_id.as_str(),
            )
            .ok_or_else(|| {
                SurfaceProxyError::SchemaValidationFailed(
                    "internal error: could not determine notification channel type".to_string(),
                )
            })?;
            let tenant_db = uptrakit_web_api_queries::TenantDb::new(db.clone(), tenant_id);
            let result = execute_allowlisted_notification_channel_action(
                &tenant_db,
                plugin_ops,
                channel_type,
                resolved.interaction.interaction_id.as_str(),
                &request.params,
            )
            .await
            .map_err(SurfaceProxyError::SchemaValidationFailed);
            emit_notification_channel_audit_event(
                self.audit_emitter.as_ref(),
                caller_user_id,
                tenant_id,
                resolved.interaction.interaction_id.as_str(),
                channel_type,
                &request.params,
                result.as_ref(),
            );
            return result;
        }

        // --- Tier 1: Proxmox add-config (db + plugin_ops required) ---
        if allowlisted_proxmox_provider(resolved.provider_id.as_str())
            && allowlisted_proxmox_add_config_controller_local_action(
                resolved.descriptor.surface_id.as_str(),
                resolved.interaction.interaction_id.as_str(),
            )
        {
            let db = self.action_context_db.as_deref().ok_or_else(|| {
                SurfaceProxyError::SchemaValidationFailed(
                    "internal error: expected DatabaseConnection".to_string(),
                )
            })?;
            let plugin_ops = self.plugin_ops.as_deref().ok_or_else(|| {
                SurfaceProxyError::SchemaValidationFailed(
                    "internal error: expected PluginOps".to_string(),
                )
            })?;
            let tenant_db = uptrakit_web_api_queries::TenantDb::new(db.clone(), tenant_id);
            let result = execute_allowlisted_proxmox_add_config_action(
                &tenant_db,
                plugin_ops,
                uptrakit_shared_types::plugin_ids::INFRASTRUCTURE_PROXMOX.clone(),
                &request.params,
            )
            .await;
            emit_proxmox_add_config_audit_event(
                self.audit_emitter.as_ref(),
                caller_user_id,
                tenant_id,
                &request.params,
                result.as_ref(),
            );
            return result;
        }

        // --- Tier 2: Notification settings (plugin_invoker + audit) ---
        if let Some(action) = allowlisted_notification_settings_controller_local_action(
            resolved.provider_id.as_str(),
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        ) {
            let result = self
                .plugin_invoker
                .invoke(
                    self.action_context_db.as_deref(),
                    Some(tenant_id),
                    caller_user_id,
                    request.surface_id.as_str(),
                    request.interaction_id.as_str(),
                    serde_json::Value::Object(request.params.clone()),
                )
                .await
                .map_err(map_surface_action_error);
            emit_notification_settings_audit_event(
                self.audit_emitter.as_ref(),
                caller_user_id,
                tenant_id,
                action,
                &request.params,
                result.as_ref(),
            );
            return result;
        }

        // --- Tier 2: Docker switch-tag (plugin_invoker + audit) ---
        if allowlisted_docker_switch_tag_controller_local_action(
            resolved.provider_id.as_str(),
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        ) {
            let result = self
                .plugin_invoker
                .invoke(
                    self.action_context_db.as_deref(),
                    Some(tenant_id),
                    caller_user_id,
                    request.surface_id.as_str(),
                    request.interaction_id.as_str(),
                    serde_json::Value::Object(request.params.clone()),
                )
                .await
                .map_err(map_surface_action_error);
            emit_docker_switch_tag_audit_event(
                self.audit_emitter.as_ref(),
                caller_user_id,
                tenant_id,
                &request.params,
                result.as_ref(),
            );
            return result;
        }

        // --- Tier 2: Proxmox update-protection (plugin_invoker + audit) ---
        if let Some(action) = allowlisted_proxmox_update_protection_controller_local_action(
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        ) {
            let result = self
                .plugin_invoker
                .invoke(
                    self.action_context_db.as_deref(),
                    Some(tenant_id),
                    caller_user_id,
                    request.surface_id.as_str(),
                    request.interaction_id.as_str(),
                    serde_json::Value::Object(request.params.clone()),
                )
                .await
                .map_err(map_surface_action_error);
            emit_proxmox_update_protection_audit_event(
                self.audit_emitter.as_ref(),
                caller_user_id,
                tenant_id,
                action,
                &request.params,
                result.as_ref(),
            );
            return result;
        }

        // --- Tier 3: Generic invoke (no audit) ---
        self.plugin_invoker
            .invoke(
                self.action_context_db.as_deref(),
                Some(tenant_id),
                caller_user_id,
                request.surface_id.as_str(),
                request.interaction_id.as_str(),
                serde_json::Value::Object(request.params.clone()),
            )
            .await
            .map_err(map_surface_action_error)
    }
}
```

- [ ] **Step 6: Commit**

```bash
git add crates/ui/surface-proxy/src/proxy/local_executor.rs
git commit -m "refactor(surface-proxy): simplify PluginSurfaceActionInvoker trait and implement full five-family dispatch"
```

---

## Task 11: Wire `proxy.rs`, delete inline code, update external call sites (atomic)

**Files:**

- Modify: `crates/ui/surface-proxy/src/proxy.rs`
- Modify: `crates/ui/surface-proxy/src/lib.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/notifications.rs`
- Modify: `crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox.rs`

**This is an atomic commit.** All steps below must be completed before running `cargo check`.
Adding `mod local_executor;` while inline duplicates exist causes symbol conflicts. Adding
`#[cfg(test)] mod tests;` while test files reference `PluginOpsSurfaceActionInvoker` (now
`pub(super)`) causes compilation errors.

- [ ] **Step 1: Add module declarations and re-exports to `proxy.rs`**

At the top of `proxy.rs`, near the existing `mod controller_local;` declaration, add:

```rust
mod local_executor;
pub use local_executor::{
    PluginSurfaceActionInvoker, PluginSurfaceLocalExecutor, SurfaceLocalActionExecutor,
};
pub use controller_local::map_surface_action_error;

#[cfg(test)]
mod tests;
```

- [ ] **Step 2: Remove deferred `#![expect(unreachable_pub)]` attributes**

Now that proxy.rs will re-export the items publicly, the suppressors on the two pre-existing
submodule files are no longer needed. The three new files (`notification_settings.rs`, `docker.rs`,
`proxmox_update_protection.rs`) use `pub(crate)` throughout and never received this attribute.

- Remove `#![expect(unreachable_pub, ...)]` from the top of `controller_local/notifications.rs`
- Remove `#![expect(unreachable_pub, ...)]` from the top of `controller_local/proxmox_add_config.rs`
- Remove `#![expect(unreachable_pub, ...)]` from the top of `controller_local.rs`

These were kept in the two pre-existing files throughout Tasks 1–10 because `unreachable_pub = "deny"`
in the workspace would have rejected them earlier. After Step 1 adds the `pub use` chain in proxy.rs,
items flow all the way to lib.rs and are truly reachable.

- [ ] **Step 3: Delete all inline duplicates from `proxy.rs`**

Delete from `proxy.rs`:

- The `PluginSurfaceActionInvoker` trait definition
- The `PluginOpsSurfaceActionInvoker` struct + impl
- The `PluginSurfaceLocalExecutor` struct + impl
- The `NoopSurfaceLocalExecutor` struct + impl (it's already `pub(super)` in `local_executor.rs`)
- All 5 allowlist functions (`allowlisted_notification_channel_controller_local_action`,
  `allowlisted_notification_settings_controller_local_action`,
  `allowlisted_docker_switch_tag_controller_local_action`,
  `allowlisted_proxmox_update_protection_controller_local_action`,
  `allowlisted_proxmox_add_config_controller_local_action`)
- All execute functions (`execute_allowlisted_notification_channel_action`,
  `execute_allowlisted_proxmox_add_config_action`, and the helper
  `execute_notification_channel_test_action`)
- All audit emit/classify functions (all `emit_*` and `classify_*` functions)
- All inline helper functions duplicated from `controller_local/` (`build_notification_channel_*`,
  `build_proxmox_add_config_create_request`, `build_proxmox_config_from_params`,
  `resolve_notification_channel_config`, `resolve_proxmox_add_config`,
  `required_string_param`, `optional_string_param`, `required_uuid_param`,
  `strict_bool_param_with_default`, `strict_optional_bool_param`,
  `validate_or_reject_mismatched_channel_type`,
  `parse_csv_array_or_string_array_param`, `proxmox_verify_tls_param_with_default`,
  `notification_channel_type_from_surface`, `allowlisted_notification_channel_provider`,
  `require_notification_channel_type`, `strip_container_suffix`, `extract_container_suffix`,
  `allowlisted_proxmox_provider`, all `notification_settings_*` helper fns,
  all `proxmox_update_protection_*` helper fns,
  `PLUGIN_TYPE_RELEASES_DOCKER`, `PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX` constants)
- The `NotificationSettingsAction` and `ProxmoxUpdateProtectionAction` inline enum definitions
- The entire `#[cfg(test)] mod tests { ... }` inline block (~3,200 lines)

Do NOT delete: `SurfaceProxy` struct and impl, `SurfaceProxyError`, `SurfaceCallerOrigin`,
`SurfaceInvokeRequest`, `ServiceConnectionRegistry`, `SurfaceLocalActionExecutor` re-export,
`AppStateSurfaceActionController` re-export, or any routing/registry logic.

- [ ] **Step 4: Update `lib.rs` to remove `PluginOpsSurfaceActionInvoker`**

```rust
// crates/ui/surface-proxy/src/lib.rs
pub use proxy::{
    AppStateSurfaceActionController, PluginSurfaceActionInvoker,
    PluginSurfaceLocalExecutor, SurfaceCallerOrigin, SurfaceInvokeRequest,
    SurfaceLocalActionExecutor, SurfaceProxy, SurfaceProxyError,
    map_surface_action_error,
};
```

- [ ] **Step 5: Update `controller_owned/notifications.rs` call sites**

Replace every occurrence of:

```rust
PluginSurfaceLocalExecutor::new(
    Arc::new(db.clone()),
    Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
)
```

with:

```rust
PluginSurfaceLocalExecutor::new(Arc::new(db.clone()), Arc::clone(&plugin_ops))
```

Remove the `PluginOpsSurfaceActionInvoker` import from the file's use block.

- [ ] **Step 6: Update `controller_owned/proxmox.rs` call sites**

Same replacement as Step 4. Remove `PluginOpsSurfaceActionInvoker` import.

- [ ] **Step 7: Run all quality gates**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
cargo clippy --all-targets --no-default-features --features db-sqlite
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: all pass. `proxy.rs` has no `PluginSurfaceActionInvoker` definition. No
`PluginOpsSurfaceActionInvoker` in `lib.rs`. No inline test block. Zero `#[expect(dead_code)]`,
`#[expect(unused_imports)]`, or `#[expect(unreachable_pub)]` in `controller_local.rs` or its
submodules.

- [ ] **Step 8: Commit**

```bash
git add \
  crates/ui/surface-proxy/src/proxy.rs \
  crates/ui/surface-proxy/src/lib.rs \
  crates/ui/surface-proxy/src/proxy/tests/controller_owned/notifications.rs \
  crates/ui/surface-proxy/src/proxy/tests/controller_owned/proxmox.rs
git commit -m "feat(surface-proxy): wire local_executor and tests into proxy.rs; delete 3200-line inline block"
```

---

## Task 12: Update `controller-runtime` constructor call

**Files:**

- Modify: `crates/core/controller-runtime/src/lib.rs`

- [ ] **Step 1: Update the executor construction**

In `crates/core/controller-runtime/src/lib.rs` around line 440, replace:

```rust
uptrakit_web_api::surface_proxy::PluginSurfaceLocalExecutor::new(
    Arc::new(db_conn.clone()),
    Arc::new(
        uptrakit_web_api::surface_proxy::PluginOpsSurfaceActionInvoker::new(
            Arc::clone(&plugin_ops),
        ),
    ),
)
.with_audit_emitter(audit_emitter.clone())
```

with:

```rust
uptrakit_web_api::surface_proxy::PluginSurfaceLocalExecutor::new(
    Arc::new(db_conn.clone()),
    Arc::clone(&plugin_ops),
)
.with_audit_emitter(audit_emitter.clone())
```

Remove any import of `PluginOpsSurfaceActionInvoker` from this file if present.

- [ ] **Step 2: Run quality gates and commit**

```bash
cargo check --all-features
cargo clippy --all-targets --all-features
cargo test --all-features
```

Expected: all pass.

```bash
git add crates/core/controller-runtime/src/lib.rs
git commit -m "fix(controller-runtime): update PluginSurfaceLocalExecutor constructor to new(db, plugin_ops)"
```
