//! HTTP handlers for `/api/v1/instance-plugins`.
//!
//! All four endpoints are gated by `CanManageSystemSettings`. Write paths
//! persist changes to `instance_plugin_setting` and atomically swap the
//! in-memory [`InstancePluginSnapshot`] in `AppState`.

use std::sync::Arc;
use uptrakit_shared_db::begin_immediate;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, Stateful};
use uptrakit_plugin_infrastructure_registry::{PluginDescriptor, PluginOps, PluginScope};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_queries::instance_plugin_settings::{
    self, InstancePluginRow, InstancePluginSettingView, InstancePluginSnapshot,
};
use uptrakit_web_api_types::instance_plugins::{
    InstancePluginDetail, InstancePluginSummary, SetInstancePluginEnabledRequest,
    UpsertInstancePluginConfigRequest,
};

use crate::AppState;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::action::CanManageSystemSettings;
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
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
        .map(|r| ops.mask_config_secrets(id, &r.config))
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
    responses(
        (status = 200, description = "List of instance-scoped plugins", body = Vec<InstancePluginSummary>),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_instance_plugins(
    State(state): State<Arc<AppState>>,
    CanManageSystemSettings(_user): CanManageSystemSettings,
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
    responses(
        (status = 200, description = "Instance plugin detail", body = InstancePluginDetail),
        (status = 404, description = "Instance plugin not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_instance_plugin(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageSystemSettings(_user): CanManageSystemSettings,
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
    responses(
        (status = 200, description = "Plugin enabled state updated", body = InstancePluginSummary),
        (status = 404, description = "Instance plugin not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn set_instance_plugin_enabled(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageSystemSettings(user): CanManageSystemSettings,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<SetInstancePluginEnabledRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|v| v.0);
    let id = PluginTypeId::new(&plugin_type);
    let ops = state.plugin.plugin_ops.as_ref();
    if let Err(r) = resolve_instance_plugin(ops, &id) {
        return r;
    }

    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);

    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for instance plugin toggle");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let (before_model, after_model) =
        match instance_plugin_settings::set_enabled_in_tx(&tx, &plugin_type, req.enabled).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::error!(error = %e, "Failed to set instance plugin enabled");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let after_view = InstancePluginSettingView::from(&after_model);
    let audit_entry_result = match before_model.as_ref() {
        Some(before) => {
            let before_view = InstancePluginSettingView::from(before);
            AuditEntry::<Stateful>::instance_plugin_toggled(&before_view, &after_view)
                .system_scope()
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Success)
                .details(serde_json::json!({}))
                .build()
        }
        None => {
            AuditEntry::<Stateful>::instance_plugin_toggled(&AbsentView(&after_view), &after_view)
                .system_scope()
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Success)
                .details(serde_json::json!({}))
                .build()
        }
    };
    let audit_entry = match audit_entry_result {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(error = %e, "Failed to build audit entry for instance plugin toggle");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hook = state.audit_emitter.commit_hook();
    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(error = %e, "Failed to emit audit entry for instance plugin toggle");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit instance plugin toggle");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Atomically update the in-memory snapshot AFTER commit.
    let new_row = InstancePluginRow {
        enabled: after_model.enabled,
        config: after_model.config.as_json().clone(),
        updated_at: after_model.updated_at,
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
    responses(
        (status = 200, description = "Plugin configuration updated", body = InstancePluginSummary),
        (status = 400, description = "Invalid configuration"),
        (status = 404, description = "Instance plugin not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized"),
    ),
    tag = "Instance Plugins",
    security(("oauth2" = ["system.settings:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn upsert_instance_plugin_config(
    State(state): State<Arc<AppState>>,
    Path(plugin_type): Path<String>,
    CanManageSystemSettings(user): CanManageSystemSettings,
    api_token_id: Option<axum::Extension<AuthenticatedApiTokenId>>,
    Validated(mut req): Validated<UpsertInstancePluginConfigRequest>,
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

    let snapshot = state.instance_plugin_snapshot.load();
    if let Some(existing_row) = snapshot.get(id.as_str()) {
        ops.restore_config_secrets(&id, &mut req.config, &existing_row.config);
    }

    if let Err(e) = ops.assert_no_sentinel(&id, &req.config) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid instance config: {e}"),
        );
    }

    if let Err(e) = (instance_config_ops.validate)(&req.config) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid instance config: {e}"),
        );
    }

    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);

    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "Failed to begin transaction for instance plugin config upsert");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let (before_model, after_model) =
        match instance_plugin_settings::upsert_config_in_tx(&tx, &plugin_type, req.config).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::error!(error = %e, "Failed to upsert instance plugin config");
                drop(tx);
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };

    let after_view = InstancePluginSettingView::from(&after_model);
    let audit_entry_result = match before_model.as_ref() {
        Some(before) => {
            let before_view = InstancePluginSettingView::from(before);
            AuditEntry::<Stateful>::instance_plugin_config_upserted(&before_view, &after_view)
                .system_scope()
                .actor(actor_type, actor_id)
                .outcome(AuditOutcome::Success)
                .details(serde_json::json!({}))
                .build()
        }
        None => AuditEntry::<Stateful>::instance_plugin_config_upserted(
            &AbsentView(&after_view),
            &after_view,
        )
        .system_scope()
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({}))
        .build(),
    };
    let audit_entry = match audit_entry_result {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to build audit entry for instance plugin config upsert"
            );
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let hook = state.audit_emitter.commit_hook();
    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!(
            error = %e,
            "Failed to emit audit entry for instance plugin config upsert"
        );
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!(error = %e, "Failed to commit instance plugin config upsert");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    // Atomically update the in-memory snapshot AFTER commit.
    let new_row = InstancePluginRow {
        enabled: after_model.enabled,
        config: after_model.config.as_json().clone(),
        updated_at: after_model.updated_at,
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

    // The registry is immutable after boot; resolve_instance_plugin already verified this
    // plugin type exists and is Instance-scoped, so get() will succeed here.
    if let Some(desc) = ops.get(&id) {
        let summary = build_summary(&id, desc, new_snapshot.as_ref(), ops);
        return (StatusCode::OK, Json(summary)).into_response();
    }
    error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}
