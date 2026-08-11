use crate::AppState;
use crate::error_response::error_response;
use crate::extract::{Unvalidated, Validated};
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::action::{
    AccessAuthority, CanManageCommands, CanReadSoftware, authorize_any,
};
use crate::middleware::require_auth::{AuthenticatedApiTokenId, authenticated_user_audit_actor};
use crate::queries::plugin_configs as pc_queries;
use crate::queries::plugin_configs::PluginConfigView;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, AuditView, Event, Stateful};
use uptrakit_plugin_infrastructure_registry::{ConfigModel, PluginDescriptor};
use uptrakit_shared_db::begin_immediate;
use uptrakit_shared_types::PluginTypeId;
use uptrakit_shared_types::access::actions;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, PluginTypeInfo, UpdatePluginConfigRequest,
};
use uuid::Uuid;

use super::audit::{CommandRiskSummary, dangerous_pattern_matches_to_json};
use super::command_safety::format_dangerous_pattern_rejection;

#[expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
pub(crate) fn plugin_field_to_api_field<T>(
    field: T,
) -> uptrakit_web_api_types::plugin_configs::FormField
where
    T: serde::Serialize,
{
    let value = serde_json::to_value(field)
        .expect("plugin field schema serialization should succeed at route boundary");
    serde_json::from_value(value)
        .expect("plugin field schema conversion to API DTO should succeed at route boundary")
}

fn descriptor_is_config_model_none(descriptor: &PluginDescriptor) -> bool {
    matches!(descriptor.config_model, ConfigModel::None)
}

pub(super) fn reject_config_model_none_plugin_type(
    state: &AppState,
    plugin_type_id: &PluginTypeId,
) -> Option<Response> {
    let descriptor = match state.plugin.plugin_ops.get(plugin_type_id) {
        Some(d) => d,
        None => {
            return Some(error_response(
                StatusCode::BAD_REQUEST,
                "Unknown plugin type",
            ));
        }
    };

    if descriptor_is_config_model_none(descriptor) {
        return Some(error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Plugin type '{}' does not support per-instance plugin configs",
                plugin_type_id
            ),
        ));
    }

    None
}

/// List all known plugin types with their display names and capabilities.
///
/// Returns static registry metadata — no tenant data is involved. Clients
/// should call this endpoint to populate plugin-type selectors rather than
/// hard-coding plugin type strings.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-types",
    responses(
        (status = 200, description = "List of known plugin types", body = Vec<PluginTypeInfo>),
    ),
    tag = "Plugin Configs",
    security(
        ("oauth2" = ["software:read"]),
        ("oauth2" = ["settings:read"]),
        ("oauth2" = ["system.settings:manage"]),
        ("developer_token" = [])
    )
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_types(
    State(state): State<Arc<AppState>>,
    Extension(authority): Extension<AccessAuthority>,
) -> Response {
    let Some(access_ctx) = authority.ready() else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    if authorize_any(
        &state.access_engine,
        access_ctx,
        &state.audit_emitter,
        &[
            actions::SOFTWARE_READ,
            actions::SETTINGS_READ,
            actions::SYSTEM_SETTINGS_MANAGE,
        ],
    )
    .is_err()
    {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

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
                .map(|d| {
                    crate::visibility::is_plugin_visible_to_user(
                        d,
                        state.plugin.plugin_ops.as_ref(),
                        snapshot.as_ref(),
                        &state.access_engine,
                        access_ctx,
                    )
                })
                .unwrap_or(false)
        })
        .map(|id| {
            let capabilities = state.plugin.plugin_ops.capabilities(&id);
            let config_form_fields = state
                .plugin
                .plugin_ops
                .config_form_schema(&id)
                .unwrap_or_default()
                .into_iter()
                .map(plugin_field_to_api_field)
                .collect();
            let type_settings_form_fields = state
                .plugin
                .plugin_ops
                .type_settings_form_schema(&id)
                .unwrap_or_default()
                .into_iter()
                .map(plugin_field_to_api_field)
                .collect();
            let type_settings_sample = state.plugin.plugin_ops.type_settings_sample(&id);
            let display_name = state.plugin.plugin_ops.display_name(&id);
            let plugin_type = id.clone();
            let supports_plugin_configs = state
                .plugin
                .plugin_ops
                .get(&id)
                .map(|d| !descriptor_is_config_model_none(d))
                .unwrap_or(false);
            let sample_config = state
                .plugin
                .plugin_ops
                .get(&id)
                .map(|d| {
                    if descriptor_is_config_model_none(d) {
                        serde_json::json!({})
                    } else {
                        state.plugin.plugin_ops.sample_config(&id)
                    }
                })
                .unwrap_or_default();
            PluginTypeInfo {
                display_name,
                plugin_type,
                supports_plugin_configs,
                capabilities,
                sample_config,
                config_form_fields,
                type_settings_form_fields,
                type_settings_sample,
            }
        })
        .collect();
    (StatusCode::OK, Json(types)).into_response()
}

/// Create a new plugin configuration.
///
/// Requires `manage_commands` permission because plugin configs can contain
/// arbitrary shell commands executed on managed hosts.
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs",
    request_body = CreatePluginConfigRequest,
    responses(
        (status = 201, description = "Plugin config created", body = PluginConfigResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "A plugin config with this name already exists")
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["commands:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_plugin_config(
    State(state): State<Arc<AppState>>,
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(mut req): Validated<CreatePluginConfigRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let plugin_type_str = req.plugin_type.to_string();
    let config_name = req.name.clone();
    let config_risk = CommandRiskSummary::from_config(&req.config);

    // Validate plugin-specific config and plugin type support.
    let plugin_type_id = PluginTypeId::new(&plugin_type_str);
    if let Some(rejection) = reject_config_model_none_plugin_type(&state, &plugin_type_id) {
        return rejection;
    }
    if let Err(e) = state
        .plugin
        .plugin_ops
        .validate_config(&plugin_type_id, &req.config)
    {
        return error_response(StatusCode::BAD_REQUEST, e.to_string());
    }

    let _pruned = state
        .plugin
        .plugin_ops
        .prune_stale_sensitive_keys(&plugin_type_id, &mut req.config);

    // Reject dangerous command patterns when operator policy is enabled.
    if state.reject_dangerous_commands && !config_risk.dangerous_matches.is_empty() {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "plugin_type": plugin_type_str,
            "config_name": config_name,
            "contains_command_fields": !config_risk.command_fields.is_empty(),
            "reason_code": "dangerous_command_patterns_detected",
            "dangerous_matches": dangerous_pattern_matches_to_json(&config_risk.dangerous_matches),
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::BAD_REQUEST,
            format_dangerous_pattern_rejection(&config_risk.dangerous_matches),
        );
    }

    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for plugin config create: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let model = match pc_queries::create_plugin_config_in_tx(&tx, tenant_id, req).await {
        Ok(m) => m,
        Err(err) => {
            drop(tx);
            let (outcome, reason_code) = if matches!(
                err.current_context(),
                pc_queries::PluginConfigError::DuplicateName
            ) {
                (AuditOutcome::Denied, "duplicate_name")
            } else {
                tracing::error!("DB error creating plugin config: {err}");
                (AuditOutcome::Failed, "plugin_config_create_failed")
            };
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .outcome(outcome)
            .details(serde_json::json!({
                "plugin_type": plugin_type_str,
                "config_name": config_name,
                "reason_code": reason_code,
            }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return if matches!(
                err.current_context(),
                pc_queries::PluginConfigError::DuplicateName
            ) {
                error_response(
                    StatusCode::CONFLICT,
                    "A plugin config with this name already exists",
                )
            } else {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            };
        }
    };

    let after = PluginConfigView::from(&model);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::plugin_config_create(
        &AbsentView(&after),
        &after,
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "plugin_type": plugin_type_str,
        "config_name": model.name,
        "enabled": model.enabled,
        "contains_command_fields": !config_risk.command_fields.is_empty(),
        "command_fields": config_risk.command_fields,
        "dangerous_command_match_count": config_risk.dangerous_matches.len(),
        "dangerous_matches": dangerous_pattern_matches_to_json(&config_risk.dangerous_matches),
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for plugin config create: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for plugin config create: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit plugin config create: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let Some(resp) = pc_queries::plugin_config_to_response(state.plugin.plugin_ops.as_ref(), model)
    else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    (StatusCode::CREATED, Json(resp)).into_response()
}

/// Query parameters for listing plugin configurations.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ListPluginConfigsParams {
    /// Page number (1-indexed, default 1).
    pub page: Option<u64>,
    /// Items per page (default 20, max 1000).
    pub per_page: Option<u64>,
    /// Filter by plugin type (e.g. `infrastructure.proxmox`). Returns all types when absent.
    pub plugin_type: Option<String>,
}

impl From<&ListPluginConfigsParams> for PaginationParams {
    fn from(p: &ListPluginConfigsParams) -> Self {
        Self {
            page: p.page,
            per_page: p.per_page,
        }
    }
}

/// List all non-deactivated plugin configurations.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-configs",
    params(ListPluginConfigsParams),
    responses(
        (status = 200, description = "Paginated list of plugin configs", body = PaginatedResponse<PluginConfigResponse>),
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["software:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_configs(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanReadSoftware(_user): CanReadSoftware,
    Query(params): Query<ListPluginConfigsParams>,
) -> Response {
    let pagination = PaginationParams::from(&params);
    match pc_queries::list_plugin_configs(
        state.plugin.plugin_ops.as_ref(),
        &tenant_db,
        &pagination,
        params.plugin_type.as_deref(),
    )
    .await
    {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!("Failed to list plugin configs: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Get a specific plugin configuration.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-configs/{id}",
    params(("id" = Uuid, Path, description = "Plugin config ID")),
    responses(
        (status = 200, description = "Plugin config details", body = PluginConfigResponse),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["software:read"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanReadSoftware(_user): CanReadSoftware,
) -> Response {
    match pc_queries::get_plugin_config(state.plugin.plugin_ops.as_ref(), &tenant_db, config_id)
        .await
    {
        Ok(Some(resp)) => (StatusCode::OK, Json(resp)).into_response(),
        Ok(None) => error_response(StatusCode::NOT_FOUND, "Plugin config not found"),
        Err(e) => {
            tracing::error!("Failed to get plugin config: {e}");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

/// Update a plugin configuration (partial update).
///
/// Requires `manage_commands` permission because plugin configs can contain
/// arbitrary shell commands executed on managed hosts.
#[utoipa::path(
    put,
    path = "/api/v1/plugin-configs/{id}",
    params(("id" = Uuid, Path, description = "Plugin config ID")),
    request_body = UpdatePluginConfigRequest,
    responses(
        (status = 200, description = "Plugin config updated", body = PluginConfigResponse),
        (status = 400, description = "Invalid request body"),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["commands:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
#[expect(
    clippy::indexing_slicing,
    reason = "index is computed or validated to be in bounds"
)]
pub async fn update_plugin_config(
    State(state): State<Arc<AppState>>,
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    body: Unvalidated<UpdatePluginConfigRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let req = match body.require_valid() {
        Ok(req) => req,
        Err(e) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("plugin_config", config_id.to_string(), None)
            .outcome(AuditOutcome::ValidationFailed)
            .details(serde_json::json!({ "reason_code": "invalid_request" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::BAD_REQUEST, e.to_string());
        }
    };

    // Pre-tx: load existing to check plugin type and dangerous patterns.
    let existing = match pc_queries::find_raw_active_config(&tenant_db, config_id).await {
        Ok(Some(m)) => m,
        Ok(None) => {
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("plugin_config", config_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "plugin_config_not_found" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
        }
        Err(e) => {
            tracing::error!("DB error loading plugin config for update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let existing_plugin_type = PluginTypeId::new(&existing.plugin_type);
    if let Some(rejection) = reject_config_model_none_plugin_type(&state, &existing_plugin_type) {
        return rejection;
    }

    let new_config_risk = req.config.as_ref().map(CommandRiskSummary::from_config);

    // Reject dangerous command patterns when operator policy is enabled.
    if state.reject_dangerous_commands
        && let Some(ref risk) = new_config_risk
        && !risk.dangerous_matches.is_empty()
    {
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("plugin_config", config_id.to_string(), req.name.clone())
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "plugin_type": existing_plugin_type.to_string(),
            "contains_command_fields": !risk.command_fields.is_empty(),
            "reason_code": "dangerous_command_patterns_detected",
            "dangerous_matches": dangerous_pattern_matches_to_json(&risk.dangerous_matches),
        }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(
            StatusCode::BAD_REQUEST,
            format_dangerous_pattern_rejection(&risk.dangerous_matches),
        );
    }

    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for plugin config update: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let pair = match pc_queries::update_plugin_config_in_tx(
        &tx,
        state.plugin.plugin_ops.as_ref(),
        tenant_id,
        config_id,
        req,
    )
    .await
    {
        Ok(p) => p,
        Err(err) => {
            drop(tx);
            let ctx = err.current_context();
            let (status, reason_code) = match ctx {
                pc_queries::PluginConfigError::EmptyName => (StatusCode::BAD_REQUEST, "empty_name"),
                pc_queries::PluginConfigError::ConfigValidation(_) => {
                    (StatusCode::BAD_REQUEST, "config_validation_failed")
                }
                _ => {
                    tracing::error!("DB error updating plugin config: {err}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "plugin_config_update_failed",
                    )
                }
            };
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("plugin_config", config_id.to_string(), None)
            .outcome(if status == StatusCode::BAD_REQUEST {
                AuditOutcome::ValidationFailed
            } else {
                AuditOutcome::Failed
            })
            .details(serde_json::json!({ "reason_code": reason_code }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(status, err.current_context().to_string());
        }
    };

    let Some((before_model, after_model)) = pair else {
        drop(tx);
        if let Ok(entry) = AuditEntry::<Event>::builder_event(
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target("plugin_config", config_id.to_string(), None)
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({ "reason_code": "plugin_config_not_found" }))
        .build()
        {
            state.audit_emitter.emit_event(entry);
        }
        return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
    };

    let before_view = PluginConfigView::from(&before_model);
    let after_view = PluginConfigView::from(&after_model);

    let risk_details = new_config_risk
        .as_ref()
        .map(CommandRiskSummary::details_fragment)
        .unwrap_or_else(|| CommandRiskSummary::default().details_fragment());

    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::plugin_config_update(&before_view, &after_view)
        // Override target_display with the after (new) name so the audit row
        // reflects the post-update display, not the pre-update name.
        .target(
            "plugin_config",
            after_view.audit_target_id(),
            after_view.audit_target_display(),
        )
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .outcome(AuditOutcome::Success)
        .details(serde_json::json!({
            "plugin_type": after_model.plugin_type,
            "config_name": after_model.name,
            "enabled": after_model.enabled,
            "contains_command_fields": risk_details["contains_command_fields"],
            "command_fields": risk_details["command_fields"],
            "dangerous_command_match_count": risk_details["dangerous_command_match_count"],
            "dangerous_matches": risk_details["dangerous_matches"],
        }))
        .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for plugin config update: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for plugin config update: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit plugin config update: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    let Some(resp) =
        pc_queries::plugin_config_to_response(state.plugin.plugin_ops.as_ref(), after_model)
    else {
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    };
    (StatusCode::OK, Json(resp)).into_response()
}

/// Soft-delete a plugin configuration.
#[utoipa::path(
    delete,
    path = "/api/v1/plugin-configs/{id}",
    params(("id" = Uuid, Path, description = "Plugin config ID")),
    responses(
        (status = 204, description = "Plugin config deleted"),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("oauth2" = ["commands:manage"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_config(
    State(state): State<Arc<AppState>>,
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match begin_immediate(state.db()).await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!("Failed to begin transaction for plugin config delete: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_model = match pc_queries::delete_plugin_config_in_tx(&tx, tenant_id, config_id).await
    {
        Ok(Some(m)) => m,
        Ok(None) => {
            drop(tx);
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("plugin_config", config_id.to_string(), None)
            .outcome(AuditOutcome::Denied)
            .details(serde_json::json!({ "reason_code": "plugin_config_not_found" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
        }
        Err(e) => {
            drop(tx);
            tracing::error!("Failed to delete plugin config: {e}");
            if let Ok(entry) = AuditEntry::<Event>::builder_event(
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
            )
            .tenant_scope(tenant_id)
            .actor(actor_type, actor_id)
            .target("plugin_config", config_id.to_string(), None)
            .outcome(AuditOutcome::Failed)
            .details(serde_json::json!({ "reason_code": "plugin_config_delete_failed" }))
            .build()
            {
                state.audit_emitter.emit_event(entry);
            }
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let before_view = PluginConfigView::from(&before_model);
    let config_risk = CommandRiskSummary::from_config(&before_model.config);
    let hook = state.audit_emitter.commit_hook();
    let audit_entry = match AuditEntry::<Stateful>::plugin_config_delete(
        &before_view,
        &AbsentView(&before_view),
    )
    .tenant_scope(tenant_id)
    .actor(actor_type, actor_id)
    .outcome(AuditOutcome::Success)
    .details(serde_json::json!({
        "plugin_type": before_model.plugin_type,
        "config_name": before_model.name,
        "enabled": before_model.enabled,
        "contains_command_fields": !config_risk.command_fields.is_empty(),
        "command_fields": config_risk.command_fields,
        "dangerous_command_match_count": config_risk.dangerous_matches.len(),
        "dangerous_matches": dangerous_pattern_matches_to_json(&config_risk.dangerous_matches),
    }))
    .build()
    {
        Ok(entry) => entry,
        Err(e) => {
            tracing::error!("Failed to build audit entry for plugin config delete: {e}");
            drop(tx);
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    if let Err(e) = state
        .audit_emitter
        .emit_stateful(&tx, &hook, audit_entry)
        .await
    {
        tracing::error!("Failed to emit audit entry for plugin config delete: {e}");
        drop(tx);
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("Failed to commit plugin config delete: {e}");
        return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
    }
    hook.flush_after_commit().await;

    StatusCode::NO_CONTENT.into_response()
}
