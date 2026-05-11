//! HTTP handlers for `/api/v1/instance-plugins`.
//!
//! All four endpoints are gated by `CanManageGlobalSettings`. Write paths
//! persist changes to `instance_plugin_setting` and atomically swap the
//! in-memory [`InstancePluginSnapshot`] in `AppState`.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};

use uptrakit_plugin_infrastructure_registry::{PluginDescriptor, PluginOps, PluginScope};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_queries::instance_plugin_settings::{
    self, InstancePluginRow, InstancePluginSnapshot,
};
use uptrakit_web_api_types::instance_plugins::{
    InstancePluginDetail, InstancePluginSummary, SetInstancePluginEnabledRequest,
    UpsertInstancePluginConfigRequest,
};

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::routes::plugin_configs::plugin_field_to_api_field;

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Build an [`InstancePluginSummary`] from descriptor + snapshot + live ops.
fn build_summary(
    id: &PluginTypeId,
    desc: &PluginDescriptor,
    snapshot: &InstancePluginSnapshot,
    ops: &dyn PluginOps,
) -> InstancePluginSummary {
    let row = snapshot.get(id.as_str());
    let enabled = row.map(|r| r.enabled).unwrap_or(false);
    let current_config = row
        .map(|r| r.config.clone())
        .unwrap_or_else(|| serde_json::json!({}));
    let updated_at = row.map(|r| r.updated_at);

    let instance_config_form_fields = desc
        .instance_config
        .map(|ic| (ic.form_schema)())
        .unwrap_or_default()
        .into_iter()
        .map(plugin_field_to_api_field)
        .collect();

    let type_settings_form_fields = ops
        .type_settings_form_schema(id)
        .unwrap_or_default()
        .into_iter()
        .map(plugin_field_to_api_field)
        .collect();

    InstancePluginSummary {
        plugin_type: id.clone(),
        display_name: ops.display_name(id),
        enabled,
        running_enabled: ops.instance_enabled(id),
        has_instance_config: desc.instance_config.is_some(),
        instance_config_form_fields,
        type_settings_form_fields,
        current_config,
        updated_at,
    }
}

/// Emit an `INSTANCE_PLUGIN_TOGGLED` audit entry.
fn emit_toggle_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
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
    .system_scope()
    .actor(actor_type, actor_id)
    .target(
        "instance_plugin",
        plugin_type.to_string(),
        Some(plugin_type.to_string()),
    )
    .outcome(uptrakit_audit_log::AuditOutcome::Success)
    .details(details)
    .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}

/// Emit an `INSTANCE_PLUGIN_CONFIG_UPSERTED` audit entry.
fn emit_config_upsert_audit(
    audit_emitter: &uptrakit_audit_log::AuditEmitter,
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
    .system_scope()
    .actor(actor_type, actor_id)
    .target(
        "instance_plugin",
        plugin_type.to_string(),
        Some(plugin_type.to_string()),
    )
    .outcome(uptrakit_audit_log::AuditOutcome::Success)
    .details(details)
    .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}

/// Resolve a plugin type to its descriptor, or return a 404 if unknown or not
/// Instance-scoped. The 404 body is identical for both cases to avoid
/// existence leaks.
#[expect(
    clippy::result_large_err,
    reason = "error variant carries a Response which is large but unavoidable at this API boundary"
)]
fn resolve_instance_plugin<'a>(
    ops: &'a dyn PluginOps,
    id: &PluginTypeId,
) -> Result<&'a PluginDescriptor, Response> {
    match ops.get(id) {
        Some(desc) if desc.scope == PluginScope::Instance => Ok(desc),
        _ => Err(error_response(
            StatusCode::NOT_FOUND,
            "Instance plugin not found",
        )),
    }
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// List all instance-scoped plugins.
#[utoipa::path(
    get,
    path = "/api/v1/instance-plugins",
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "List of instance-scoped plugins", body = Vec<InstancePluginSummary>),
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
    let ops = state.plugin.plugin_ops.as_ref();
    let snapshot = state.instance_plugin_snapshot.load_full();

    let summaries: Vec<InstancePluginSummary> = ops
        .all()
        .into_iter()
        .filter(|d| d.scope == PluginScope::Instance)
        .map(|d| {
            let id = PluginTypeId::from_static(d.type_id);
            build_summary(&id, d, &snapshot, ops)
        })
        .collect();

    (StatusCode::OK, Json(summaries)).into_response()
}

/// Get a single instance-scoped plugin by type.
#[utoipa::path(
    get,
    path = "/api/v1/instance-plugins/{plugin_type}",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "Instance plugin detail", body = InstancePluginDetail),
        (status = 404, description = "Instance plugin not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_instance_plugin(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    let id = PluginTypeId::new(&plugin_type);
    let ops = state.plugin.plugin_ops.as_ref();
    let desc = match resolve_instance_plugin(ops, &id) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let snapshot = state.instance_plugin_snapshot.load_full();
    let summary = build_summary(&id, desc, &snapshot, ops);
    (StatusCode::OK, Json(InstancePluginDetail { summary })).into_response()
}

/// Enable or disable an instance-scoped plugin.
#[utoipa::path(
    put,
    path = "/api/v1/instance-plugins/{plugin_type}/enabled",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    request_body = SetInstancePluginEnabledRequest,
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "Plugin enabled state updated", body = InstancePluginSummary),
        (status = 404, description = "Instance plugin not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn set_instance_plugin_enabled(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<SetInstancePluginEnabledRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let id = PluginTypeId::new(&plugin_type);
    let ops = state.plugin.plugin_ops.as_ref();
    if let Err(r) = resolve_instance_plugin(ops, &id) {
        return r;
    }

    let (previous_enabled, model) =
        match instance_plugin_settings::set_enabled(state.db(), &plugin_type, req.enabled).await {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(error = %e, "Failed to set instance plugin enabled");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    // Atomically update the in-memory snapshot.
    let new_row = InstancePluginRow {
        enabled: model.enabled,
        config: model.config,
        updated_at: model.updated_at,
    };
    let new_snapshot = Arc::new(
        state
            .instance_plugin_snapshot
            .load()
            .with_upserted(plugin_type.clone(), new_row),
    );
    state
        .instance_plugin_snapshot
        .store(Arc::clone(&new_snapshot));

    emit_toggle_audit(
        &state.audit_emitter,
        &user,
        api_token_id,
        &plugin_type,
        previous_enabled,
        req.enabled,
    );

    // The registry is immutable after boot; resolve_instance_plugin already verified this
    // plugin type exists and is Instance-scoped, so get() will succeed here.
    if let Some(desc) = ops.get(&id) {
        let summary = build_summary(&id, desc, new_snapshot.as_ref(), ops);
        return (StatusCode::OK, Json(summary)).into_response();
    }
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

/// Upsert the instance-wide configuration for an instance-scoped plugin.
#[utoipa::path(
    put,
    path = "/api/v1/instance-plugins/{plugin_type}/config",
    params(("plugin_type" = String, Path, description = "Plugin type identifier")),
    request_body = UpsertInstancePluginConfigRequest,
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    responses(
        (status = 200, description = "Plugin configuration updated", body = InstancePluginSummary),
        (status = 400, description = "Invalid configuration"),
        (status = 404, description = "Instance plugin not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn upsert_instance_plugin_config(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<UpsertInstancePluginConfigRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let id = PluginTypeId::new(&plugin_type);
    let ops = state.plugin.plugin_ops.as_ref();
    let desc = match resolve_instance_plugin(ops, &id) {
        Ok(d) => d,
        Err(r) => return r,
    };

    let Some(instance_config_ops) = desc.instance_config else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "This plugin has no instance configuration schema",
        );
    };

    if let Err(e) = (instance_config_ops.validate)(&req.config) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid instance config: {e}"),
        );
    }

    let config_field_count = req.config.as_object().map(|o| o.len()).unwrap_or(0);

    let model =
        match instance_plugin_settings::upsert_config(state.db(), &plugin_type, req.config).await {
            Ok(m) => m,
            Err(e) => {
                tracing::error!(error = %e, "Failed to upsert instance plugin config");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    // Atomically update the in-memory snapshot.
    let new_row = InstancePluginRow {
        enabled: model.enabled,
        config: model.config,
        updated_at: model.updated_at,
    };
    let new_snapshot = Arc::new(
        state
            .instance_plugin_snapshot
            .load()
            .with_upserted(plugin_type.clone(), new_row),
    );
    state
        .instance_plugin_snapshot
        .store(Arc::clone(&new_snapshot));

    emit_config_upsert_audit(
        &state.audit_emitter,
        &user,
        api_token_id,
        &plugin_type,
        config_field_count,
    );

    // The registry is immutable after boot; resolve_instance_plugin already verified this
    // plugin type exists and is Instance-scoped, so get() will succeed here.
    if let Some(desc) = ops.get(&id) {
        let summary = build_summary(&id, desc, new_snapshot.as_ref(), ops);
        return (StatusCode::OK, Json(summary)).into_response();
    }
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}
