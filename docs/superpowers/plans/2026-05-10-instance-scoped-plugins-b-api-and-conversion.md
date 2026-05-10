# Instance-Scoped Plugins — Plan B: API Surface, dashboard-icons Conversion, Audit, ADR

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose the four `/api/v1/instance-plugins` endpoints, gate every existing plugin-listing endpoint with the visibility predicate from Plan A,
flip `enhancement_dashboard_icons` to `PluginScope::Instance`, emit two new audit events, and document the architectural decision in an ADR.

**Architecture:** Add `routes/instance_plugins.rs` with four `CanManageGlobalSettings`-gated handlers (list, list-all, set-enabled, upsert-config).
Define DTOs in `uptrakit-web-api-types` with `Validate` impls. Apply the predicate from `crate::visibility` at every existing plugin
listing/get/upsert/delete handler in `plugin_configs.rs` and `plugin_type_settings.rs`. Flip the dashboard-icons descriptor to `PluginScope::Instance`
with `instance_config: None` (kill switch only) — `type_settings` preserved unchanged. Add audit constants mirroring the `PLUGIN_TYPE_SETTINGS_*`
shape.

**Tech Stack:** Axum, `utoipa`, `uptrakit_audit_log::AuditEntry::builder`, SeaORM, `Validate` trait. Source of truth: spec
`docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`. Snapshot: `.superpowers/standards-snapshot.md`. Depends on Plan A merged (or
running on the same branch).

**Quality gates (final task):** identical to Plan A.

---

## File structure

| File                                                              | Status             | Responsibility                                                                                                                                    |
| ----------------------------------------------------------------- | ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/shared/audit-log/src/action_type.rs`                      | modify             | Add `INSTANCE_PLUGIN_TOGGLED`, `INSTANCE_PLUGIN_CONFIG_UPSERTED` constants + registration arrays                                                  |
| `crates/shared/web-api-types/src/instance_plugins.rs`             | create             | DTOs (`InstancePluginSummary`, `InstancePluginDetail`, `SetInstancePluginEnabledRequest`, `UpsertInstancePluginConfigRequest`) + `Validate` impls |
| `crates/shared/web-api-types/src/lib.rs`                          | modify             | `pub mod instance_plugins;` + re-exports                                                                                                          |
| `crates/ui/web-api/src/routes/instance_plugins.rs`                | create             | Four handlers + audit emission                                                                                                                    |
| `crates/ui/web-api/src/routes/mod.rs`                             | modify             | `pub mod instance_plugins;`                                                                                                                       |
| `crates/ui/web-api/src/router.rs`                                 | modify             | Register the four routes                                                                                                                          |
| `crates/ui/web-api/src/routes/plugin_configs.rs`                  | modify             | Predicate filter on `list_plugin_types`                                                                                                           |
| `crates/ui/web-api/src/routes/plugin_type_settings.rs`            | modify             | Predicate filter on list/get/upsert/**delete**                                                                                                    |
| `crates/ui/web-api/src/integration_tests/instance_plugins.rs`     | create             | End-to-end route tests                                                                                                                            |
| `crates/ui/web-api/src/integration_tests/mod.rs`                  | modify             | `mod instance_plugins;`                                                                                                                           |
| `crates/ui/web-api/src/integration_tests/plugin_configs.rs`       | modify             | Tests for hidden disabled instance plugin in `list_plugin_types`                                                                                  |
| `crates/ui/web-api/src/integration_tests/plugin_type_settings.rs` | modify (or create) | Tests for hidden disabled instance plugin in all 4 type-settings handlers                                                                         |
| Surface registry filter site                                      | modify             | Apply predicate when reading surfaces (file located in Step 1 of Task 8)                                                                          |
| `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`       | modify             | Add `scope: PluginScope::Instance` to `declare_plugin!`                                                                                           |
| `crates/plugins/enhancements/dashboard-icons/Cargo.toml`          | modify (if needed) | If `PluginScope` re-export requires updated dep                                                                                                   |
| `docs/adr/0006-instance-scoped-plugins.md`                        | create             | ADR documenting the architectural decision                                                                                                        |

---

## Task 1: Branch confirmation

**Files:** none

- [ ] **Step 1: Confirm Plan A is committed (last commit ends with controller boot wiring)**

```bash
git log --oneline -5
```

Expected: at least the Plan-A commits in history.

- [ ] **Step 2: Branch (or stay on Plan A's branch if continuing)**

```bash
git checkout -b feat/instance-scoped-plugins-api  # if separate branch desired
```

No commit.

---

## Task 2: Add audit action constants

**Files:**

- Modify: `crates/shared/audit-log/src/action_type.rs`

- [ ] **Step 1: Append the two new constants after `PLUGIN_TYPE_SETTINGS_DELETE` (around line 78)**

```rust
    pub const INSTANCE_PLUGIN_TOGGLED: RegisteredAuditAction =
        RegisteredAuditAction::new("instance_plugin.toggled");
    pub const INSTANCE_PLUGIN_CONFIG_UPSERTED: RegisteredAuditAction =
        RegisteredAuditAction::new("instance_plugin.config_upserted");
```

- [ ] **Step 2: Register in any iteration array used by registration tests (around line 263)**

Add the two new constants to the existing `&[ ... ]` literal that lists every registered action.

- [ ] **Step 3: Build + test**

```bash
cargo test -p uptrakit-audit-log --all-features
```

- [ ] **Step 4: Commit**

```bash
git add crates/shared/audit-log/src/action_type.rs
git commit -m "feat(audit): INSTANCE_PLUGIN_TOGGLED + INSTANCE_PLUGIN_CONFIG_UPSERTED

Mirror PLUGIN_TYPE_SETTINGS_* shape. Snake-cased keys: instance_plugin.toggled,
instance_plugin.config_upserted."
```

---

## Task 3: Define DTOs in `uptrakit-web-api-types`

**Files:**

- Create: `crates/shared/web-api-types/src/instance_plugins.rs`
- Modify: `crates/shared/web-api-types/src/lib.rs`

Snapshot rules: every HTTP request DTO implements `Validate`; `serde` + `utoipa::ToSchema` derive; no `Eq` on types containing `serde_json::Value`.

- [ ] **Step 1: Create the file**

```rust
//! DTOs for `/api/v1/instance-plugins`.
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use utoipa::ToSchema;
use uptrakit_shared_types::PluginTypeId;

use crate::plugin_configs::FormField;
use crate::validate::{Validate, ValidationError};

/// One row in the Instance Plugins admin section. Returned only to users
/// holding `ManageGlobalSettings`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InstancePluginSummary {
    pub plugin_type: PluginTypeId,
    pub display_name: String,
    /// Stored desired state from the `instance_plugin_setting` table.
    pub enabled: bool,
    /// Catalog snapshot from controller boot. When `enabled != running_enabled`,
    /// the UI shows a "Pending restart" badge.
    pub running_enabled: bool,
    pub has_instance_config: bool,
    pub instance_config_form_fields: Vec<FormField>,
    pub type_settings_form_fields: Vec<FormField>,
    pub current_config: serde_json::Value,
    pub updated_at: Option<OffsetDateTime>,
}

/// Detailed view (currently identical to summary; reserved for future fields
/// such as last-toggled-by, audit trail, etc.).
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InstancePluginDetail {
    #[serde(flatten)]
    pub summary: InstancePluginSummary,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetInstancePluginEnabledRequest {
    pub enabled: bool,
}

impl Validate for SetInstancePluginEnabledRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpsertInstancePluginConfigRequest {
    pub config: serde_json::Value,
}

impl Validate for UpsertInstancePluginConfigRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if !self.config.is_object() {
            return Err(ValidationError::new(
                "config",
                "must be a JSON object",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_config_rejects_non_object() {
        let req = UpsertInstancePluginConfigRequest { config: serde_json::json!(null) };
        assert!(req.validate().is_err());
        let req = UpsertInstancePluginConfigRequest { config: serde_json::json!([]) };
        assert!(req.validate().is_err());
        let req = UpsertInstancePluginConfigRequest { config: serde_json::json!("foo") };
        assert!(req.validate().is_err());
    }

    #[test]
    fn upsert_config_accepts_empty_object() {
        let req = UpsertInstancePluginConfigRequest { config: serde_json::json!({}) };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn set_enabled_always_valid() {
        assert!(SetInstancePluginEnabledRequest { enabled: true }.validate().is_ok());
        assert!(SetInstancePluginEnabledRequest { enabled: false }.validate().is_ok());
    }
}
```

(If the local `Validate` trait or `ValidationError` constructor signature differs, adapt — keep the validation logic intact.)

- [ ] **Step 2: Register in `lib.rs`**

```rust
pub mod instance_plugins;
```

Re-export the four types alongside other DTOs.

- [ ] **Step 3: Build + test**

```bash
cargo test -p uptrakit-web-api-types --all-features instance_plugins
```

Expected: 3 passed.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/web-api-types/src/instance_plugins.rs \
        crates/shared/web-api-types/src/lib.rs
git commit -m "feat(web-api-types): DTOs for /api/v1/instance-plugins

InstancePluginSummary carries running_enabled (catalog snapshot from boot)
alongside enabled (DB state) so the UI can flag restart-pending drift.
Both request DTOs implement Validate; UpsertInstancePluginConfigRequest
rejects non-object configs at the validator boundary."
```

---

## Task 4: Implement the four route handlers

**Files:**

- Create: `crates/ui/web-api/src/routes/instance_plugins.rs`
- Modify: `crates/ui/web-api/src/routes/mod.rs`

Snapshot rules: `Validate` extracted via existing `Validated<T>` extractor; `rootcause::Report` errors; `tracing::instrument(skip_all)` per existing
handler convention; OpenAPI `extensions(("x-required-permission" = ...))`; `tokio::sync::*` forbidden — pull snapshot via `Arc<ArcSwap<...>>` from
`AppState`.

- [ ] **Step 1: Create the file (skeleton — fill in concrete code in subsequent steps)**

Pattern after `crates/ui/web-api/src/routes/plugin_type_settings.rs` (it has the most similar shape).

- [ ] **Step 2: `list_instance_plugins` (GET /api/v1/instance-plugins)**

```rust
#[utoipa::path(
    get,
    path = "/api/v1/instance-plugins",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "Every instance-scoped plugin with state", body = Vec<InstancePluginSummary>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_instance_plugins(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let snapshot = state.instance_plugin_snapshot.load_full();
    let summaries: Vec<InstancePluginSummary> = state
        .plugin
        .plugin_ops
        .known_type_ids()
        .into_iter()
        .filter_map(|id| {
            let desc = state.plugin.plugin_ops.get(&id)?;
            if desc.scope != PluginScope::Instance {
                return None;
            }
            Some(build_summary(&id, desc, snapshot.as_ref(), &state.plugin.plugin_ops))
        })
        .collect();
    (StatusCode::OK, Json(summaries)).into_response()
}
```

`build_summary` is a private helper:

```rust
fn build_summary(
    id: &PluginTypeId,
    desc: &PluginDescriptor,
    snapshot: &InstancePluginSnapshot,
    ops: &dyn PluginOps,
) -> InstancePluginSummary {
    let row = snapshot.get(id.as_str());
    let stored_enabled = row.map(|r| r.enabled).unwrap_or(false);
    let running_enabled = ops.instance_enabled(id);
    let current_config = row.map(|r| r.config.clone()).unwrap_or_else(|| serde_json::json!({}));
    InstancePluginSummary {
        plugin_type: id.clone(),
        display_name: ops.display_name(id),
        enabled: stored_enabled,
        running_enabled,
        has_instance_config: desc.instance_config.is_some(),
        instance_config_form_fields: desc
            .instance_config
            .map(|ic| (ic.form_schema)())
            .unwrap_or_default()
            .into_iter()
            .map(plugin_field_to_api_field)
            .collect(),
        type_settings_form_fields: ops
            .type_settings_form_schema(id)
            .unwrap_or_default()
            .into_iter()
            .map(plugin_field_to_api_field)
            .collect(),
        current_config,
        updated_at: row.map(|r| r.updated_at),
    }
}
```

(Reuse the existing `plugin_field_to_api_field` helper — already in `plugin_configs.rs`. Make it `pub(crate)` if not already.)

- [ ] **Step 3: `get_instance_plugin` (GET /api/v1/instance-plugins/{plugin_type})**

Returns 404 when the plugin type is unknown OR when `descriptor.scope != Instance`.

```rust
#[utoipa::path(
    get,
    path = "/api/v1/instance-plugins/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "Detailed instance plugin record", body = InstancePluginDetail),
        (status = 404, description = "Unknown plugin type or not instance-scoped"),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all, fields(plugin_type = %plugin_type))]
pub async fn get_instance_plugin(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let id = PluginTypeId::new(&plugin_type);
    let Some(desc) = state.plugin.plugin_ops.get(&id) else {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    };
    if desc.scope != PluginScope::Instance {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    }
    let snapshot = state.instance_plugin_snapshot.load_full();
    let summary = build_summary(&id, desc, snapshot.as_ref(), state.plugin.plugin_ops.as_ref());
    (StatusCode::OK, Json(InstancePluginDetail { summary })).into_response()
}
```

- [ ] **Step 4: `set_instance_plugin_enabled` (PUT /api/v1/instance-plugins/{plugin_type}/enabled)**

```rust
#[utoipa::path(
    put,
    path = "/api/v1/instance-plugins/{plugin_type}/enabled",
    params(("plugin_type" = String, Path)),
    request_body = SetInstancePluginEnabledRequest,
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, body = InstancePluginSummary),
        (status = 400),
        (status = 404),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all, fields(plugin_type = %plugin_type))]
pub async fn set_instance_plugin_enabled(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    State(audit_emitter_state): State<AuditEmitterState>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<SetInstancePluginEnabledRequest>,
) -> Response {
    let id = PluginTypeId::new(&plugin_type);
    let Some(desc) = state.plugin.plugin_ops.get(&id) else {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    };
    if desc.scope != PluginScope::Instance {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    }

    let result = uptrakit_web_api_queries::instance_plugin_settings::set_enabled(
        state.db(),
        &plugin_type,
        req.enabled,
    )
    .await;

    match result {
        Ok((previous_enabled, model)) => {
            // Update in-memory snapshot for read-back consistency.
            let mut new_snapshot = (**state.instance_plugin_snapshot.load()).clone();
            new_snapshot.upsert(
                model.plugin_type_id.clone(),
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                    enabled: model.enabled,
                    config: model.config.clone(),
                    updated_at: model.updated_at,
                },
            );
            state.instance_plugin_snapshot.store(std::sync::Arc::new(new_snapshot));

            emit_toggle_audit(
                &audit_emitter_state.0,
                &user,
                api_token_id.map(|v| v.0),
                &plugin_type,
                previous_enabled,
                req.enabled,
            );

            let snapshot = state.instance_plugin_snapshot.load_full();
            let summary = build_summary(&id, desc, snapshot.as_ref(), state.plugin.plugin_ops.as_ref());
            (StatusCode::OK, Json(summary)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, plugin_type = %plugin_type, "set_enabled failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

fn emit_toggle_audit(
    emitter: &uptrakit_audit_log::AuditEmitter,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    plugin_type: &str,
    previous_enabled: Option<bool>,
    new_enabled: bool,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
    let details = serde_json::json!({
        "plugin_type": plugin_type,
        "operation": "toggle",
        "previous_enabled": previous_enabled,
        "new_enabled": new_enabled,
    });
    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_TOGGLED,
    )
    .actor(actor_type, actor_id)
    .target("instance_plugin", plugin_type.to_string(), Some(plugin_type.to_string()))
    .outcome(uptrakit_audit_log::AuditOutcome::Success)
    .details(details)
    .build()
    {
        emitter.emit_best_effort(entry);
    }
}
```

- [ ] **Step 5: `upsert_instance_plugin_config` (PUT /api/v1/instance-plugins/{plugin_type}/config)**

```rust
#[utoipa::path(
    put,
    path = "/api/v1/instance-plugins/{plugin_type}/config",
    params(("plugin_type" = String, Path)),
    request_body = UpsertInstancePluginConfigRequest,
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, body = InstancePluginSummary),
        (status = 400, description = "Plugin has no instance_config schema OR validation failed"),
        (status = 404),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all, fields(plugin_type = %plugin_type))]
pub async fn upsert_instance_plugin_config(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    State(audit_emitter_state): State<AuditEmitterState>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpsertInstancePluginConfigRequest>,
) -> Response {
    let id = PluginTypeId::new(&plugin_type);
    let Some(desc) = state.plugin.plugin_ops.get(&id) else {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    };
    if desc.scope != PluginScope::Instance {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    }
    let Some(ops) = desc.instance_config else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "This plugin has no instance configuration schema",
        );
    };
    if let Err(e) = (ops.validate)(&req.config) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid instance config: {e}"),
        );
    }

    let field_count = req.config.as_object().map(|o| o.len()).unwrap_or(0);
    let result = uptrakit_web_api_queries::instance_plugin_settings::upsert_config(
        state.db(),
        &plugin_type,
        req.config.clone(),
    )
    .await;

    match result {
        Ok(model) => {
            let mut new_snapshot = (**state.instance_plugin_snapshot.load()).clone();
            new_snapshot.upsert(
                model.plugin_type_id.clone(),
                uptrakit_web_api_queries::instance_plugin_settings::InstancePluginRow {
                    enabled: model.enabled,
                    config: model.config.clone(),
                    updated_at: model.updated_at,
                },
            );
            state.instance_plugin_snapshot.store(std::sync::Arc::new(new_snapshot));

            emit_config_upsert_audit(
                &audit_emitter_state.0,
                &user,
                api_token_id.map(|v| v.0),
                &plugin_type,
                field_count,
            );

            let snapshot = state.instance_plugin_snapshot.load_full();
            let summary = build_summary(&id, desc, snapshot.as_ref(), state.plugin.plugin_ops.as_ref());
            (StatusCode::OK, Json(summary)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, plugin_type = %plugin_type, "upsert_config failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

fn emit_config_upsert_audit(
    emitter: &uptrakit_audit_log::AuditEmitter,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    plugin_type: &str,
    config_field_count: usize,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);
    let details = serde_json::json!({
        "plugin_type": plugin_type,
        "operation": "config_upsert",
        "config_field_count": config_field_count,
    });
    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_CONFIG_UPSERTED,
    )
    .actor(actor_type, actor_id)
    .target("instance_plugin", plugin_type.to_string(), Some(plugin_type.to_string()))
    .outcome(uptrakit_audit_log::AuditOutcome::Success)
    .details(details)
    .build()
    {
        emitter.emit_best_effort(entry);
    }
}
```

- [ ] **Step 6: Register in `routes/mod.rs`**

```rust
pub mod instance_plugins;
```

- [ ] **Step 7: Build**

```bash
cargo check -p uptrakit-web-api --all-features
```

Expected: clean.

- [ ] **Step 8: Commit**

```bash
git add crates/ui/web-api/src/routes/instance_plugins.rs \
        crates/ui/web-api/src/routes/mod.rs
git commit -m "feat(web-api): /api/v1/instance-plugins handlers + audit

list, get, set-enabled, upsert-config — all gated by ManageGlobalSettings.
Audit events INSTANCE_PLUGIN_TOGGLED and INSTANCE_PLUGIN_CONFIG_UPSERTED
mirror PLUGIN_TYPE_SETTINGS_* shape. Snapshot updated in-memory after
write for read-back consistency on the same request."
```

---

## Task 5: Wire routes into the router

**Files:**

- Modify: `crates/ui/web-api/src/router.rs`

- [ ] **Step 1: Locate where `plugin_type_settings` routes are registered**

```bash
grep -n "plugin-type-settings\|plugin_type_settings" crates/ui/web-api/src/router.rs
```

- [ ] **Step 2: Append the four new route registrations next to it**

```rust
    .route(
        "/api/v1/instance-plugins",
        get(routes::instance_plugins::list_instance_plugins),
    )
    .route(
        "/api/v1/instance-plugins/{plugin_type}",
        get(routes::instance_plugins::get_instance_plugin),
    )
    .route(
        "/api/v1/instance-plugins/{plugin_type}/enabled",
        put(routes::instance_plugins::set_instance_plugin_enabled),
    )
    .route(
        "/api/v1/instance-plugins/{plugin_type}/config",
        put(routes::instance_plugins::upsert_instance_plugin_config),
    )
```

- [ ] **Step 3: Add the four `utoipa::path` references to the OpenAPI doc collector** (locate the existing collector list near
      `plugin_type_settings::list_plugin_type_settings`):

```rust
    routes::instance_plugins::list_instance_plugins,
    routes::instance_plugins::get_instance_plugin,
    routes::instance_plugins::set_instance_plugin_enabled,
    routes::instance_plugins::upsert_instance_plugin_config,
```

- [ ] **Step 4: Build + sanity-check OpenAPI export**

```bash
cargo check -p uptrakit-web-api --all-features
```

If the project has an `openapi` subcommand, run it:

```bash
cargo run -p uptrakit-controller --all-features -- openapi 2>&1 | grep instance-plugins
```

Expected: four paths printed.

- [ ] **Step 5: Commit**

```bash
git add crates/ui/web-api/src/router.rs
git commit -m "feat(web-api): register /api/v1/instance-plugins routes + OpenAPI"
```

---

## Task 6: Add predicate filter to existing plugin-listing endpoints

**Files:**

- Modify: `crates/ui/web-api/src/routes/plugin_configs.rs`
- Modify: `crates/ui/web-api/src/routes/plugin_type_settings.rs`

Snapshot rules: 404 (not 403) when filter rejects, matching existing "unknown plugin type" response shape — no existence side-channel.

- [ ] **Step 1: Patch `list_plugin_types` (around line 250 in `plugin_configs.rs`)**

After the existing permission check, before the `.map(...)` conversion, insert:

```rust
    let snapshot = state.instance_plugin_snapshot.load_full();
    let types: Vec<PluginTypeInfo> = state
        .plugin
        .plugin_ops
        .known_type_ids()
        .into_iter()
        .filter(|id| {
            state
                .plugin
                .plugin_ops
                .get(id)
                .map(|d| crate::visibility::is_plugin_visible_to_user(d, snapshot.as_ref(), &auth_user))
                .unwrap_or(false)
        })
        .map(|id| {
            // ... existing per-id PluginTypeInfo construction ...
        })
        .collect();
```

- [ ] **Step 2: Patch `list_plugin_type_settings` (around line 152 in `plugin_type_settings.rs`)**

The current handler signature has no `State<Arc<AppState>>` extractor. Add it (and `State<PluginOpsState>` if not already present), then apply the
predicate filter on the returned model list. Updated signature + body:

```rust
pub async fn list_plugin_type_settings(
    State(state): State<Arc<AppState>>,
    State(plugin_ops): State<PluginOpsState>,
    tenant_db: TenantDb,
    Extension(auth_user): Extension<AuthenticatedUser>,
) -> Response {
    if !can_view_type_settings(&auth_user) {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    match pts_queries::list_type_settings(tenant_db.db(), tenant_db.tenant_id()).await {
        Ok(models) => {
            let snapshot = state.instance_plugin_snapshot.load_full();
            let responses: Vec<PluginTypeSettingsResponse> = models
                .into_iter()
                .filter(|m| {
                    plugin_ops
                        .0
                        .get(&PluginTypeId::new(&m.plugin_type))
                        .map(|d| crate::visibility::is_plugin_visible_to_user(d, snapshot.as_ref(), &auth_user))
                        .unwrap_or(false)
                })
                .map(model_to_response)
                .collect();
            (StatusCode::OK, Json(responses)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to list plugin type settings");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}
```

(`PluginOpsState` is the existing extractor used by sibling handlers — see `plugin_configs.rs:250` for precedent. Reuse it; don't introduce a new
pattern.)

- [ ] **Step 3: Patch `get_plugin_type_settings` (around line 188)**

Add the same `State<Arc<AppState>>` + `State<PluginOpsState>` extractors to the handler signature. After fetching the model, run the predicate; if it
returns false, respond 404 with the same body as the "not found" branch:

```rust
    if let Some(desc) = plugin_ops.0.get(&PluginTypeId::new(&plugin_type))
        && !crate::visibility::is_plugin_visible_to_user(
            desc,
            state.instance_plugin_snapshot.load().as_ref(),
            &auth_user,
        )
    {
        return error_response(
            StatusCode::NOT_FOUND,
            "No settings found for this plugin type",
        );
    }
```

- [ ] **Step 4: Patch `upsert_plugin_type_settings` (around line 232)**

Add `State<Arc<AppState>>` to the handler signature (it already has `State<PluginOpsState>`). Apply the predicate before
`validate_type_settings_payload`. On reject:

```rust
    if let Some(desc) = plugin_ops.0.get(&plugin_type_id)
        && !crate::visibility::is_plugin_visible_to_user(
            desc,
            state.instance_plugin_snapshot.load().as_ref(),
            &user,
        )
    {
        return error_response(StatusCode::NOT_FOUND, "Unknown plugin type");
    }
```

(No audit emission on the predicate-reject path — same shape as `unknown_plugin_type` validation_failed audit that already exists.)

- [ ] **Step 5: Patch `delete_plugin_type_settings` (around line 314)**

Add `State<Arc<AppState>>` + `State<PluginOpsState>` extractors to the handler signature. Same predicate pattern as Step 4 — if it rejects, return 404
with `"No settings found for this plugin type"` body — closes the existence-leak via 404-vs-204 differential.

- [ ] **Step 6: Build**

```bash
cargo check -p uptrakit-web-api --all-features
```

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/routes/plugin_configs.rs \
        crates/ui/web-api/src/routes/plugin_type_settings.rs
git commit -m "feat(web-api): apply visibility predicate to plugin listings

list_plugin_types, list/get/upsert/delete_plugin_type_settings now consult
crate::visibility. Disabled instance-scoped plugins return 404 to tenants
(matching unknown-plugin-type shape) and visible to ManageGlobalSettings
holders."
```

---

## Task 7: Apply predicate at the surface registry read path

**Files:**

- Locate: surfaces registry read path. Likely `crates/ui/surface-proxy/src/registry.rs` or `crates/ui/web-api/src/surfaces.rs`. Find via:

  ```bash
  grep -rn "fn.*surfaces\|filterSurfacesByPermission\|SurfaceResponse" crates/ui --include="*.rs" | head -10
  ```

- [ ] **Step 1: Identify the request-time surface enumeration site**

Look for a function that, given a `Slot` or `tab_group`, returns `Vec<SurfaceResponse>`. The frontend calls it via
`getSurfacesBySlot('settings.tabs')`.

- [ ] **Step 2: Add predicate filter after the existing per-surface-permission filter**

```rust
    let snapshot = state.instance_plugin_snapshot.load_full();
    surfaces
        .into_iter()
        .filter(|s| {
            // existing per-surface permission filter goes here
        })
        .filter(|s| {
            let plugin_id = s.plugin_type_id.as_str();
            state
                .plugin
                .plugin_ops
                .get(&PluginTypeId::new(plugin_id))
                .map(|d| crate::visibility::is_plugin_visible_to_user(d, snapshot.as_ref(), user))
                .unwrap_or(true) // Surfaces from non-plugin sources (built-ins) pass.
        })
        .collect()
```

(Adjust to whatever the actual registry function signature looks like — the structural pattern is: load snapshot, filter by predicate, default-true
for surfaces with no associated plugin descriptor.)

- [ ] **Step 3: Build + run integration tests touching surfaces**

```bash
cargo test -p uptrakit-web-api --all-features surface
```

- [ ] **Step 4: Commit**

```bash
git add # whichever files were modified
git commit -m "feat(web-api): filter surfaces by visibility predicate

Surfaces owned by instance-disabled plugins are filtered out of /api/v1/surfaces
responses for tenant Operators."
```

---

## Task 8: Flip dashboard-icons to `PluginScope::Instance`

**Files:**

- Modify: `crates/plugins/enhancements/dashboard-icons/src/plugin.rs`

Snapshot rules: keep behavior at runtime hook unchanged (per spec §7).

- [ ] **Step 1: Patch `declare_plugin!` invocation (around line 99 in `plugin.rs`)**

```rust
declare_plugin!(DashboardIconsPlugin, DashboardIconsConfig, "enhancement_dashboard_icons", {
    display_name: "Dashboard Icons",
    family: PluginFamily::Enhancement,
    config_model: ConfigModel::None,
    scope: PluginScope::Instance,
    type_settings: true,
    roles: [SoftwareItemLifecycle],
    software_item_lifecycle: create_dashboard_icons_lifecycle,
    global_provider_consumers: ["github"],
});
```

(Add `use uptrakit_plugin_infrastructure_core::PluginScope;` to the existing `use` group at the top of the file.)

- [ ] **Step 2: Update existing descriptor test (around line 184)**

```rust
#[test]
fn descriptor_has_correct_metadata() {
    assert_eq!(DESCRIPTOR.type_id, "enhancement_dashboard_icons");
    assert_eq!(DESCRIPTOR.display_name, "Dashboard Icons");
    assert_eq!(DESCRIPTOR.family, PluginFamily::Enhancement);
    assert_eq!(DESCRIPTOR.config_model, ConfigModel::None);
    assert_eq!(DESCRIPTOR.scope, PluginScope::Instance);  // NEW
    assert!(DESCRIPTOR.instance_config.is_none());        // NEW
    assert_eq!(DESCRIPTOR.global_provider_consumers.len(), 1);
    assert_eq!(DESCRIPTOR.global_provider_consumers[0].as_str(), "github");
}
```

- [ ] **Step 3: Verify `type_settings` survives**

```rust
#[test]
fn descriptor_keeps_type_settings_for_tenant_opt_out() {
    assert!(DESCRIPTOR.type_settings.is_some());
}
```

- [ ] **Step 4: Run plugin tests**

```bash
cargo test -p uptrakit-plugin-enhancement-dashboard-icons --all-features
```

Expected: all green.

- [ ] **Step 5: Verify the controller boot still skips construction by default**

With no row in `instance_plugin_setting`, the snapshot reports `enabled = false` for `enhancement_dashboard_icons`, the catalog skips singleton
construction, no background refresh loop spawns. Confirm via:

```bash
RUST_LOG=info cargo run -p uptrakit-controller --all-features -- --master-key-file <test-key> --help 2>&1 | grep -i dashboard
```

Expected: log line "skipping instance-scoped plugin construction (disabled at boot)" with `plugin = enhancement_dashboard_icons`.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/enhancements/dashboard-icons/src/plugin.rs
git commit -m "feat(dashboard-icons): convert to PluginScope::Instance

Disabled by default (no row in instance_plugin_setting). type_settings
stays — tenant 'enabled' opt-out remains operational once an instance
owner enables the plugin. instance_config = None (kill-switch only for
v1)."
```

---

## Task 9: Verify leakage vectors checklist for dashboard-icons

**Files:** none (audit-only); produces a notes file as commit artifact

This task implements spec §6's instruction: "The plan-writing step must run this checklist for `enhancement_dashboard_icons` and document the result."
The result lives in the ADR (Task 13).

- [ ] **Step 1: Walk every checklist row from spec §6 against dashboard-icons**

Inspect each item — the spec already documents the expected answer for v1, but reviewers must re-verify:

| #   | Vector                                | Expected for dashboard-icons                                      | How to verify                                                                        |
| --- | ------------------------------------- | ----------------------------------------------------------------- | ------------------------------------------------------------------------------------ |
| 1   | HTTP plugin-type/type-settings routes | Covered by predicate (Task 6)                                     | Integration tests in Task 11                                                         |
| 2   | Surfaces registry                     | Covered by predicate (Task 7)                                     | Integration test in Task 11                                                          |
| 3   | AdminEvent SSE                        | Plugin emits no AdminEvent                                        | grep AdminEvent references in the plugin source — expect zero matches                |
| 4   | Agent-side runtime                    | Controller-only role (`SoftwareItemLifecycle`)                    | `grep` plugin's roles list — confirm no `Discoverer`/`UpdateExecutor`/etc.           |
| 5   | MQTT topics                           | Plugin doesn't publish                                            | grep MQTT references in the plugin source — expect zero matches                      |
| 6   | Audit log target rows                 | Pre-existing `enhancement_dashboard_icons` audit rows acceptable  | Document in ADR + end-user doc                                                       |
| 7   | OpenAPI schema                        | Plugin type ids not in `utoipa` schemas as enum members           | run the OpenAPI exporter and grep for `dashboard_icons` — confirm no enum membership |
| 8   | DB tables tenant can read             | `plugin_type_setting` filtered by predicate (Task 6)              | Integration test in Task 11                                                          |
| 9   | **Persisted side effects**            | `software_item.icon_url` may carry CDN URLs from prior enrichment | Known limitation per spec §7; documented in end-user doc (Plan C) and ADR            |

- [ ] **Step 2: Add a notes section under `crates/plugins/enhancements/dashboard-icons/README.md`**

If the README doesn't exist, create it. Otherwise add a section:

```markdown
## Leakage vectors checklist

This plugin is `PluginScope::Instance`. The spec's leakage checklist has been verified for this plugin:

- HTTP routes / surfaces / DB tables: gated by `crate::visibility::is_plugin_visible_to_user`.
- AdminEvent SSE / agent runtime / MQTT / OpenAPI enum: not used by this plugin.
- Audit log: pre-existing `enhancement_dashboard_icons` audit rows from before conversion may persist; tenants viewing audit logs may see them.
  Acceptable known limitation.
- Persisted side effects (`software_item.icon_url`): URLs of the form `https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/...` set by prior
  enrichment remain visible to tenants. Acceptable known limitation (no provenance column on `software_item.icon_url`); documented in the end-user
  docs.

See `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md` §6 and ADR `docs/adr/0006-instance-scoped-plugins.md`.
```

- [ ] **Step 3: Commit**

```bash
git add crates/plugins/enhancements/dashboard-icons/README.md
git commit -m "docs(dashboard-icons): record leakage vectors checklist

Per spec §6 — every channel the plugin could leak through verified for
this v1 conversion. Two known acceptable limitations documented (audit
log historical rows + CDN URLs in icon_url)."
```

---

## Task 10: Integration tests — `/api/v1/instance-plugins` handlers

**Files:**

- Create: `crates/ui/web-api/src/integration_tests/instance_plugins.rs`
- Modify: `crates/ui/web-api/src/integration_tests/mod.rs`

Snapshot rules: tests may use `unwrap()`/`expect()` (per `clippy.toml` test exemptions). Use the existing `TestApp::new` and
`fixtures::register_and_get_token` patterns.

- [ ] **Step 1: Add module declaration**

```rust
// integration_tests/mod.rs
mod instance_plugins;
```

- [ ] **Step 2: Write the test file**

Tests required (from spec §9):

```rust
//! Integration tests for /api/v1/instance-plugins.
use crate::test_harness::TestApp;
use crate::test_harness::fixtures;
use http::StatusCode;
use uptrakit_web_api_types::instance_plugins::{
    InstancePluginSummary, SetInstancePluginEnabledRequest,
    UpsertInstancePluginConfigRequest,
};

#[tokio::test]
async fn list_requires_manage_global_settings() {
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_tenant_user_token(&client).await;
    let status = client
        .get("/api/v1/instance-plugins")
        .bearer(&access_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn list_returns_all_instance_scoped_plugins_with_state() {
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_token(&client).await; // admin

    // Seed a row for dashboard-icons. The fixture writes the DB row AND
    // mutates `app.state.instance_plugin_snapshot` (ArcSwap) so the
    // GET below reads the seeded value back. The catalog snapshot
    // (frozen at TestApp::new() boot) is intentionally NOT touched —
    // running_enabled stays at its boot value (false) regardless.
    fixtures::upsert_instance_plugin_setting(&app, "enhancement_dashboard_icons", true).await;

    let resp: Vec<InstancePluginSummary> = client
        .get("/api/v1/instance-plugins")
        .bearer(&access_token)
        .send_json()
        .await;

    let dash = resp
        .iter()
        .find(|p| p.plugin_type.as_str() == "enhancement_dashboard_icons")
        .expect("dashboard-icons present");
    assert!(dash.enabled);
    // running_enabled reflects boot snapshot (false in this test app, since boot ran with
    // an empty snapshot before the seed).
    assert!(!dash.running_enabled);
}

#[tokio::test]
async fn set_enabled_persists_and_audits() {
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_token(&client).await;

    let resp: InstancePluginSummary = client
        .put_json(
            "/api/v1/instance-plugins/enhancement_dashboard_icons/enabled",
            &SetInstancePluginEnabledRequest { enabled: true },
        )
        .bearer(&access_token)
        .send_json()
        .await;
    assert!(resp.enabled);

    let row = audit_row_for_action(
        &app.db,
        uptrakit_audit_log::AuditActionType::INSTANCE_PLUGIN_TOGGLED,
    )
    .await;
    let details = row.details_json.expect("details");
    assert_eq!(details["plugin_type"], serde_json::json!("enhancement_dashboard_icons"));
    assert_eq!(details["operation"], serde_json::json!("toggle"));
    assert_eq!(details["new_enabled"], serde_json::json!(true));
    assert_eq!(details["previous_enabled"], serde_json::json!(null));
}

#[tokio::test]
async fn set_enabled_for_unknown_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_token(&client).await;
    let status = client
        .put_json(
            "/api/v1/instance-plugins/totally_made_up/enabled",
            &SetInstancePluginEnabledRequest { enabled: true },
        )
        .bearer(&access_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn set_enabled_for_tenant_scoped_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_token(&client).await;
    // Pick any tenant-scoped plugin id from the registry, e.g. package_manager_apt.
    let status = client
        .put_json(
            "/api/v1/instance-plugins/package_manager_apt/enabled",
            &SetInstancePluginEnabledRequest { enabled: true },
        )
        .bearer(&access_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upsert_config_for_kill_switch_only_plugin_returns_400() {
    // Dashboard-icons has instance_config = None. Upsert config returns 400.
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_token(&client).await;
    let status = client
        .put_json(
            "/api/v1/instance-plugins/enhancement_dashboard_icons/config",
            &UpsertInstancePluginConfigRequest { config: serde_json::json!({}) },
        )
        .bearer(&access_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upsert_config_validates_against_validate_trait() {
    // SetInstancePluginEnabledRequest is always valid; UpsertInstancePluginConfigRequest
    // rejects non-objects at the Validate boundary.
    let app = TestApp::new().await;
    let client = app.client();
    let access_token = fixtures::register_and_get_token(&client).await;
    let status = client
        .put_json(
            "/api/v1/instance-plugins/enhancement_dashboard_icons/config",
            &serde_json::json!({ "config": "not-an-object" }),
        )
        .bearer(&access_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn upsert_config_validates_against_instance_config_schema_and_persists() {
    // Spec §9: invalid payload → 400 with descriptor-level validation reason;
    // valid payload → 200, row updated, INSTANCE_PLUGIN_CONFIG_UPSERTED audit emitted.
    //
    // dashboard-icons has instance_config = None (kill-switch only) so it cannot
    // exercise the schema-validation arm. This test must use a synthetic descriptor
    // with a non-trivial InstanceConfigOps schema. Two options:
    //   (a) Add a test-only descriptor to the registry under #[cfg(test)] that
    //       declares scope = Instance + instance_config = Some(&TEST_OPS) where
    //       TEST_OPS::validate rejects payloads missing a required `theme` field;
    //   (b) Build a TestApp variant that injects a synthetic PluginOps mock.
    //
    // (a) is simpler — match the existing test-fixture pattern in
    // crates/plugins/infrastructure/registry/src/test_support.rs.
    //
    // Test body:
    // 1. Try config = {} → expect 400, body mentions the missing field.
    // 2. Try config = { "theme": "dark" } → expect 200, response carries
    //    current_config = { "theme": "dark" }.
    // 3. Poll for INSTANCE_PLUGIN_CONFIG_UPSERTED audit row, assert
    //    details["operation"] == "config_upsert", details["plugin_type"] matches,
    //    details["config_field_count"] == 1.
}

// audit_row_for_action: copy the polling helper from
// crates/ui/web-api/src/routes/plugin_type_settings.rs (lines 386-402).
```

(Implementer: add a `fixtures::upsert_instance_plugin_setting(app: &TestApp, plugin_type_id: &str, enabled: bool)` helper to the test harness module.
It must (1) call `uptrakit_web_api_queries::instance_plugin_settings::set_enabled(app.db(), plugin_type_id, enabled)` to write the DB row, and (2)
call `app.state.instance_plugin_snapshot.store(...)` with the updated snapshot so the very next request sees the seeded value. The catalog snapshot —
frozen at `TestApp::new()` boot — must NOT be mutated; `running_enabled` deliberately reflects boot state and is independent of the ArcSwap snapshot.
Mirror the shape of `fixtures::upsert_plugin_type_setting` if it exists.)

- [ ] **Step 3: Run**

```bash
cargo test -p uptrakit-web-api --all-features integration_tests::instance_plugins
```

Expected: all 8 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/ui/web-api/src/integration_tests/instance_plugins.rs \
        crates/ui/web-api/src/integration_tests/mod.rs \
        crates/ui/web-api/src/test_harness.rs # if helpers added
git commit -m "test(web-api): integration tests for /api/v1/instance-plugins

Cover: permission gate, list-with-state, set-enabled audit, 404 on
unknown id, 404 on tenant-scoped id, 400 on kill-switch-only config
upsert, 400 on Validate-failed payload."
```

---

## Task 11: Integration tests — visibility predicate filter behavior

**Files:**

- Modify: `crates/ui/web-api/src/integration_tests/plugin_configs.rs`
- Modify: `crates/ui/web-api/src/integration_tests/plugin_type_settings.rs` (or create if absent)

- [ ] **Step 1: `tenant_user_does_not_see_disabled_instance_plugin_in_plugin_types_list`**

```rust
#[tokio::test]
async fn tenant_user_does_not_see_disabled_instance_plugin_in_plugin_types_list() {
    let app = TestApp::new().await;
    let client = app.client();
    let tenant_token = fixtures::register_and_get_tenant_user_token(&client).await;
    // Note: dashboard-icons defaults to disabled (no instance_plugin_setting row).
    let resp: Vec<PluginTypeInfo> = client
        .get("/api/v1/plugin-types")
        .bearer(&tenant_token)
        .send_json()
        .await;
    assert!(resp.iter().all(|t| t.plugin_type.as_str() != "enhancement_dashboard_icons"));
}
```

- [ ] **Step 2: `tenant_user_get_plugin_type_settings_for_disabled_instance_plugin_returns_404`**

```rust
#[tokio::test]
async fn tenant_user_get_plugin_type_settings_for_disabled_instance_plugin_returns_404() {
    let app = TestApp::new().await;
    let client = app.client();
    let tenant_token = fixtures::register_and_get_tenant_user_token(&client).await;
    let status = client
        .get("/api/v1/plugin-type-settings/enhancement_dashboard_icons")
        .bearer(&tenant_token)
        .send_status()
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
```

- [ ] **Step 3: `tenant_user_put_plugin_type_settings_for_disabled_instance_plugin_returns_404`**

PUT returns 404; verify no audit row was emitted.

- [ ] **Step 4: `tenant_user_delete_plugin_type_settings_for_disabled_instance_plugin_returns_404`**

DELETE returns 404 — closes existence-leak via 404-vs-204 differential.

- [ ] **Step 5: `instance_owner_sees_disabled_instance_plugin_everywhere`**

Same calls performed by an admin user with `ManageGlobalSettings` — all return 200.

- [ ] **Step 6: Run**

```bash
cargo test -p uptrakit-web-api --all-features tenant_user_does_not_see \
                                                tenant_user_get_plugin_type_settings_for_disabled \
                                                tenant_user_put_plugin_type_settings_for_disabled \
                                                tenant_user_delete_plugin_type_settings_for_disabled \
                                                instance_owner_sees_disabled
```

Expected: 5 passed.

- [ ] **Step 7: Commit**

```bash
git add crates/ui/web-api/src/integration_tests/plugin_configs.rs \
        crates/ui/web-api/src/integration_tests/plugin_type_settings.rs
git commit -m "test(web-api): visibility predicate filter for plugin-type endpoints

Five tests covering disabled-instance hide for tenants and visible-for-
admins across list_plugin_types and all four plugin_type_settings handlers."
```

---

## Task 12: Integration test — surface filter

**Files:**

- Modify: `crates/ui/web-api/src/integration_tests/` (the file matching surfaces)

- [ ] **Step 1: Test that disabled instance plugin's surfaces are hidden from tenant**

If dashboard-icons does not currently register surfaces, skip this task. Otherwise add a test analogous to plugin-type filter tests.

- [ ] **Step 2: Run + commit (if applicable)**

---

## Task 13: Write ADR `docs/adr/0006-instance-scoped-plugins.md`

**Files:**

- Create: `docs/adr/0006-instance-scoped-plugins.md`

ADR justified per skill criteria: (1) hard to reverse — descriptor extension and table; (2) surprising without context — future readers will wonder
why two config surfaces; (3) result of real tradeoffs — restart-required vs hot-reload, dedicated table vs `global_settings`, reuse
`ManageGlobalSettings` vs new permission.

- [ ] **Step 1: Inspect the existing ADR template**

```bash
cat docs/adr/0001-web-api-decomposition-strategy.md | head -40
```

Match its structural conventions exactly.

- [ ] **Step 2: Write the ADR**

```markdown
# 0006. Instance-Scoped Plugins

Date: 2026-05-10

## Status

Accepted

## Context

Some Plugins manage cost-bearing or instance-wide concerns (rate limits on shared resources, kill switches, credentials shared across tenants) that
the existing tenant-scoped plugin model does not express. The first concrete case is `enhancement_dashboard_icons`: pulling icon metadata from a
public CDN should be opt-in at the instance level rather than enabled-by-default for every tenant.

The current Plugin model assumes all plugins are tenant-scoped: any `PluginCapability` is exposed to every Operator, `plugin_configs` and
`plugin_type_settings` rows live per-tenant, and disabling a plugin requires either uninstalling it (compile-time change) or asking each tenant to
flip a type-setting toggle. None of these match the requirement that an instance owner be able to globally turn the feature on/off, with the disabled
state being invisible to tenants.

## Decision

Introduce a new plugin scope, **Instance-Scoped Plugin** (`PluginScope::Instance`), managed exclusively by Operators with the `ManageGlobalSettings`
permission. Convert `enhancement_dashboard_icons` as the first instance-scoped plugin, disabled by default for both fresh and upgraded installs.

The decision comprises four sub-decisions, each with rejected alternatives:

### 1. New table vs raw keys in `global_settings`

A dedicated `instance_plugin_setting` table holds (`plugin_type_id`, `enabled`, `config`, `updated_at`). The `global_settings` table is **not**
extended.

**Rejected:** raw keys like `plugin.<id>.enabled` in `global_settings`. The `SettingKey` enum is itself a maintainability investment (typed keys,
exhaustive `from_db_key`, `is_global` predicate). Plugin-prefixed raw keys bypass that contract permanently. A dedicated table also makes future
hot-reload (per-row change events) and a single-shot `SELECT * FROM instance_plugin_setting` query for the admin UI both trivial.

### 2. Restart-required toggle vs hot-reload

Toggling enable/disable persists immediately, but the catalog reads the table **only at controller boot** to decide whether to construct each
instance-scoped plugin's singleton. Operators must restart the controller to apply a toggle.

**Rejected for v1:** hot-reload. Hot-reload requires a broadcast invalidation channel, lazy spawn/cancel for background tasks (e.g. dashboard-icons
cache refresh loop), and concurrency reasoning about partially-constructed singletons. The decoupled snapshot architecture (catalog snapshot at boot,
separate web-api snapshot per request) keeps hot-reload achievable as an additive change.

**Mitigation for restart-drift:** the `InstancePluginSummary` API response exposes both `enabled` (stored desired state) and `running_enabled`
(catalog snapshot from boot). The Plugin Configs UI renders a "Pending restart" badge when the two differ, so operators are not left with silently
invisible drift.

### 3. Reuse `ManageGlobalSettings` vs a new permission

All instance-plugin admin endpoints reuse the existing `ManageGlobalSettings` permission. No new `Permission` variant.

**Rejected for v1:** `ManageInstancePlugins`. The persona is identical (instance owner). Adding a Permission variant means non-exhaustive enum churn,
frontend role-mapping update, and additional friction for admins assigning roles — all to separate two activities the same human already does. A
future split is additive.

### 4. Tenant invisibility when disabled

A single visibility predicate (`web-api/src/visibility.rs`) gates every existing plugin-listing endpoint and the surfaces registry. Disabled
instance-scoped plugins return 404 (matching the existing "unknown plugin type" response shape) to users without `ManageGlobalSettings`. Instance
owners see disabled plugins everywhere, for debugging.

**Out of scope** (documented in the spec's leakage vectors checklist): the predicate covers HTTP and surfaces; it does **not** cover `AdminEvent` SSE
(`AdminEvent` carries no plugin-origin field — dashboard-icons does not emit `AdminEvent` directly, so the v1 invariant holds), agent-side runtime
(dashboard-icons is controller-only), MQTT topics (dashboard-icons doesn't publish), or persisted side effects on tenant-readable rows
(`software_item.icon_url` may carry CDN URLs from prior enrichment — accepted known limitation; no provenance column on `icon_url`).

## Consequences

**Positive:**

- Single source of truth (`instance_plugin_setting` table) for instance-plugin admin state; no overloading of `global_settings` semantics.
- Tenant-facing surface area is unchanged when the plugin is enabled — tenants continue to use `plugin_type_settings` for per-tenant overrides.
- Restart UX is honest (badge in UI when stored ≠ running).
- Future plugins promoted to `Instance` scope inherit the entire mechanism without schema changes.

**Negative:**

- Restart required to pick up enable/disable toggles. Mitigated by the Pending restart badge but not solved.
- Two distinct config surfaces per instance-scoped plugin (instance config + type_settings). New plugin authors must reason about which knob belongs
  where; documented in `docs/development/plugin-guidelines.md`.
- Visibility predicate must be re-applied at every new tenant-readable plugin surface. Future plugin authors must walk the leakage vectors checklist
  (per spec §6).

## See also

- Spec: `docs/superpowers/specs/2026-05-10-instance-scoped-plugins-design.md`
- Plans: `docs/superpowers/plans/2026-05-10-instance-scoped-plugins-{a,b,c}.md`
- CONTEXT.md glossary: Plugin Scope, Instance-Scoped Plugin
```

- [ ] **Step 3: Verify markdownlint**

```bash
markdownlint --config .markdownlint.json docs/adr/0006-instance-scoped-plugins.md
```

If lint errors: `npx prettier --write --prose-wrap always --print-width 150 docs/adr/0006-instance-scoped-plugins.md`.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0006-instance-scoped-plugins.md
git commit -m "docs(adr): 0006 instance-scoped plugins

Captures the four architectural sub-decisions: dedicated table vs raw
global_settings keys, restart-required vs hot-reload, ManageGlobalSettings
reuse vs new permission, predicate-based tenant invisibility."
```

---

## Task 14: Quality gates checkpoint

**Files:** none (verification only)

- [ ] **Step 1: Format**

```bash
cargo fmt --all
```

- [ ] **Step 2: cargo check both feature combos**

```bash
cargo check --no-default-features --features db-sqlite
cargo check --all-features
```

- [ ] **Step 3: clippy both feature combos**

```bash
cargo clippy --all-targets --no-default-features --features db-sqlite -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

- [ ] **Step 4: tests**

```bash
cargo test --all-features
```

- [ ] **Step 5: cargo deny**

```bash
cargo deny check
```

- [ ] **Step 6: markdownlint**

```bash
markdownlint --config .markdownlint.json '**/*.md'
```

- [ ] **Step 7: Boot smoke**

```bash
RUST_LOG=info cargo run -p uptrakit-controller --all-features -- --master-key-file <test-key> --help 2>&1 | grep -i "instance-scoped"
```

Expected: log line indicating dashboard-icons skipped construction at boot (since no row in `instance_plugin_setting`).

- [ ] **Step 8: No commit. Plan B done.**

---

## Self-review

Plan B vs spec:

- **Spec §4 (audit constants, snapshot writes for read-back consistency, route handlers, OpenAPI):** Tasks 2, 4, 5.
- **Spec §4 / §6 (visibility predicate applied to existing handlers):** Tasks 6, 7.
- **Spec §6 (delete_plugin_type_settings closes existence leak via 404-vs-204):** Task 6 step 5.
- **Spec §7 (dashboard-icons descriptor flip + behavior preservation):** Task 8.
- **Spec §6 (leakage vectors checklist for dashboard-icons):** Task 9.
- **Spec §9 (every test from the spec's test surface):** Tasks 10, 11, 12.
- **Spec §10 (ADR new):** Task 13.
- **Quality gates:** Task 14.

Doc deliverables touched in Plan B: ADR (Task 13) + dashboard-icons README (Task 9). Remaining doc deliverables (plugin-guidelines.md,
ARCHITECTURE.md, end-user dashboard-icons doc, admin guide page) live in Plan C.

Snapshot conformance per task:

- Every new public type has `Validate` (where relevant) and follows existing patterns.
- Every match on `#[non_exhaustive]` enum has wildcard arm.
- Every error path uses `rootcause::Report`.
- All locks via `parking_lot` or `arc_swap`; never `tokio::sync::*`.
- Every audit emission uses `AuditEntry::builder` + best-effort emit.
- Conventional commits.
- No `#[allow(...)]`. No "silence the lint" tasks.
- No fights with framework — extends `axum`, `utoipa`, `sea-orm`, `arc_swap`, `AuditEntry::builder`.
