#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
#![expect(
    clippy::indexing_slicing,
    reason = "index is computed or validated to be in bounds"
)]

use crate::AppState;
use crate::app_state::AuditEmitterState;
use crate::auth::permissions::Permission;
use crate::config_test_proxy::ConfigTestProxyError;
use crate::error_response::error_response;
use crate::extract::Validated;
use crate::extractors::{IfMatch, SettingsVersion};
use crate::middleware::permission::{
    CanManageCommands, CanTestPluginConfigs, CanTriggerChecks, CanViewSoftware,
};
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use crate::queries::plugin_configs as pc_queries;
use crate::queries::plugin_configs::PluginConfigView;
use crate::tenant_db::TenantDb;
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, JoinType, QueryFilter, QuerySelect, RelationTrait,
    SqliteTransactionMode, TransactionOptions, TransactionTrait,
};
use std::sync::Arc;
use uptrakit_audit_log::{AbsentView, AuditEntry, AuditOutcome, AuditView, Event, Stateful};
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

fn reject_config_model_none_plugin_type(
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
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id()))
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

struct AuditContext<'a> {
    audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    tenant_id: Uuid,
    user: &'a AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
}

fn emit_plugin_config_semantic_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_type: Option<&'static str>,
    target_id: Option<String>,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let target_type = target_type.map(std::string::ToString::to_string);
    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
            .tenant_scope(ctx.tenant_id)
            .actor(actor_type, actor_id)
            .target_opt(target_type, target_id, target_display)
            .outcome(outcome)
            .details(details)
            .build()
    {
        ctx.audit_emitter.emit_event(entry);
    }
}

fn dangerous_pattern_matches_to_json(
    matches: &[command_safety::DangerousPatternMatch],
) -> serde_json::Value {
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
    dangerous_matches: Vec<command_safety::DangerousPatternMatch>,
}

impl CommandRiskSummary {
    fn from_config(config: &serde_json::Value) -> Self {
        Self {
            command_fields: command_safety::detect_command_fields(config),
            dangerous_matches: command_safety::collect_dangerous_patterns(config),
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
                    crate::visibility::is_plugin_visible_to_user(d, snapshot.as_ref(), &auth_user)
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
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(req): Validated<CreatePluginConfigRequest>,
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
            command_safety::format_dangerous_pattern_rejection(&config_risk.dangerous_matches),
        );
    }

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
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
#[derive(Debug, serde::Deserialize)]
pub struct ListPluginConfigsParams {
    /// Page number (1-indexed, default 1).
    pub page: Option<u64>,
    /// Items per page (default 20, max 1000).
    pub per_page: Option<u64>,
    /// Filter by plugin type (e.g. `infrastructure_proxmox`). Returns all types when absent.
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
    params(
        ("page" = Option<u64>, Query, description = "Page number (1-indexed, default 1)"),
        ("per_page" = Option<u64>, Query, description = "Items per page (default 20, max 1000)"),
        ("plugin_type" = Option<String>, Query, description = "Filter by plugin type identifier")
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
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Json(req): Json<UpdatePluginConfigRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

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
            command_safety::format_dangerous_pattern_rejection(&risk.dangerous_matches),
        );
    }

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
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
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    Path(config_id): Path<Uuid>,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let (actor_type, actor_id) = authenticated_user_audit_actor(&user, api_token_id);
    let tenant_id = tenant_db.tenant_id();

    let tx = match state
        .db()
        .begin_with_options(TransactionOptions {
            sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
            ..Default::default()
        })
        .await
    {
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
        .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id()))
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
        .plugin
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
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
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
            let msg = uptrakit_wire::ControllerMessage::DiscoverSoftware(
                uptrakit_wire::DiscoverSoftwarePayload {
                    host_machine_id: machine_id.clone(),
                    plugins: vec![uptrakit_wire::DiscoveryPluginAssignment {
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
    State(audit_emitter_state): State<AuditEmitterState>,
    _if_match: IfMatch<SettingsVersion>,
    tenant_db: TenantDb,
    CanManageCommands(user): CanManageCommands,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Validated(body): Validated<BatchActionRequest>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &audit_emitter_state.0,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    let (succeeded_ids, failed) = match body.action.as_str() {
        "delete" => match pc_queries::batch_delete_plugin_configs(&tenant_db, &body.ids).await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("batch delete failed: {e}");
                emit_plugin_config_semantic_audit(
                    &audit_ctx,
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
                &audit_ctx,
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
        &audit_ctx,
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
    let caps = state.plugin.plugin_ops.capabilities(&plugin_type_id);

    // 2. Merge with saved config if plugin_config_id is provided.
    let config = if let Some(config_id) = body.plugin_config_id {
        let saved = match PluginConfig::find_by_id(config_id)
            .filter(plugin_config::Column::TenantId.eq(tenant_db.tenant_id()))
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
    if let Err(e) = state
        .plugin
        .plugin_ops
        .validate_config(&plugin_type_id, &config)
    {
        return error_response(
            StatusCode::BAD_REQUEST,
            format!("Invalid plugin config: {e}"),
        );
    }

    // 4. Reject dangerous commands if enabled.
    if state.reject_dangerous_commands {
        let matches = command_safety::collect_dangerous_patterns(&config);
        if !matches.is_empty() {
            return error_response(
                StatusCode::BAD_REQUEST,
                command_safety::format_dangerous_pattern_rejection(&matches),
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
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
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
        "version_detection" => uptrakit_wire::ConfigTestKind::VersionDetection,
        "update_command_validation" => uptrakit_wire::ConfigTestKind::UpdateCommandValidation,
        "pre_update_hook" => uptrakit_wire::ConfigTestKind::PreUpdateHook,
        "post_update_hook" => uptrakit_wire::ConfigTestKind::PostUpdateHook,
        "connectivity" => uptrakit_wire::ConfigTestKind::Connectivity,
        _ => {
            return error_response(
                StatusCode::BAD_REQUEST,
                format!("Unknown test_kind: {test_kind_str}"),
            );
        }
    };

    // 8. Build payload and invoke via proxy.
    let request_id = Uuid::now_v7().to_string();
    let mut payload = uptrakit_wire::TestPluginConfigPayload::new(
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

mod command_safety;

#[cfg(test)]
mod tests;
