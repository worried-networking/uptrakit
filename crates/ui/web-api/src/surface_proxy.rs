use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::Mutex;
use uptrakit_plugin_infrastructure_registry::{
    PluginOps, SurfaceActionContext, SurfaceActionError,
};

mod controller_local;
pub(crate) use controller_local::AppStateSurfaceActionController;
pub(crate) mod entity_enrichment;
use uuid::Uuid;

use uptrakit_wire::{ControllerMessage, surfaces};

use crate::service_connections::ServiceConnectionRegistry;
use crate::surface_registry::{SurfaceRegistry, SurfaceRegistryLookupError};

const DEFAULT_TIMEOUT_SECONDS: u16 = 30;
const MIN_TIMEOUT_SECONDS: u16 = 1;
const MAX_TIMEOUT_SECONDS: u16 = 300;
const MAX_IN_FLIGHT_PER_PROVIDER: usize = 32;
const MAX_IN_FLIGHT_PER_TENANT: usize = 128;
const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MAX_RESULT_ROWS: usize = 200;
const IDEMPOTENCY_RETENTION: Duration = Duration::from_secs(20 * 60);
const FAILURE_WINDOW: Duration = Duration::from_secs(60);
const FAILURE_LIMIT: usize = 5;
const FAILURE_COOLDOWN: Duration = Duration::from_secs(60);

const PLUGIN_TYPE_RELEASES_DOCKER: &str = "releases_docker";
const PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX: &str = "infrastructure_proxmox";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SurfaceProxyError {
    NoProvider,
    TargetProviderRequired,
    InvalidProvider(String),
    InteractionNotFound,
    PermissionDenied(String),
    Conflict { message: String, code: &'static str },
    SchemaValidationFailed(String),
    SensitiveFieldRejected(String),
    DuplicateRequest,
    RateLimited,
    ServiceDisconnected,
    SendFailed,
    Timeout,
}

impl std::fmt::Display for SurfaceProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoProvider => write!(f, "no provider available for surface interaction"),
            Self::TargetProviderRequired => {
                write!(
                    f,
                    "target_provider_id is required for targeted surface interactions"
                )
            }
            Self::InvalidProvider(provider_id) => {
                write!(
                    f,
                    "provider `{provider_id}` is not valid for this surface interaction"
                )
            }
            Self::InteractionNotFound => write!(f, "surface interaction was not found"),
            Self::PermissionDenied(message) => write!(f, "{message}"),
            Self::Conflict { message, .. } => write!(f, "{message}"),
            Self::SchemaValidationFailed(message) => write!(f, "{message}"),
            Self::SensitiveFieldRejected(message) => write!(f, "{message}"),
            Self::DuplicateRequest => write!(f, "duplicate idempotency key"),
            Self::RateLimited => write!(f, "surface provider is temporarily rate-limited"),
            Self::ServiceDisconnected => write!(f, "target service disconnected"),
            Self::SendFailed => write!(f, "failed to send proxied surface request"),
            Self::Timeout => write!(f, "surface provider timed out"),
        }
    }
}

impl std::error::Error for SurfaceProxyError {}

#[derive(Debug, Clone)]
pub enum SurfaceCallerOrigin {
    UserSession { user_id: Uuid, session_id: String },
    BuiltInSystem { principal: String },
    Provider { service_id: Uuid },
}

#[derive(Debug, Clone)]
pub struct SurfaceInvokeRequest {
    pub tenant_id: Uuid,
    pub surface_id: String,
    pub interaction_id: String,
    pub idempotency_key: String,
    pub target_provider_id: Option<String>,
    pub caller_origin: SurfaceCallerOrigin,
    pub params: serde_json::Map<String, serde_json::Value>,
    pub encrypted_sensitive_params: Option<surfaces::EncryptedSensitiveParams>,
}

#[derive(Default)]
struct PendingState {
    pending: HashMap<Uuid, PendingRequest>,
    in_flight_per_provider: HashMap<String, usize>,
    in_flight_per_tenant: HashMap<Uuid, usize>,
    in_flight_idempotency: HashMap<IdempotencyKey, IdempotencyInFlight>,
    idempotency_cache: HashMap<IdempotencyKey, CachedIdempotent>,
    provider_failures: HashMap<String, ProviderFailureState>,
}

#[derive(Debug)]
struct PendingRequest {
    provider_id: String,
    tenant_id: Uuid,
    idempotency_key: IdempotencyKey,
    sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IdempotencyKey {
    tenant_id: Uuid,
    surface_id: String,
    interaction_id: String,
    caller_key: String,
    idempotency_key: String,
}

#[derive(Debug, Clone)]
struct IdempotencyInFlight {
    request_fingerprint: u64,
}

#[derive(Debug, Clone)]
struct CachedIdempotent {
    request_fingerprint: u64,
    response: surfaces::SurfaceActionResponse,
    stored_at: std::time::Instant,
}

#[derive(Debug, Default)]
struct ProviderFailureState {
    failures: VecDeque<std::time::Instant>,
    blocked_until: Option<std::time::Instant>,
}

#[async_trait]
pub trait SurfaceLocalActionExecutor: Send + Sync {
    async fn execute(
        &self,
        _resolved: &crate::surface_registry::ResolvedSurfaceAction,
        _request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError>;
}

#[async_trait]
pub trait PluginSurfaceActionInvoker: Send + Sync {
    async fn invoke(
        &self,
        db: &(dyn std::any::Any + Send + Sync),
        tenant_id: Option<Uuid>,
        caller_user_id: Option<Uuid>,
        surface_id: &str,
        interaction_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError>;

    async fn invoke_allowlisted_notification_channel_action(
        &self,
        _db: &(dyn std::any::Any + Send + Sync),
        _tenant_id: Uuid,
        _surface_id: &str,
        _interaction_id: &str,
        _params: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<Option<serde_json::Value>, String> {
        Ok(None)
    }

    async fn invoke_allowlisted_proxmox_add_config_action(
        &self,
        _db: &(dyn std::any::Any + Send + Sync),
        _tenant_id: Uuid,
        _surface_id: &str,
        _interaction_id: &str,
        _params: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, SurfaceProxyError> {
        Ok(None)
    }
}

pub struct PluginOpsSurfaceActionInvoker {
    plugin_ops: Arc<dyn PluginOps>,
}

impl PluginOpsSurfaceActionInvoker {
    pub fn new(plugin_ops: Arc<dyn PluginOps>) -> Self {
        Self { plugin_ops }
    }
}

#[async_trait]
impl PluginSurfaceActionInvoker for PluginOpsSurfaceActionInvoker {
    async fn invoke(
        &self,
        db: &(dyn std::any::Any + Send + Sync),
        tenant_id: Option<Uuid>,
        caller_user_id: Option<Uuid>,
        surface_id: &str,
        interaction_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
        let tenant_id = tenant_id.ok_or_else(|| {
            SurfaceActionError::InvalidInput(
                "tenant_id is required for controller-local surface actions".to_string(),
            )
        })?;
        let db = db
            .downcast_ref::<sea_orm::DatabaseConnection>()
            .ok_or_else(|| {
                SurfaceActionError::ControllerIntegration(
                    "internal error: expected DatabaseConnection".to_string(),
                )
            })?;
        let controller = AppStateSurfaceActionController::from_database_connection(
            db,
            self.plugin_ops.as_ref(),
            tenant_id,
            caller_user_id,
        );
        let ctx = SurfaceActionContext {
            controller: &controller,
        };
        self.plugin_ops
            .handle_surface_action(&ctx, surface_id, interaction_id, params)
            .await
    }

    async fn invoke_allowlisted_notification_channel_action(
        &self,
        db: &(dyn std::any::Any + Send + Sync),
        tenant_id: Uuid,
        surface_id: &str,
        interaction_id: &str,
        params: &serde_json::Map<String, serde_json::Value>,
    ) -> std::result::Result<Option<serde_json::Value>, String> {
        let Some(channel_type) = notification_channel_type_from_surface(surface_id) else {
            return Ok(None);
        };
        if !matches!(interaction_id, "create" | "edit" | "test" | "delete") {
            return Ok(None);
        }

        let db = db
            .downcast_ref::<sea_orm::DatabaseConnection>()
            .ok_or_else(|| "internal error: expected DatabaseConnection".to_string())?;
        let tenant_db = uptrakit_web_api_queries::TenantDb::new(db.clone(), tenant_id);

        execute_allowlisted_notification_channel_action(
            &tenant_db,
            &*self.plugin_ops,
            channel_type,
            interaction_id,
            params,
        )
        .await
        .map(Some)
    }

    async fn invoke_allowlisted_proxmox_add_config_action(
        &self,
        db: &(dyn std::any::Any + Send + Sync),
        tenant_id: Uuid,
        surface_id: &str,
        interaction_id: &str,
        params: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<serde_json::Value>, SurfaceProxyError> {
        if !allowlisted_proxmox_add_config_controller_local_action(surface_id, interaction_id) {
            return Ok(None);
        }

        let db = db
            .downcast_ref::<sea_orm::DatabaseConnection>()
            .ok_or_else(|| {
                SurfaceProxyError::SchemaValidationFailed(
                    "internal error: expected DatabaseConnection".to_string(),
                )
            })?;
        let tenant_db = uptrakit_web_api_queries::TenantDb::new(db.clone(), tenant_id);

        execute_allowlisted_proxmox_add_config_action(&tenant_db, &*self.plugin_ops, params)
            .await
            .map(Some)
    }
}

pub struct PluginSurfaceLocalExecutor {
    action_context_db: Arc<dyn std::any::Any + Send + Sync>,
    plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>,
    audit_emitter: Option<uptrakit_audit_log::AuditEmitter>,
}

impl PluginSurfaceLocalExecutor {
    pub fn new(
        action_context_db: Arc<dyn std::any::Any + Send + Sync>,
        plugin_invoker: Arc<dyn PluginSurfaceActionInvoker>,
    ) -> Self {
        Self {
            action_context_db,
            plugin_invoker,
            audit_emitter: None,
        }
    }

    pub fn with_audit_emitter(mut self, audit_emitter: uptrakit_audit_log::AuditEmitter) -> Self {
        self.audit_emitter = Some(audit_emitter);
        self
    }
}

#[async_trait]
impl SurfaceLocalActionExecutor for PluginSurfaceLocalExecutor {
    async fn execute(
        &self,
        resolved: &crate::surface_registry::ResolvedSurfaceAction,
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

        let tenant_id = Uuid::parse_str(request.tenant_id.as_str()).map_err(|error| {
            SurfaceProxyError::SchemaValidationFailed(format!(
                "invalid tenant_id in surface action request: {error}"
            ))
        })?;
        let caller_user_id = match &request.caller_origin {
            surfaces::CallerOrigin::UserSession { user_id, .. } => {
                Some(Uuid::parse_str(user_id.as_str()).map_err(|error| {
                    SurfaceProxyError::SchemaValidationFailed(format!(
                        "invalid caller user_id in surface action request: {error}"
                    ))
                })?)
            }
            _ => None,
        };

        if let Some(channel_type) = allowlisted_notification_channel_controller_local_action(
            resolved.provider_id.as_str(),
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        ) {
            let invoke_result = self
                .plugin_invoker
                .invoke_allowlisted_notification_channel_action(
                    self.action_context_db.as_ref(),
                    tenant_id,
                    resolved.descriptor.surface_id.as_str(),
                    resolved.interaction.interaction_id.as_str(),
                    &request.params,
                )
                .await;
            let result = match invoke_result {
                Ok(Some(result)) => {
                    emit_notification_channel_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        resolved.interaction.interaction_id.as_str(),
                        channel_type,
                        &request.params,
                        Ok(&result),
                    );
                    result
                }
                Ok(None) => {
                    let error = SurfaceProxyError::SchemaValidationFailed(
                        "allowlisted notification controller_local action is unavailable"
                            .to_string(),
                    );
                    emit_notification_channel_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        resolved.interaction.interaction_id.as_str(),
                        channel_type,
                        &request.params,
                        Err(&error),
                    );
                    return Err(error);
                }
                Err(error) => {
                    let error = SurfaceProxyError::SchemaValidationFailed(error);
                    emit_notification_channel_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        resolved.interaction.interaction_id.as_str(),
                        channel_type,
                        &request.params,
                        Err(&error),
                    );
                    return Err(error);
                }
            };
            return Ok(result);
        }

        if let Some(settings_action) = allowlisted_notification_settings_controller_local_action(
            resolved.provider_id.as_str(),
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        ) {
            let invoke_result = self
                .plugin_invoker
                .invoke(
                    self.action_context_db.as_ref(),
                    Some(tenant_id),
                    caller_user_id,
                    resolved.descriptor.surface_id.as_str(),
                    resolved.interaction.interaction_id.as_str(),
                    serde_json::Value::Object(request.params.clone()),
                )
                .await;
            let result = match invoke_result {
                Ok(result) => {
                    emit_notification_settings_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        settings_action,
                        &request.params,
                        Ok(&result),
                    );
                    result
                }
                Err(error) => {
                    let error = SurfaceProxyError::SchemaValidationFailed(error.to_string());
                    emit_notification_settings_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        settings_action,
                        &request.params,
                        Err(&error),
                    );
                    return Err(error);
                }
            };
            return Ok(result);
        }

        if allowlisted_docker_switch_tag_controller_local_action(
            resolved.provider_id.as_str(),
            resolved.descriptor.surface_id.as_str(),
            resolved.interaction.interaction_id.as_str(),
        ) {
            let invoke_result = self
                .plugin_invoker
                .invoke(
                    self.action_context_db.as_ref(),
                    Some(tenant_id),
                    caller_user_id,
                    resolved.descriptor.surface_id.as_str(),
                    resolved.interaction.interaction_id.as_str(),
                    serde_json::Value::Object(request.params.clone()),
                )
                .await;
            let result = match invoke_result {
                Ok(result) => {
                    emit_docker_switch_tag_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        &request.params,
                        Ok(&result),
                    );
                    result
                }
                Err(error) => {
                    let error = SurfaceProxyError::SchemaValidationFailed(error.to_string());
                    emit_docker_switch_tag_audit_event(
                        self.audit_emitter.as_ref(),
                        caller_user_id,
                        tenant_id,
                        &request.params,
                        Err(&error),
                    );
                    return Err(error);
                }
            };
            return Ok(result);
        }

        if allowlisted_proxmox_provider(resolved.provider_id.as_str()) {
            if let Some(update_protection_action) =
                allowlisted_proxmox_update_protection_controller_local_action(
                    resolved.descriptor.surface_id.as_str(),
                    resolved.interaction.interaction_id.as_str(),
                )
            {
                let invoke_result = self
                    .plugin_invoker
                    .invoke(
                        self.action_context_db.as_ref(),
                        Some(tenant_id),
                        caller_user_id,
                        resolved.descriptor.surface_id.as_str(),
                        resolved.interaction.interaction_id.as_str(),
                        serde_json::Value::Object(request.params.clone()),
                    )
                    .await;
                let result = match invoke_result {
                    Ok(result) => {
                        emit_proxmox_update_protection_audit_event(
                            self.audit_emitter.as_ref(),
                            caller_user_id,
                            tenant_id,
                            update_protection_action,
                            &request.params,
                            Ok(&result),
                        );
                        result
                    }
                    Err(error) => {
                        let error = SurfaceProxyError::SchemaValidationFailed(error.to_string());
                        emit_proxmox_update_protection_audit_event(
                            self.audit_emitter.as_ref(),
                            caller_user_id,
                            tenant_id,
                            update_protection_action,
                            &request.params,
                            Err(&error),
                        );
                        return Err(error);
                    }
                };
                return Ok(result);
            }

            if allowlisted_proxmox_add_config_controller_local_action(
                resolved.descriptor.surface_id.as_str(),
                resolved.interaction.interaction_id.as_str(),
            ) {
                let invoke_result = self
                    .plugin_invoker
                    .invoke_allowlisted_proxmox_add_config_action(
                        self.action_context_db.as_ref(),
                        tenant_id,
                        resolved.descriptor.surface_id.as_str(),
                        resolved.interaction.interaction_id.as_str(),
                        &request.params,
                    )
                    .await;
                let result = match invoke_result {
                    Ok(Some(result)) => {
                        emit_proxmox_add_config_audit_event(
                            self.audit_emitter.as_ref(),
                            caller_user_id,
                            tenant_id,
                            &request.params,
                            Ok(&result),
                        );
                        result
                    }
                    Ok(None) => {
                        let error = SurfaceProxyError::SchemaValidationFailed(
                            "allowlisted proxmox controller_local action is unavailable"
                                .to_string(),
                        );
                        emit_proxmox_add_config_audit_event(
                            self.audit_emitter.as_ref(),
                            caller_user_id,
                            tenant_id,
                            &request.params,
                            Err(&error),
                        );
                        return Err(error);
                    }
                    Err(error) => {
                        emit_proxmox_add_config_audit_event(
                            self.audit_emitter.as_ref(),
                            caller_user_id,
                            tenant_id,
                            &request.params,
                            Err(&error),
                        );
                        return Err(error);
                    }
                };
                return Ok(result);
            }
        }

        self.plugin_invoker
            .invoke(
                self.action_context_db.as_ref(),
                Some(tenant_id),
                caller_user_id,
                request.surface_id.as_str(),
                request.interaction_id.as_str(),
                serde_json::Value::Object(request.params.clone()),
            )
            .await
            .map_err(|e| SurfaceProxyError::SchemaValidationFailed(e.to_string()))
    }
}

fn notification_channel_type_from_surface(surface_id: &str) -> Option<&str> {
    surface_id
        .strip_prefix("notifications.")
        .and_then(|s| s.split('.').next())
        .filter(|s| !s.is_empty())
}

fn allowlisted_notification_channel_controller_local_action<'a>(
    provider_id: &str,
    surface_id: &'a str,
    interaction_id: &str,
) -> Option<&'a str> {
    if !matches!(interaction_id, "create" | "edit" | "test" | "delete") {
        return None;
    }
    let channel_type = notification_channel_type_from_surface(surface_id)?;
    (provider_id.strip_prefix("plugin.") == Some(channel_type)).then_some(channel_type)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotificationSettingsAction {
    ConfigureSmtp,
    SaveGlobalSmtp,
    SaveGlobalTelegram,
}

fn allowlisted_notification_settings_controller_local_action(
    provider_id: &str,
    surface_id: &str,
    interaction_id: &str,
) -> Option<NotificationSettingsAction> {
    let channel_type = notification_channel_type_from_surface(surface_id)?;
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

fn allowlisted_proxmox_provider(provider_id: &str) -> bool {
    matches!(
        provider_id,
        "plugin.infrastructure_proxmox" | "infrastructure_proxmox"
    )
}

fn allowlisted_docker_switch_tag_controller_local_action(
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
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        );
    }

    if message.contains("no plugin assignments found for this host")
        || message.contains("host_software_item not found for host")
    {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "host_assignment_not_found",
        );
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

fn emit_docker_switch_tag_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };

    let requested_software_item_id = request_params
        .get("software_item_id")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);
    let requested_host_id = request_params
        .get("host_id")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);
    let requested_new_image_ref = request_params
        .get("new_image_ref")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
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

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
    )
    .tenant_scope(tenant_id)
    .actor(
        uptrakit_audit_log::AuditActorType::User,
        Some(caller_user_id),
    )
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxmoxUpdateProtectionAction {
    SaveGlobalDefaults,
    SaveItemOverrides,
}

fn allowlisted_proxmox_update_protection_controller_local_action(
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

fn allowlisted_proxmox_add_config_controller_local_action(
    surface_id: &str,
    interaction_id: &str,
) -> bool {
    surface_id == "proxmox.hosts" && interaction_id == "add-config"
}

async fn execute_allowlisted_notification_channel_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    channel_type: &str,
    interaction_id: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    use uptrakit_web_api_types::validation::Validate as _;

    match interaction_id {
        "create" => {
            let req = build_notification_channel_create_request(channel_type, params)?;
            req.validate().map_err(|error| error.to_string())?;
            let response =
                crate::queries::notifications::create_channel(tenant_db, &req, plugin_ops)
                    .await
                    .map_err(|error| error.to_string())?;
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize create response: {error}"))
        }
        "edit" => {
            let channel_id = required_uuid_param(params, "id")?;
            require_notification_channel_type(tenant_db, channel_id, channel_type).await?;
            let req = build_notification_channel_update_request(params)?;
            req.validate().map_err(|error| error.to_string())?;
            let response = crate::queries::notifications::update_channel(
                tenant_db, channel_id, &req, plugin_ops,
            )
            .await
            .map_err(|error| error.to_string())?;
            let Some(response) = response else {
                return Err("Channel not found".to_string());
            };
            serde_json::to_value(response)
                .map_err(|error| format!("failed to serialize update response: {error}"))
        }
        "delete" => {
            let channel_id = required_uuid_param(params, "id")?;
            require_notification_channel_type(tenant_db, channel_id, channel_type).await?;
            let deleted = crate::queries::notifications::delete_channel(tenant_db, channel_id)
                .await
                .map_err(|error| error.to_string())?;
            if !deleted {
                return Err("Channel not found".to_string());
            }
            Ok(serde_json::json!({}))
        }
        "test" => {
            let channel_id = required_uuid_param(params, "id")?;
            execute_notification_channel_test_action(
                tenant_db,
                plugin_ops,
                channel_id,
                channel_type,
            )
            .await
        }
        _ => Err(format!(
            "action `{interaction_id}` is not allowlisted for notification controller_local execution"
        )),
    }
}

async fn execute_allowlisted_proxmox_add_config_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, SurfaceProxyError> {
    use uptrakit_web_api_types::validation::Validate as _;

    let request = build_proxmox_add_config_create_request(params)
        .map_err(SurfaceProxyError::SchemaValidationFailed)?;
    request
        .validate()
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?;
    plugin_ops
        .validate_config(&request.plugin_type, &request.config)
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?;
    let response = uptrakit_web_api_queries::queries::plugin_configs::create_plugin_config(
        plugin_ops, tenant_db, request,
    )
    .await
    .map_err(|error| match error.current_context() {
        uptrakit_web_api_queries::queries::plugin_configs::PluginConfigError::DuplicateName => {
            SurfaceProxyError::Conflict {
                message: error.to_string(),
                code: "duplicate_name",
            }
        }
        _ => SurfaceProxyError::SchemaValidationFailed(error.to_string()),
    })?;
    serde_json::to_value(response).map_err(|error| {
        SurfaceProxyError::SchemaValidationFailed(format!(
            "failed to serialize proxmox add-config response: {error}"
        ))
    })
}

fn classify_proxmox_add_config_error(
    error: &SurfaceProxyError,
) -> (
    uptrakit_audit_log::AuditOutcome,
    &'static str,
    Option<&'static str>,
) {
    match error {
        SurfaceProxyError::SchemaValidationFailed(_)
        | SurfaceProxyError::SensitiveFieldRejected(_) => (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "validation_failed",
            None,
        ),
        SurfaceProxyError::Conflict { code, .. } => (
            uptrakit_audit_log::AuditOutcome::Failed,
            code,
            Some("conflict"),
        ),
        _ => (uptrakit_audit_log::AuditOutcome::Failed, "failed", None),
    }
}

fn emit_proxmox_add_config_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };
    let requested_name = request_params
        .get("name")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, error_kind, target_id, target_display, plugin_type) = match result {
        Ok(result) => {
            let Some(plugin_config_id) = result.get("id").and_then(|value| value.as_str()) else {
                return;
            };
            let config_name = result
                .get("name")
                .and_then(|value| value.as_str())
                .map(std::string::ToString::to_string)
                .or(requested_name.clone());
            let plugin_type = result
                .get("plugin_type")
                .and_then(|value| value.as_str())
                .unwrap_or(PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX);
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
                PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX.to_string(),
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

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .tenant_scope(tenant_id)
    .actor(
        uptrakit_audit_log::AuditActorType::User,
        Some(caller_user_id),
    )
    .target_opt(Some("plugin_config".to_string()), target_id, target_display)
    .outcome(outcome)
    .details(details)
    .build()
    {
        audit_emitter.emit_best_effort(entry);
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
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        );
    }

    if message.contains("not found in tenant scope")
        || message.contains("not assigned to software item")
        || message.contains("not present in cache")
    {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "resource_not_available",
        );
    }

    if message.contains("failed to save")
        || message.contains("failed to clear")
        || message.contains("database error")
    {
        return (uptrakit_audit_log::AuditOutcome::Failed, "storage_error");
    }

    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

fn emit_proxmox_update_protection_audit_event(
    audit_emitter: Option<&uptrakit_audit_log::AuditEmitter>,
    caller_user_id: Option<Uuid>,
    tenant_id: Uuid,
    action: ProxmoxUpdateProtectionAction,
    request_params: &serde_json::Map<String, serde_json::Value>,
    result: Result<&serde_json::Value, &SurfaceProxyError>,
) {
    let Some(audit_emitter) = audit_emitter else {
        return;
    };
    let Some(caller_user_id) = caller_user_id else {
        return;
    };

    let requested_plugin_config_id = request_params
        .get("plugin_config_id")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);
    let requested_software_item_id = request_params
        .get("software_item_id")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);
    let requested_mode = request_params
        .get("mode")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, target_type, target_id, details_target_plugin_config_id) =
        match (action, result) {
            (ProxmoxUpdateProtectionAction::SaveGlobalDefaults, Ok(response)) => (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                Some("plugin_config".to_string()),
                response
                    .get("plugin_config_id")
                    .and_then(|value| value.as_str())
                    .map(std::string::ToString::to_string)
                    .or(requested_plugin_config_id.clone()),
                response
                    .get("plugin_config_id")
                    .and_then(|value| value.as_str())
                    .map(std::string::ToString::to_string)
                    .or(requested_plugin_config_id.clone()),
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
                    .and_then(|value| value.as_str())
                    .map(std::string::ToString::to_string)
                    .or(requested_software_item_id.clone()),
                response
                    .get("plugin_config_id")
                    .and_then(|value| value.as_str())
                    .map(std::string::ToString::to_string)
                    .or(requested_plugin_config_id.clone()),
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
        && let Some(cleared) = response.get("cleared").and_then(|value| value.as_bool())
    {
        details["cleared"] = serde_json::json!(cleared);
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::json!(reason_code);
    }

    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::builder(proxmox_update_protection_action_type(action))
            .tenant_scope(tenant_id)
            .actor(
                uptrakit_audit_log::AuditActorType::User,
                Some(caller_user_id),
            )
            .target_opt(target_type, target_id, None)
            .outcome(outcome)
            .details(details)
            .build()
    {
        audit_emitter.emit_best_effort(entry);
    }
}

fn notification_channel_action_type(
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
            (
                uptrakit_audit_log::AuditOutcome::Failed,
                "channel_not_found",
            )
        } else {
            (
                uptrakit_audit_log::AuditOutcome::Denied,
                "channel_not_found",
            )
        };
    }
    if message.contains("Channel type mismatch") {
        return (
            uptrakit_audit_log::AuditOutcome::Denied,
            "channel_type_mismatch",
        );
    }
    if message.contains("Unsupported channel type") {
        return (
            uptrakit_audit_log::AuditOutcome::Failed,
            "unsupported_channel_type",
        );
    }
    if message.contains("Failed to parse channel config") {
        return (
            uptrakit_audit_log::AuditOutcome::Failed,
            "channel_config_parse_failed",
        );
    }
    if message.contains("field `")
        || message.contains("invalid")
        || message.contains("must be")
        || matches!(error, SurfaceProxyError::SensitiveFieldRejected(_))
    {
        return (
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            "invalid_request",
        );
    }

    (uptrakit_audit_log::AuditOutcome::Failed, "failed")
}

fn emit_notification_channel_audit_event(
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
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);
    let requested_name = request_params
        .get("name")
        .and_then(|value| value.as_str())
        .map(std::string::ToString::to_string);

    let (outcome, reason_code, target_id, target_display) = match result {
        Ok(response) => {
            let target_id = response
                .get("id")
                .and_then(|value| value.as_str())
                .map(std::string::ToString::to_string)
                .or(requested_id.clone())
                .or_else(|| (interaction_id == "create").then(|| "pending".to_string()));
            let target_display = response
                .get("name")
                .and_then(|value| value.as_str())
                .map(std::string::ToString::to_string)
                .or(requested_name.clone());
            (
                uptrakit_audit_log::AuditOutcome::Success,
                None,
                target_id,
                target_display,
            )
        }
        Err(error) => {
            let (outcome, reason_code) = classify_notification_channel_error(interaction_id, error);
            let target_id = requested_id
                .clone()
                .or_else(|| (interaction_id == "create").then(|| "pending".to_string()));
            (
                outcome,
                Some(reason_code),
                target_id,
                requested_name.clone(),
            )
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
        .actor(
            uptrakit_audit_log::AuditActorType::User,
            Some(caller_user_id),
        )
        .target_opt(
            Some("notification_channel".to_string()),
            target_id,
            target_display,
        )
        .outcome(outcome)
        .details(details)
        .build()
    {
        audit_emitter.emit_best_effort(entry);
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

fn emit_notification_settings_audit_event(
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

async fn execute_notification_channel_test_action(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    plugin_ops: &dyn PluginOps,
    channel_id: Uuid,
    expected_channel_type: &str,
) -> Result<serde_json::Value, String> {
    let channel =
        require_notification_channel_type(tenant_db, channel_id, expected_channel_type).await?;
    let config_json: serde_json::Value = serde_json::from_str(channel.config.expose_secret())
        .map_err(|error| format!("Failed to parse channel config: {error}"))?;
    let channel_type_id = uptrakit_shared_types::PluginTypeId::new(&channel.channel_type);
    let channel_transport = plugin_ops
        .transport(&channel_type_id)
        .ok_or_else(|| format!("Unsupported channel type: {}", channel.channel_type))?;

    let settings_bag =
        crate::notifications::dispatcher::build_settings_bag(tenant_db.db(), tenant_db.tenant_id)
            .await;
    let test_msg = uptrakit_plugin_infrastructure_registry::DeliveryMessage::new(
        "Test Notification",
        "This is a test notification from Uptrakit.",
        None,
        serde_json::json!({"test": true}),
        vec![],
    );

    channel_transport
        .deliver(&config_json, &settings_bag, &test_msg)
        .await
        .map_err(|error| error.to_string())?;

    serde_json::to_value(
        uptrakit_web_api_types::notifications::TestNotificationResponse {
            success: true,
            message: "Test notification delivered successfully".to_string(),
        },
    )
    .map_err(|error| format!("failed to serialize test response: {error}"))
}

async fn require_notification_channel_type(
    tenant_db: &uptrakit_web_api_queries::TenantDb,
    channel_id: Uuid,
    expected_channel_type: &str,
) -> Result<uptrakit_shared_db::entity::notification_channel::Model, String> {
    let model = tenant_db
        .find_by_id::<uptrakit_shared_db::entity::notification_channel::Entity, _>(channel_id)
        .one(tenant_db.db())
        .await
        .map_err(|error| format!("failed to load notification channel: {error}"))?;
    let Some(model) = model else {
        return Err("Channel not found".to_string());
    };
    if model.channel_type != expected_channel_type {
        return Err("Channel type mismatch for selected notification surface".to_string());
    }
    Ok(model)
}

fn build_notification_channel_create_request(
    channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::CreateNotificationChannelRequest, String> {
    validate_or_reject_mismatched_channel_type(channel_type, params)?;

    Ok(
        uptrakit_web_api_types::notifications::CreateNotificationChannelRequest {
            name: required_string_param(params, "name")?,
            channel_type: channel_type.to_string(),
            config: serde_json::from_value(resolve_notification_channel_config(params)?)
                .map_err(|e| e.to_string())?,
            enabled: strict_bool_param_with_default(params, "enabled", true)?,
        },
    )
}

fn build_proxmox_add_config_create_request(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest, String> {
    Ok(
        uptrakit_web_api_types::plugin_configs::CreatePluginConfigRequest {
            name: required_string_param(params, "name")?,
            plugin_type: uptrakit_shared_types::PluginTypeId::from_static(
                PLUGIN_TYPE_INFRASTRUCTURE_PROXMOX,
            ),
            config: resolve_proxmox_add_config(params)?,
            enabled: true,
        },
    )
}

fn build_notification_channel_update_request(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest, String> {
    Ok(
        uptrakit_web_api_types::notifications::UpdateNotificationChannelRequest {
            name: optional_string_param(params, "name")?,
            config: Some(
                serde_json::from_value(resolve_notification_channel_config(params)?)
                    .map_err(|e| e.to_string())?,
            ),
            enabled: strict_optional_bool_param(params, "enabled")?,
        },
    )
}

fn validate_or_reject_mismatched_channel_type(
    expected_channel_type: &str,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), String> {
    let Some(channel_type) = params.get("channel_type") else {
        return Ok(());
    };
    let Some(channel_type) = channel_type.as_str() else {
        return Err("field `channel_type` must be a string".to_string());
    };
    if channel_type != expected_channel_type {
        return Err(format!(
            "field `channel_type` must be `{expected_channel_type}` for this surface"
        ));
    }
    Ok(())
}

fn resolve_notification_channel_config(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let config = params
        .get("config")
        .ok_or_else(|| "field `config` is required".to_string())?;
    if !config.is_object() {
        return Err("field `config` must be a JSON object".to_string());
    }
    Ok(config.clone())
}

fn resolve_proxmox_add_config(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    if let Some(config) = params.get("config") {
        let Some(config) = config.as_object() else {
            return Err("field `config` must be a JSON object".to_string());
        };
        return build_proxmox_config_from_params(config);
    }
    build_proxmox_config_from_params(params)
}

fn build_proxmox_config_from_params(
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "api_url": required_string_param(params, "api_url")?,
        "api_token": required_string_param(params, "api_token")?,
        "verify_tls": proxmox_verify_tls_param_with_default(params, "verify_tls", true)?,
        "node_filter": parse_csv_array_or_string_array_param(params, "node_filter")?,
    }))
}

fn required_string_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<String, String> {
    let Some(value) = params.get(key) else {
        return Err(format!("missing required field `{key}`"));
    };
    let Some(value) = value.as_str() else {
        return Err(format!("field `{key}` must be a string"));
    };
    if value.trim().is_empty() {
        return Err(format!("field `{key}` must not be empty"));
    }
    Ok(value.to_string())
}

fn optional_string_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_str() else {
        return Err(format!("field `{key}` must be a string"));
    };
    Ok(Some(value.to_string()))
}

fn required_uuid_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Uuid, String> {
    let value = required_string_param(params, key)?;
    Uuid::parse_str(value.as_str())
        .map_err(|error| format!("field `{key}` must be a UUID: {error}"))
}

fn strict_bool_param_with_default(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = params.get(key) else {
        return Ok(default);
    };
    let Some(value) = value.as_bool() else {
        return Err(format!("field `{key}` must be a boolean"));
    };
    Ok(value)
}

fn strict_optional_bool_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<bool>, String> {
    let Some(value) = params.get(key) else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let Some(value) = value.as_bool() else {
        return Err(format!("field `{key}` must be a boolean"));
    };
    Ok(Some(value))
}

fn proxmox_verify_tls_param_with_default(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    let Some(value) = params.get(key) else {
        return Ok(default);
    };
    match value {
        serde_json::Value::Bool(value) => Ok(*value),
        serde_json::Value::String(value) => match value.trim() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!(
                "field `{key}` must be a boolean or the string `true`/`false`"
            )),
        },
        _ => Err(format!(
            "field `{key}` must be a boolean or the string `true`/`false`"
        )),
    }
}

fn parse_csv_array_or_string_array_param(
    params: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(value) = params.get(key) else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }

    match value {
        serde_json::Value::String(text) => Ok(text
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect()),
        serde_json::Value::Array(values) => {
            let mut parsed = Vec::new();
            for value in values {
                let Some(value) = value.as_str() else {
                    return Err(format!("field `{key}` array entries must be strings"));
                };
                let value = value.trim();
                if !value.is_empty() {
                    parsed.push(value.to_string());
                }
            }
            Ok(parsed)
        }
        _ => Err(format!(
            "field `{key}` must be either a csv string or an array of strings"
        )),
    }
}

struct NoopSurfaceLocalExecutor;

#[async_trait]
impl SurfaceLocalActionExecutor for NoopSurfaceLocalExecutor {
    async fn execute(
        &self,
        resolved: &crate::surface_registry::ResolvedSurfaceAction,
        _request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError> {
        Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "local surface transport is not implemented for provider `{}`",
            resolved.provider_id
        )))
    }
}

pub struct SurfaceProxy {
    pending: Arc<Mutex<PendingState>>,
    local_executor: Arc<dyn SurfaceLocalActionExecutor>,
}

impl Default for SurfaceProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceProxy {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(Mutex::new(PendingState::default())),
            local_executor: Arc::new(NoopSurfaceLocalExecutor),
        }
    }

    pub fn with_local_executor(
        mut self,
        local_executor: Arc<dyn SurfaceLocalActionExecutor>,
    ) -> Self {
        self.local_executor = local_executor;
        self
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn invoke(
        &self,
        service_connections: &ServiceConnectionRegistry,
        registry: &SurfaceRegistry,
        request: SurfaceInvokeRequest,
        timeout_override: Option<Duration>,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        self.invoke_inner(service_connections, registry, request, timeout_override)
            .await
    }

    async fn invoke_inner(
        &self,
        service_connections: &ServiceConnectionRegistry,
        registry: &SurfaceRegistry,
        request: SurfaceInvokeRequest,
        timeout_override: Option<Duration>,
    ) -> Result<surfaces::SurfaceActionResponse, SurfaceProxyError> {
        let target_provider_id =
            implicit_target_provider_for_request(service_connections, registry, &request).await?;
        let resolved = registry
            .resolve_surface_action(
                request.tenant_id,
                &request.surface_id,
                &request.interaction_id,
                target_provider_id.as_deref(),
            )
            .map_err(map_lookup_error)?;

        let caller_origin =
            caller_origin_for_request(registry, &request.caller_origin, &resolved, &request)?;

        validate_input_schema(&resolved.interaction, &request.params)?;
        validate_sensitive_fields(
            &resolved.interaction,
            &request.params,
            request.encrypted_sensitive_params.as_ref(),
        )?;

        let timeout = resolve_timeout(timeout_override, &resolved.interaction)?;
        let request_fingerprint =
            fingerprint_request(&request.params, request.encrypted_sensitive_params.as_ref());
        let idem_key = build_idempotency_key(&request, &caller_origin);

        if matches!(&caller_origin, surfaces::CallerOrigin::Provider { .. })
            && resolved.interaction.required_permission.is_some()
        {
            return Err(SurfaceProxyError::PermissionDenied(
                "provider-initiated requests cannot satisfy user permission gates".to_string(),
            ));
        }

        if let Some(cached) = self.try_get_cached_response(&idem_key, request_fingerprint) {
            return Ok(cached);
        }

        match &resolved.interaction.transport {
            surfaces::InteractionTransport::ControllerLocal
            | surfaces::InteractionTransport::DirectBuiltInApi { .. } => {
                if matches!(
                    resolved.interaction.transport,
                    surfaces::InteractionTransport::ControllerLocal
                ) && resolved.provider_kind != surfaces::ProviderKind::Plugin
                {
                    return Err(SurfaceProxyError::SchemaValidationFailed(
                        "controller_local transport is only supported for plugin providers"
                            .to_string(),
                    ));
                }

                {
                    let mut state = self.pending.lock();
                    state.cleanup_expired();
                    state.ensure_idempotency_available(&idem_key, request_fingerprint)?;
                    state.reserve_idempotency(idem_key.clone(), request_fingerprint);
                }

                let local_request = surfaces::SurfaceActionRequest {
                    request_id: Uuid::now_v7(),
                    tenant_id: request.tenant_id.to_string(),
                    surface_id: resolved.descriptor.surface_id.clone(),
                    interaction_id: resolved.interaction.interaction_id.clone(),
                    idempotency_key: request.idempotency_key.clone(),
                    target_provider_id: Some(resolved.provider_id.clone()),
                    caller_origin,
                    params: request.params.clone(),
                    encrypted_sensitive_params: request.encrypted_sensitive_params.clone(),
                };
                let local_result = self.local_executor.execute(&resolved, &local_request).await;
                {
                    let mut state = self.pending.lock();
                    state.release_idempotency(&idem_key);
                }
                let result = local_result?;
                validate_result_schema(&resolved.interaction, Some(&result))?;
                validate_result_limits(&result)?;
                let response = surfaces::SurfaceActionResponse {
                    request_id: local_request.request_id,
                    success: true,
                    result: Some(result),
                    error: None,
                };
                self.store_cached_response(idem_key, request_fingerprint, response.clone());
                Ok(response)
            }
            surfaces::InteractionTransport::ProviderProxied => {
                let Some(service_id) = resolved.service_id else {
                    return Err(SurfaceProxyError::NoProvider);
                };
                if !service_connections.is_connected(&service_id).await
                    || service_connections.is_yielded(&service_id)
                {
                    return Err(SurfaceProxyError::NoProvider);
                }

                let request_id = Uuid::now_v7();
                let (tx, rx) = tokio::sync::oneshot::channel();

                {
                    let mut state = self.pending.lock();
                    state.cleanup_expired();
                    state.ensure_provider_not_rate_limited(&resolved.provider_id)?;
                    state.ensure_budget(&resolved.provider_id, request.tenant_id)?;
                    state.ensure_idempotency_available(&idem_key, request_fingerprint)?;
                    state.register_pending(
                        request_id,
                        &resolved.provider_id,
                        request.tenant_id,
                        idem_key.clone(),
                        request_fingerprint,
                        tx,
                    );
                }

                let outbound = surfaces::SurfaceActionRequest {
                    request_id,
                    tenant_id: request.tenant_id.to_string(),
                    surface_id: resolved.descriptor.surface_id.clone(),
                    interaction_id: resolved.interaction.interaction_id.clone(),
                    idempotency_key: request.idempotency_key.clone(),
                    target_provider_id: Some(resolved.provider_id.clone()),
                    caller_origin,
                    params: request.params.clone(),
                    encrypted_sensitive_params: request.encrypted_sensitive_params.clone(),
                };

                let sent = service_connections
                    .send(
                        &service_id,
                        ControllerMessage::SurfaceActionRequest(outbound),
                    )
                    .await;
                if !sent {
                    self.fail_pending_request(&resolved.provider_id, request_id);
                    return Err(SurfaceProxyError::SendFailed);
                }

                let response = match tokio::time::timeout(timeout, rx).await {
                    Ok(Ok(response)) => response,
                    Ok(Err(_)) => {
                        self.record_provider_failure(&resolved.provider_id);
                        return Err(SurfaceProxyError::ServiceDisconnected);
                    }
                    Err(_) => {
                        self.timeout_pending_request(
                            service_connections,
                            service_id,
                            &resolved.provider_id,
                            request_id,
                        )
                        .await;
                        return Err(SurfaceProxyError::Timeout);
                    }
                };

                if response.success {
                    validate_result_schema(&resolved.interaction, response.result.as_ref())?;
                    if let Some(result) = response.result.as_ref() {
                        validate_result_limits(result)?;
                    }
                }

                self.store_cached_response(idem_key, request_fingerprint, response.clone());
                Ok(response)
            }
        }
    }

    pub fn complete(&self, request_id: Uuid, response: surfaces::SurfaceActionResponse) {
        let sender = self.pending.lock().take_pending(&request_id);
        if let Some(sender) = sender {
            let _ = sender.send(response);
        }
    }

    pub fn fail_in_flight_for_provider(&self, provider_id: &str) {
        let mut state = self.pending.lock();
        let request_ids: Vec<Uuid> = state
            .pending
            .iter()
            .filter_map(|(request_id, pending)| {
                if pending.provider_id == provider_id {
                    Some(*request_id)
                } else {
                    None
                }
            })
            .collect();

        for request_id in &request_ids {
            let _ = state.remove_pending(request_id);
        }
        if !request_ids.is_empty() {
            state.record_provider_failure(provider_id);
        }
    }

    async fn timeout_pending_request(
        &self,
        service_connections: &ServiceConnectionRegistry,
        service_id: Uuid,
        provider_id: &str,
        request_id: Uuid,
    ) {
        let removed = {
            let mut state = self.pending.lock();
            state.remove_pending(&request_id)
        };
        if removed {
            let _ = service_connections
                .send(
                    &service_id,
                    ControllerMessage::SurfaceActionCancel(surfaces::SurfaceActionCancel {
                        request_id,
                        target_provider_id: provider_id.to_string(),
                        reason: surfaces::SurfaceActionCancelReason::Timeout,
                    }),
                )
                .await;
            self.record_provider_failure(provider_id);
        }
    }

    fn fail_pending_request(&self, provider_id: &str, request_id: Uuid) {
        let removed = {
            let mut state = self.pending.lock();
            state.remove_pending(&request_id)
        };
        if removed {
            self.record_provider_failure(provider_id);
        }
    }

    fn record_provider_failure(&self, provider_id: &str) {
        let mut state = self.pending.lock();
        state.record_provider_failure(provider_id);
    }

    fn try_get_cached_response(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Option<surfaces::SurfaceActionResponse> {
        let mut state = self.pending.lock();
        state.cleanup_expired();
        let cached = state.idempotency_cache.get(key)?;
        if cached.request_fingerprint == request_fingerprint {
            return Some(cached.response.clone());
        }
        None
    }

    fn store_cached_response(
        &self,
        key: IdempotencyKey,
        request_fingerprint: u64,
        response: surfaces::SurfaceActionResponse,
    ) {
        let mut state = self.pending.lock();
        state.cleanup_expired();
        state.idempotency_cache.insert(
            key,
            CachedIdempotent {
                request_fingerprint,
                response,
                stored_at: std::time::Instant::now(),
            },
        );
    }
}

impl PendingState {
    fn register_pending(
        &mut self,
        request_id: Uuid,
        provider_id: &str,
        tenant_id: Uuid,
        idempotency_key: IdempotencyKey,
        request_fingerprint: u64,
        sender: tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>,
    ) {
        self.pending.insert(
            request_id,
            PendingRequest {
                provider_id: provider_id.to_string(),
                tenant_id,
                idempotency_key: idempotency_key.clone(),
                sender,
            },
        );
        *self
            .in_flight_per_provider
            .entry(provider_id.to_string())
            .or_default() += 1;
        *self.in_flight_per_tenant.entry(tenant_id).or_default() += 1;
        self.reserve_idempotency(idempotency_key, request_fingerprint);
    }

    fn take_pending(
        &mut self,
        request_id: &Uuid,
    ) -> Option<tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>> {
        let pending = self.pending.remove(request_id)?;
        decrement_counter(&mut self.in_flight_per_provider, &pending.provider_id);
        decrement_counter(&mut self.in_flight_per_tenant, &pending.tenant_id);
        self.in_flight_idempotency.remove(&pending.idempotency_key);
        Some(pending.sender)
    }

    fn remove_pending(&mut self, request_id: &Uuid) -> bool {
        self.take_pending(request_id).is_some()
    }

    fn reserve_idempotency(&mut self, key: IdempotencyKey, request_fingerprint: u64) {
        self.in_flight_idempotency.insert(
            key,
            IdempotencyInFlight {
                request_fingerprint,
            },
        );
    }

    fn release_idempotency(&mut self, key: &IdempotencyKey) {
        self.in_flight_idempotency.remove(key);
    }

    fn ensure_budget(&self, provider_id: &str, tenant_id: Uuid) -> Result<(), SurfaceProxyError> {
        if self
            .in_flight_per_provider
            .get(provider_id)
            .copied()
            .unwrap_or(0)
            >= MAX_IN_FLIGHT_PER_PROVIDER
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        if self
            .in_flight_per_tenant
            .get(&tenant_id)
            .copied()
            .unwrap_or(0)
            >= MAX_IN_FLIGHT_PER_TENANT
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        Ok(())
    }

    fn ensure_idempotency_available(
        &self,
        key: &IdempotencyKey,
        request_fingerprint: u64,
    ) -> Result<(), SurfaceProxyError> {
        if let Some(in_flight) = self.in_flight_idempotency.get(key) {
            if in_flight.request_fingerprint == request_fingerprint {
                return Err(SurfaceProxyError::DuplicateRequest);
            }
            return Err(SurfaceProxyError::DuplicateRequest);
        }
        if let Some(cached) = self.idempotency_cache.get(key)
            && cached.request_fingerprint != request_fingerprint
        {
            return Err(SurfaceProxyError::DuplicateRequest);
        }
        Ok(())
    }

    fn ensure_provider_not_rate_limited(&self, provider_id: &str) -> Result<(), SurfaceProxyError> {
        let now = std::time::Instant::now();
        if let Some(state) = self.provider_failures.get(provider_id)
            && let Some(blocked_until) = state.blocked_until
            && blocked_until > now
        {
            return Err(SurfaceProxyError::RateLimited);
        }
        Ok(())
    }

    fn record_provider_failure(&mut self, provider_id: &str) {
        let now = std::time::Instant::now();
        let tracker = self
            .provider_failures
            .entry(provider_id.to_string())
            .or_default();
        tracker.failures.push_back(now);
        while let Some(oldest) = tracker.failures.front().copied() {
            if now.duration_since(oldest) <= FAILURE_WINDOW {
                break;
            }
            tracker.failures.pop_front();
        }
        if tracker.failures.len() >= FAILURE_LIMIT {
            tracker.blocked_until = Some(now + FAILURE_COOLDOWN);
            tracker.failures.clear();
        }
    }

    fn cleanup_expired(&mut self) {
        let now = std::time::Instant::now();
        self.idempotency_cache
            .retain(|_, cached| now.duration_since(cached.stored_at) <= IDEMPOTENCY_RETENTION);
        for tracker in self.provider_failures.values_mut() {
            while let Some(oldest) = tracker.failures.front().copied() {
                if now.duration_since(oldest) <= FAILURE_WINDOW {
                    break;
                }
                tracker.failures.pop_front();
            }
            if tracker
                .blocked_until
                .is_some_and(|blocked_until| blocked_until <= now)
            {
                tracker.blocked_until = None;
            }
        }
    }
}

fn decrement_counter<K: Eq + Hash + Clone>(map: &mut HashMap<K, usize>, key: &K) {
    if let Some(counter) = map.get_mut(key) {
        *counter = counter.saturating_sub(1);
        if *counter == 0 {
            map.remove(key);
        }
    }
}

fn map_lookup_error(error: SurfaceRegistryLookupError) -> SurfaceProxyError {
    match error {
        SurfaceRegistryLookupError::SurfaceNotFound => SurfaceProxyError::NoProvider,
        SurfaceRegistryLookupError::InteractionNotFound => SurfaceProxyError::InteractionNotFound,
        SurfaceRegistryLookupError::TargetProviderRequired => {
            SurfaceProxyError::TargetProviderRequired
        }
        SurfaceRegistryLookupError::InvalidProvider(provider_id) => {
            SurfaceProxyError::InvalidProvider(provider_id)
        }
        SurfaceRegistryLookupError::NoTenantCompatibleProvider => SurfaceProxyError::NoProvider,
    }
}

fn caller_origin_for_request(
    registry: &SurfaceRegistry,
    caller: &SurfaceCallerOrigin,
    _resolved: &crate::surface_registry::ResolvedSurfaceAction,
    _request: &SurfaceInvokeRequest,
) -> Result<surfaces::CallerOrigin, SurfaceProxyError> {
    match caller {
        SurfaceCallerOrigin::UserSession {
            user_id,
            session_id,
        } => Ok(surfaces::CallerOrigin::UserSession {
            user_id: user_id.to_string(),
            session_id: session_id.clone(),
        }),
        SurfaceCallerOrigin::BuiltInSystem { principal } => {
            Ok(surfaces::CallerOrigin::BuiltInSystem {
                principal: principal.clone(),
            })
        }
        SurfaceCallerOrigin::Provider { service_id } => {
            let provider_id = registry
                .provider_id_for_service(service_id)
                .ok_or_else(|| {
                    SurfaceProxyError::InvalidProvider(format!(
                        "service {service_id} has no registered surface provider"
                    ))
                })?;
            Ok(surfaces::CallerOrigin::Provider { provider_id })
        }
    }
}

async fn implicit_target_provider_for_request(
    service_connections: &ServiceConnectionRegistry,
    registry: &SurfaceRegistry,
    request: &SurfaceInvokeRequest,
) -> Result<Option<String>, SurfaceProxyError> {
    if request.target_provider_id.is_some() {
        return Ok(request.target_provider_id.clone());
    }

    match &request.caller_origin {
        SurfaceCallerOrigin::Provider { service_id } => registry
            .provider_id_for_service(service_id)
            .map(Some)
            .ok_or_else(|| {
                SurfaceProxyError::InvalidProvider(format!(
                    "service {service_id} has no registered surface provider"
                ))
            }),
        SurfaceCallerOrigin::UserSession { .. } | SurfaceCallerOrigin::BuiltInSystem { .. } => {
            let mut available_candidates = Vec::new();
            for provider in registry
                .list_targeted_providers_for_surface(request.surface_id.as_str(), request.tenant_id)
            {
                if !provider.tenant_compatible {
                    continue;
                }
                if provider_is_available(service_connections, &provider).await {
                    available_candidates.push(provider);
                }
            }

            if available_candidates.is_empty() {
                return Ok(None);
            }

            if available_candidates
                .iter()
                .any(|provider| provider.targeting == surfaces::Targeting::Targeted)
            {
                return Err(SurfaceProxyError::TargetProviderRequired);
            }

            Ok(available_candidates
                .first()
                .map(|provider| provider.provider_id.clone()))
        }
    }
}

async fn provider_is_available(
    service_connections: &ServiceConnectionRegistry,
    provider: &crate::surface_registry::SurfaceProviderSummary,
) -> bool {
    match provider.service_id {
        Some(service_id) => {
            service_connections.is_connected(&service_id).await
                && !service_connections.is_yielded(&service_id)
        }
        None => true,
    }
}

fn validate_input_schema(
    interaction: &surfaces::InteractionDescriptor,
    params: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), SurfaceProxyError> {
    if let Some(schema) = &interaction.input_schema {
        let value = serde_json::Value::Object(params.clone());
        if !schema_matches(schema, &value) {
            return Err(SurfaceProxyError::SchemaValidationFailed(
                "input schema validation failed".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_sensitive_fields(
    interaction: &surfaces::InteractionDescriptor,
    params: &serde_json::Map<String, serde_json::Value>,
    encrypted_sensitive_params: Option<&surfaces::EncryptedSensitiveParams>,
) -> Result<(), SurfaceProxyError> {
    if interaction.sensitive_fields.is_empty() {
        return Ok(());
    }

    if matches!(
        interaction.transport,
        surfaces::InteractionTransport::ControllerLocal
    ) {
        return Ok(());
    }

    for key in &interaction.sensitive_fields {
        if params.contains_key(key) {
            return Err(SurfaceProxyError::SensitiveFieldRejected(format!(
                "sensitive field `{key}` must not be sent in cleartext params"
            )));
        }
    }

    if encrypted_sensitive_params.is_none() {
        return Ok(());
    }

    Ok(())
}

fn resolve_timeout(
    timeout_override: Option<Duration>,
    interaction: &surfaces::InteractionDescriptor,
) -> Result<Duration, SurfaceProxyError> {
    let timeout = timeout_override.unwrap_or_else(|| {
        Duration::from_secs(u64::from(
            interaction
                .timeout_seconds
                .unwrap_or(DEFAULT_TIMEOUT_SECONDS),
        ))
    });
    let secs = timeout.as_secs();
    if !(u64::from(MIN_TIMEOUT_SECONDS)..=u64::from(MAX_TIMEOUT_SECONDS)).contains(&secs) {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "timeout must be between {MIN_TIMEOUT_SECONDS}s and {MAX_TIMEOUT_SECONDS}s"
        )));
    }
    Ok(timeout)
}

fn validate_result_schema(
    interaction: &surfaces::InteractionDescriptor,
    result: Option<&serde_json::Value>,
) -> Result<(), SurfaceProxyError> {
    if let Some(schema) = &interaction.result_schema {
        let result = result.unwrap_or(&serde_json::Value::Null);
        if !schema_matches(schema, result) {
            return Err(SurfaceProxyError::SchemaValidationFailed(
                "result schema validation failed".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_result_limits(result: &serde_json::Value) -> Result<(), SurfaceProxyError> {
    let bytes = serde_json::to_vec(result)
        .map_err(|error| SurfaceProxyError::SchemaValidationFailed(error.to_string()))?
        .len();
    if bytes > MAX_RESULT_BYTES {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result payload is {bytes} bytes, max {MAX_RESULT_BYTES}"
        )));
    }

    if let Some(array) = result.as_array()
        && array.len() > MAX_RESULT_ROWS
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result array has {} rows, max {MAX_RESULT_ROWS}",
            array.len()
        )));
    }

    if let Some(rows) = result.get("rows").and_then(|rows| rows.as_array())
        && rows.len() > MAX_RESULT_ROWS
    {
        return Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "result rows has {} rows, max {MAX_RESULT_ROWS}",
            rows.len()
        )));
    }

    Ok(())
}

fn build_idempotency_key(
    request: &SurfaceInvokeRequest,
    caller_origin: &surfaces::CallerOrigin,
) -> IdempotencyKey {
    IdempotencyKey {
        tenant_id: request.tenant_id,
        surface_id: request.surface_id.clone(),
        interaction_id: request.interaction_id.clone(),
        caller_key: match caller_origin {
            surfaces::CallerOrigin::UserSession {
                user_id,
                session_id,
            } => format!("user:{user_id}:{session_id}"),
            surfaces::CallerOrigin::BuiltInSystem { principal } => {
                format!("system:{principal}")
            }
            surfaces::CallerOrigin::Provider { provider_id } => {
                format!("provider:{provider_id}")
            }
        },
        idempotency_key: request.idempotency_key.clone(),
    }
}

fn fingerprint_request(
    params: &serde_json::Map<String, serde_json::Value>,
    encrypted: Option<&surfaces::EncryptedSensitiveParams>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::Value::Object(params.clone())
        .to_string()
        .hash(&mut hasher);
    encrypted
        .map(|value| (&value.key_id, &value.ciphertext_b64))
        .hash(&mut hasher);
    hasher.finish()
}

fn schema_matches(schema: &surfaces::SchemaContract, value: &serde_json::Value) -> bool {
    match schema {
        surfaces::SchemaContract::Any => true,
        surfaces::SchemaContract::Object => value.is_object(),
        surfaces::SchemaContract::Array => value.is_array(),
        surfaces::SchemaContract::String => value.is_string(),
        surfaces::SchemaContract::Integer => value.as_i64().is_some(),
        surfaces::SchemaContract::Number => value.as_f64().is_some(),
        surfaces::SchemaContract::Boolean => value.is_boolean(),
        surfaces::SchemaContract::Null => value.is_null(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};
    use std::sync::Arc as StdArc;
    use std::sync::Once;

    use async_trait::async_trait;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, ConnectOptions, ConnectionTrait, Database, EntityTrait,
        QueryFilter, QueryOrder, Set,
    };
    use uptrakit_shared_db::entity::audit_log;
    use uptrakit_wire::ControllerMessage;

    use super::*;
    use crate::surface_registry::{SurfaceRegistry, SurfaceRegistryConfig};

    fn tenant_id() -> Uuid {
        Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap()
    }

    fn user_id() -> Uuid {
        Uuid::parse_str("aaaaaaaa-1111-1111-1111-111111111111").unwrap()
    }

    fn registration(provider_id: &str, service_tenant: Uuid) -> surfaces::SurfaceRegistration {
        surfaces::SurfaceRegistration {
            provider: surfaces::ProviderIdentity {
                provider_id: provider_id.to_string(),
                provider_kind: surfaces::ProviderKind::Service,
                provider_namespace: "service".to_string(),
            },
            framework_generation: surfaces::FrameworkGeneration::new(1, 0),
            capabilities: surfaces::CapabilitySet::from_capabilities([
                surfaces::Capability::TextBlockNode,
                surfaces::Capability::TargetedTargeting,
                surfaces::Capability::MutationAction,
                surfaces::Capability::SensitiveFields,
                surfaces::Capability::ProviderInitiatedActions,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(service_tenant.to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("ssh.guest.panel").unwrap())
                    .label("SSH")
                    .priority(100)
                    .slot("software.tabs")
                    .scope(surfaces::Scope::Tenant)
                    .targeting(surfaces::Targeting::Targeted)
                    .required_permission("view_software")
                    .provider_kind(surfaces::ProviderKind::Service)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::TargetedTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("refresh").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: "Refresh".to_string(),
                    required_permission: Some("update_software".to_string()),
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
                    sensitive_fields: vec!["token".to_string()],
                    timeout_seconds: Some(2),
                    confirmation: None,
                    transport: surfaces::InteractionTransport::ProviderProxied,
                    workflow_steps: vec![],
                    form_ui: None,
                }],
                data_sources: vec![],
            }],
            encryption_metadata: Some(surfaces::ProviderEncryptionMetadata {
                key_id: "key-1".to_string(),
                algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
                public_key: "pubkey".to_string(),
            }),
        }
    }

    fn plugin_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
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
                scope: surfaces::Scope::Tenant,
                tenant_id: Some(tenant_id().to_string()),
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(
                        surfaces::SurfaceId::new("notifications.email.global_smtp").unwrap(),
                    )
                    .label("SMTP Defaults")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_BELOW_GLOBAL)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("save_global_smtp").unwrap(),
                    kind: surfaces::InteractionKind::MutationAction,
                    label: "Save Global SMTP".to_string(),
                    required_permission: None,
                    input_schema: Some(surfaces::SchemaContract::Object),
                    result_schema: Some(surfaces::SchemaContract::Object),
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

    fn plugin_registration_with_local_sensitive(
        provider_id: &str,
    ) -> surfaces::SurfaceRegistration {
        let mut registration = plugin_registration(provider_id);
        registration
            .capabilities
            .0
            .insert(surfaces::Capability::SensitiveFields);
        registration.surfaces[0].interactions[0].sensitive_fields =
            vec!["smtp_password".to_string()];
        registration
    }

    fn notification_channel_registration(
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
                surfaces::Capability::FormSubmit,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new(surface_id).unwrap())
                    .label("Notification Channels")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_TABS)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new(interaction_id).unwrap(),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: "Add Notification Channel".to_string(),
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

    fn proxmox_hosts_registration(provider_id: &str) -> surfaces::SurfaceRegistration {
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
                surfaces::Capability::FormSubmit,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("proxmox.hosts").unwrap())
                    .label("Proxmox Hosts")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_TABS)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("add-config").unwrap(),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: "Add Configuration".to_string(),
                    required_permission: Some("manage_commands".to_string()),
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
                surfaces::Capability::FormSubmit,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new(surface_id).unwrap())
                    .label("Proxmox Update Protection")
                    .priority(100)
                    .slot(surfaces::SLOT_SETTINGS_TABS)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::MutationAction,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "ok".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new(interaction_id).unwrap(),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: "Save Update Protection".to_string(),
                    required_permission: Some("update_software".to_string()),
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
                surfaces::Capability::FormSubmit,
                surfaces::Capability::FormNode,
            ]),
            effective_tenant_binding: surfaces::EffectiveTenantBinding {
                scope: surfaces::Scope::Global,
                tenant_id: None,
            },
            surfaces: vec![surfaces::RegisteredSurface {
                descriptor: surfaces::SurfaceDescriptor::builder()
                    .surface_id(surfaces::SurfaceId::new("docker.item-host-actions").unwrap())
                    .label("Docker")
                    .priority(100)
                    .slot(surfaces::SLOT_SOFTWARE_ITEM_HOST_CONTEXT_MENU)
                    .scope(surfaces::Scope::Global)
                    .targeting(surfaces::Targeting::Universal)
                    .required_permission("update_software")
                    .provider_kind(surfaces::ProviderKind::Plugin)
                    .required_capabilities(surfaces::CapabilitySet::from_capabilities([
                        surfaces::Capability::TextBlockNode,
                        surfaces::Capability::FormSubmit,
                        surfaces::Capability::FormNode,
                        surfaces::Capability::UniversalTargeting,
                    ]))
                    .root_node(surfaces::SurfaceNode::TextBlock {
                        text: "Docker".to_string(),
                    })
                    .build(),
                interactions: vec![surfaces::InteractionDescriptor {
                    interaction_id: surfaces::InteractionId::new("switch-tag").unwrap(),
                    kind: surfaces::InteractionKind::FormSubmit,
                    label: "Switch Tag".to_string(),
                    required_permission: Some("update_software".to_string()),
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

    fn ensure_master_key() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| {
            uptrakit_crypto::enable_plaintext_mode();
            let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([7_u8; 32]));
        });
    }

    async fn setup_notification_db() -> sea_orm::DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations should run");
        insert_tenant(&db, tenant_id()).await;
        db
    }

    async fn insert_tenant(db: &sea_orm::DatabaseConnection, id: Uuid) {
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::tenant::ActiveModel {
            id: Set(id),
            name: Set("Surface Test Tenant".to_string()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
    }

    async fn ensure_proxmox_update_protection_tables(db: &sea_orm::DatabaseConnection) {
        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS proxmox_protection_defaults (
                tenant_id TEXT NOT NULL,
                plugin_config_id TEXT NOT NULL,
                mode TEXT NOT NULL,
                backup_target_key TEXT NULL,
                snapshot_timeout_seconds INTEGER NULL,
                backup_timeout_seconds INTEGER NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (tenant_id, plugin_config_id)
            )",
        )
        .await
        .expect("create proxmox_protection_defaults table");

        db.execute_unprepared(
            "CREATE TABLE IF NOT EXISTS proxmox_protection_item_overrides (
                software_item_id TEXT NOT NULL,
                plugin_config_id TEXT NOT NULL,
                mode TEXT NOT NULL,
                backup_target_key TEXT NULL,
                snapshot_timeout_seconds INTEGER NULL,
                backup_timeout_seconds INTEGER NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (software_item_id, plugin_config_id)
            )",
        )
        .await
        .expect("create proxmox_protection_item_overrides table");
    }

    async fn insert_active_proxmox_plugin_config(db: &sea_orm::DatabaseConnection) -> Uuid {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::plugin_config::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id()),
            name: Set(format!("PVE {}", id)),
            plugin_type: Set("infrastructure_proxmox".to_string()),
            config: Set(serde_json::json!({
                "api_url": "https://pve.local:8006",
                "api_token": "root@pam!uptrakit=secret-token",
                "verify_tls": true
            })),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert proxmox plugin config");

        id
    }

    async fn insert_software_item(db: &sea_orm::DatabaseConnection) -> Uuid {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        uptrakit_shared_db::entity::software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id()),
            name: Set(format!("Software {}", id)),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software item");
        id
    }

    async fn assign_proxmox_plugin_to_software_item(
        db: &sea_orm::DatabaseConnection,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
    ) {
        let now = time::OffsetDateTime::now_utc();
        let host_id = Uuid::now_v7();
        let host_software_item_id = Uuid::now_v7();

        uptrakit_shared_db::entity::host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id()),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set(format!("Host {host_id}")),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host");

        uptrakit_shared_db::entity::host_software_item::ActiveModel {
            id: Set(host_software_item_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(plugin_config_id)),
            package_identifier: Set(Some("pkg".to_string())),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host_software_item");

        uptrakit_shared_db::entity::host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(Some(plugin_config_id)),
            plugin_type: Set("infrastructure_proxmox".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("pkg".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert host_software_item_plugin");
    }

    fn test_audit_emitter(db: sea_orm::DatabaseConnection) -> uptrakit_audit_log::AuditEmitter {
        let backend = StdArc::new(uptrakit_audit_log::DatabaseBackend::new(db));
        let dispatcher = uptrakit_audit_log::AuditLogDispatcher::new(backend);
        uptrakit_audit_log::AuditEmitter::new(dispatcher)
    }

    async fn latest_tenant_audit_row_for_action(
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

    async fn latest_tenant_audit_row_for_action_and_outcome(
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

    fn registry() -> SurfaceRegistry {
        SurfaceRegistry::new(SurfaceRegistryConfig {
            allowed_controller_queries: HashSet::new(),
            allowed_sse_topics: HashSet::new(),
            allowed_direct_builtin_operations: HashSet::new(),
            ..SurfaceRegistryConfig::default()
        })
    }

    fn request_with_idem(idempotency_key: &str) -> SurfaceInvokeRequest {
        SurfaceInvokeRequest {
            tenant_id: tenant_id(),
            surface_id: "ssh.guest.panel".to_string(),
            interaction_id: "refresh".to_string(),
            idempotency_key: idempotency_key.to_string(),
            target_provider_id: Some("provider-a".to_string()),
            caller_origin: SurfaceCallerOrigin::UserSession {
                user_id: user_id(),
                session_id: "session-1".to_string(),
            },
            params: serde_json::Map::new(),
            encrypted_sensitive_params: Some(surfaces::EncryptedSensitiveParams {
                key_id: "key-1".to_string(),
                algorithm: surfaces::ProviderEncryptionAlgorithm::EciesP256,
                ciphertext_b64: "AAAA".to_string(),
            }),
        }
    }

    type SeenInvocation = (String, String, Option<Uuid>, Option<Uuid>);

    struct TestPluginInvoker {
        response: serde_json::Value,
        seen: StdArc<Mutex<Vec<SeenInvocation>>>,
    }

    #[async_trait]
    impl PluginSurfaceActionInvoker for TestPluginInvoker {
        async fn invoke(
            &self,
            _db: &(dyn std::any::Any + Send + Sync),
            tenant_id: Option<Uuid>,
            caller_user_id: Option<Uuid>,
            surface_id: &str,
            interaction_id: &str,
            _params: serde_json::Value,
        ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
            self.seen.lock().push((
                surface_id.to_string(),
                interaction_id.to_string(),
                tenant_id,
                caller_user_id,
            ));
            Ok(self.response.clone())
        }
    }

    struct ErrorPluginInvoker {
        error_message: String,
    }

    #[async_trait]
    impl PluginSurfaceActionInvoker for ErrorPluginInvoker {
        async fn invoke(
            &self,
            _db: &(dyn std::any::Any + Send + Sync),
            _tenant_id: Option<Uuid>,
            _caller_user_id: Option<Uuid>,
            _surface_id: &str,
            _interaction_id: &str,
            _params: serde_json::Value,
        ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
            Err(SurfaceActionError::InvalidInput(self.error_message.clone()))
        }
    }

    struct BlockingPluginInvoker {
        started: StdArc<tokio::sync::Notify>,
        release: StdArc<tokio::sync::Notify>,
        calls: StdArc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl PluginSurfaceActionInvoker for BlockingPluginInvoker {
        async fn invoke(
            &self,
            _db: &(dyn std::any::Any + Send + Sync),
            _tenant_id: Option<Uuid>,
            _caller_user_id: Option<Uuid>,
            _surface_id: &str,
            _interaction_id: &str,
            _params: serde_json::Value,
        ) -> std::result::Result<serde_json::Value, SurfaceActionError> {
            self.calls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.started.notify_waiters();
            self.release.notified().await;
            Ok(serde_json::json!({"ok": true}))
        }
    }

    async fn register_service_for_proxy(
        registry: &SurfaceRegistry,
        service_connections: &ServiceConnectionRegistry,
    ) -> (Uuid, tokio::sync::mpsc::Receiver<ControllerMessage>) {
        let service_id = Uuid::now_v7();
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id()),
                registration("provider-a", tenant_id()),
            )
            .expect("registration should succeed");

        let (rx, _cancel) = service_connections
            .register(
                service_id,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        (service_id, rx)
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_correlates_request_and_response() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());

        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;
        let proxy_clone = Arc::clone(&proxy);

        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy_clone.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"ok": true})),
                        error: None,
                    },
                );
            }
        });

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-1"),
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("invoke should succeed");

        assert!(response.success);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_executes_plugin_controller_local_interaction() {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
            .expect("plugin registration should succeed");

        let seen = StdArc::new(Mutex::new(Vec::new()));
        let invoker = TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::clone(&seen),
        };
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(StdArc::new(()), Arc::new(invoker)),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-plugin-local".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("plugin-backed local interaction should succeed");

        assert!(response.success);
        assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
        let seen = seen.lock();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "notifications.email.global_smtp");
        assert_eq!(seen[0].1, "save_global_smtp");
        assert_eq!(seen[0].2, Some(tenant_id()));
        assert_eq!(seen[0].3, Some(user_id()));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_rejects_duplicate_idempotency_deterministically() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let proxy_clone = Arc::clone(&proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx.recv().await {
                proxy_clone.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"cached": true})),
                        error: None,
                    },
                );
            }
        });

        let first = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-dup"),
                Some(Duration::from_secs(5)),
            )
            .await;
        assert!(first.is_ok(), "first invocation should succeed");

        let second = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-dup"),
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("second invocation should be deterministic");
        assert!(
            second.success,
            "duplicate should return deterministic previous result"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_controller_local_rejects_concurrent_duplicate_idempotency() {
        let registry = Arc::new(SurfaceRegistry::new(SurfaceRegistryConfig::default()));
        registry
            .bootstrap_plugin(plugin_registration("plugin.notifications_email"))
            .expect("plugin registration should succeed");

        let started = StdArc::new(tokio::sync::Notify::new());
        let release = StdArc::new(tokio::sync::Notify::new());
        let calls = StdArc::new(std::sync::atomic::AtomicUsize::new(0));
        let proxy = Arc::new(SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                StdArc::new(()),
                Arc::new(BlockingPluginInvoker {
                    started: StdArc::clone(&started),
                    release: StdArc::clone(&release),
                    calls: StdArc::clone(&calls),
                }),
            ),
        )));
        let service_connections = Arc::new(ServiceConnectionRegistry::new());

        let proxy_first = Arc::clone(&proxy);
        let registry_first = Arc::clone(&registry);
        let service_connections_first = Arc::clone(&service_connections);
        let first_invoke = tokio::spawn(async move {
            proxy_first
                .invoke(
                    &service_connections_first,
                    &registry_first,
                    SurfaceInvokeRequest {
                        tenant_id: tenant_id(),
                        surface_id: "notifications.email.global_smtp".to_string(),
                        interaction_id: "save_global_smtp".to_string(),
                        idempotency_key: "idem-local-dup".to_string(),
                        target_provider_id: None,
                        caller_origin: SurfaceCallerOrigin::UserSession {
                            user_id: user_id(),
                            session_id: "session-1".to_string(),
                        },
                        params: serde_json::Map::new(),
                        encrypted_sensitive_params: None,
                    },
                    Some(Duration::from_secs(5)),
                )
                .await
        });

        started.notified().await;

        let second = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-local-dup".to_string(),
                    target_provider_id: None,
                    caller_origin: SurfaceCallerOrigin::UserSession {
                        user_id: user_id(),
                        session_id: "session-1".to_string(),
                    },
                    params: serde_json::Map::new(),
                    encrypted_sensitive_params: None,
                },
                Some(Duration::from_secs(5)),
            )
            .await;

        assert!(
            matches!(second, Err(SurfaceProxyError::DuplicateRequest)),
            "concurrent duplicate local invocation must be rejected"
        );

        release.notify_waiters();
        let first = first_invoke
            .await
            .expect("first invoke task should complete")
            .expect("first local invocation should succeed");
        assert!(first.success);
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_rejects_cleartext_sensitive_fields() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let (_service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

        let mut request = request_with_idem("idem-cleartext");
        request
            .params
            .insert("token".to_string(), serde_json::json!("clear"));

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect_err("cleartext sensitive field should be rejected");
        assert!(matches!(err, SurfaceProxyError::SensitiveFieldRejected(_)));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_allows_provider_proxied_requests_without_sensitive_payload() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let invoke_task = tokio::spawn({
            let request = SurfaceInvokeRequest {
                tenant_id: tenant_id(),
                surface_id: "ssh.guest.panel".to_string(),
                interaction_id: "refresh".to_string(),
                idempotency_key: "idem-no-sensitive-payload".to_string(),
                target_provider_id: Some("provider-a".to_string()),
                caller_origin: SurfaceCallerOrigin::UserSession {
                    user_id: user_id(),
                    session_id: "session-1".to_string(),
                },
                params: serde_json::Map::from_iter([(
                    "note".to_string(),
                    serde_json::json!("no-secret-change"),
                )]),
                encrypted_sensitive_params: None,
            };
            let proxy = Arc::clone(&proxy);
            async move {
                proxy
                    .invoke(
                        &service_connections,
                        &registry,
                        request,
                        Some(Duration::from_secs(5)),
                    )
                    .await
            }
        });

        let Some(ControllerMessage::SurfaceActionRequest(forwarded_request)) = rx.recv().await
        else {
            panic!("expected forwarded ControllerMessage::SurfaceActionRequest");
        };
        assert!(forwarded_request.encrypted_sensitive_params.is_none());

        let response = surfaces::SurfaceActionResponse {
            request_id: forwarded_request.request_id,
            success: true,
            result: Some(serde_json::json!({"ok": true})),
            error: None,
        };
        proxy.complete(forwarded_request.request_id, response.clone());

        let result = invoke_task
            .await
            .expect("invoke task should complete")
            .expect("provider-proxied request without sensitive payload should succeed");
        assert_eq!(result, response);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_controller_local_allows_cleartext_sensitive_fields() {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(plugin_registration_with_local_sensitive(
                "plugin.notifications_email",
            ))
            .expect("plugin registration should succeed");

        let invoker = TestPluginInvoker {
            response: serde_json::json!({"ok": true}),
            seen: StdArc::new(Mutex::new(Vec::new())),
        };
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(StdArc::new(()), Arc::new(invoker)),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let mut params = serde_json::Map::new();
        params.insert("smtp_password".to_string(), serde_json::json!("clear"));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email.global_smtp".to_string(),
                    interaction_id: "save_global_smtp".to_string(),
                    idempotency_key: "idem-plugin-local-sensitive".to_string(),
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
            .expect("controller-local sensitive fields should be accepted in cleartext");

        assert!(response.success);
        assert_eq!(response.result, Some(serde_json::json!({"ok": true})));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_times_out_and_emits_cancellation() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let invoke_task = tokio::spawn({
            let request = request_with_idem("idem-timeout");
            let proxy = proxy;
            async move {
                proxy
                    .invoke(
                        &service_connections,
                        &registry,
                        request,
                        Some(Duration::from_secs(2)),
                    )
                    .await
            }
        });

        let first = rx
            .recv()
            .await
            .expect("first message should be action request");
        assert!(matches!(first, ControllerMessage::SurfaceActionRequest(_)));
        let second = rx.recv().await.expect("second message should be cancel");
        assert!(matches!(second, ControllerMessage::SurfaceActionCancel(_)));

        let result = invoke_task.await.expect("invoke task should finish");
        assert!(matches!(result, Err(SurfaceProxyError::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_ignores_late_response_after_timeout() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let proxy_clone = Arc::clone(&proxy);
        tokio::spawn(async move {
            let request = match rx.recv().await {
                Some(ControllerMessage::SurfaceActionRequest(request)) => request,
                other => panic!("expected surface action request, got {other:?}"),
            };
            tokio::time::advance(Duration::from_secs(3)).await;
            proxy_clone.complete(
                request.request_id,
                surfaces::SurfaceActionResponse {
                    request_id: request.request_id,
                    success: true,
                    result: Some(serde_json::json!({"late": true})),
                    error: None,
                },
            );
        });

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-late"),
                Some(Duration::from_secs(2)),
            )
            .await;
        assert!(matches!(result, Err(SurfaceProxyError::Timeout)));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_validates_input_schema_before_dispatch() {
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let service_id = Uuid::now_v7();
        let mut custom_registration = registration("provider-a", tenant_id());
        custom_registration.surfaces[0].interactions[0].input_schema =
            Some(surfaces::SchemaContract::Integer);
        registry
            .register_service(
                service_id,
                "uptrakit-agent-ssh",
                Some(tenant_id()),
                custom_registration,
            )
            .expect("registration should succeed");
        let (_rx, _cancel) = service_connections
            .register(
                service_id,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let mut request = request_with_idem("idem-schema");
        request
            .params
            .insert("value".to_string(), serde_json::json!("not-integer"));

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await;
        assert!(
            matches!(result, Err(SurfaceProxyError::SchemaValidationFailed(_))),
            "expected schema validation failure, got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_targeted_surface_requires_explicit_target_provider() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new();
        let (_service_id, _rx) = register_service_for_proxy(&registry, &service_connections).await;

        let mut request = request_with_idem("idem-missing-target");
        request.target_provider_id = None;

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect_err("targeted invocation must require explicit target provider");
        assert!(matches!(err, SurfaceProxyError::TargetProviderRequired));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_provider_origin_can_route_to_another_provider() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());

        let service_a = Uuid::now_v7();
        let mut reg_a = registration("provider-a", tenant_id());
        reg_a.surfaces[0].interactions[0].required_permission = None;
        registry
            .register_service(service_a, "uptrakit-agent-ssh", Some(tenant_id()), reg_a)
            .expect("provider-a registration should succeed");

        let service_b = Uuid::now_v7();
        let mut reg_b = registration("provider-b", tenant_id());
        reg_b.surfaces[0].interactions[0].required_permission = None;
        registry
            .register_service(service_b, "uptrakit-agent-ssh", Some(tenant_id()), reg_b)
            .expect("provider-b registration should succeed");

        let (_rx_a, _cancel_a) = service_connections
            .register(
                service_a,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        let (mut rx_b, _cancel_b) = service_connections
            .register(
                service_b,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;

        let proxy_clone = Arc::clone(&proxy);
        tokio::spawn(async move {
            if let Some(ControllerMessage::SurfaceActionRequest(request)) = rx_b.recv().await {
                proxy_clone.complete(
                    request.request_id,
                    surfaces::SurfaceActionResponse {
                        request_id: request.request_id,
                        success: true,
                        result: Some(serde_json::json!({"routed": true})),
                        error: None,
                    },
                );
            }
        });

        let mut request = request_with_idem("idem-cross-provider");
        request.target_provider_id = Some("provider-b".to_string());
        request.caller_origin = SurfaceCallerOrigin::Provider {
            service_id: service_a,
        };

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                request,
                Some(Duration::from_secs(5)),
            )
            .await
            .expect("controller-authorized cross-provider invoke should succeed");
        assert!(response.success);
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_no_provider_for_yielded_service_provider() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());

        let service_a = Uuid::now_v7();
        registry
            .register_service(
                service_a,
                "uptrakit-agent-ssh",
                Some(tenant_id()),
                registration("provider-a", tenant_id()),
            )
            .expect("provider-a registration should succeed");

        let (mut rx_a, _cancel_a) = service_connections
            .register(
                service_a,
                BTreeSet::new(),
                None,
                None,
                Some("uptrakit-agent-ssh".to_string()),
            )
            .await;
        assert!(service_connections.set_yielded(&service_a, true));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                request_with_idem("idem-yielded-unavailable"),
                Some(Duration::from_secs(5)),
            )
            .await
            .expect_err("yielded provider should fail fast");

        assert!(matches!(response, SurfaceProxyError::NoProvider));
        assert!(
            rx_a.try_recv().is_err(),
            "yielded provider must not receive the proxied surface request"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_fails_immediately_when_provider_disconnects() {
        let registry = registry();
        let service_connections = ServiceConnectionRegistry::new();
        let proxy = Arc::new(SurfaceProxy::new());
        let (_service_id, mut rx) =
            register_service_for_proxy(&registry, &service_connections).await;

        let proxy_invoke = Arc::clone(&proxy);
        let invoke_task = tokio::spawn(async move {
            proxy_invoke
                .invoke(
                    &service_connections,
                    &registry,
                    request_with_idem("idem-disconnect"),
                    Some(Duration::from_secs(60)),
                )
                .await
        });

        let outbound = rx
            .recv()
            .await
            .expect("first message should be action request");
        assert!(matches!(
            outbound,
            ControllerMessage::SurfaceActionRequest(_)
        ));

        proxy.fail_in_flight_for_provider("provider-a");

        let result = invoke_task.await.expect("invoke task should finish");
        assert!(matches!(
            result,
            Err(SurfaceProxyError::ServiceDisconnected)
        ));
    }

    #[tokio::test]
    async fn invoke_allowlisted_notification_create_executes_controller_owned_path() {
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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("Ops Hook"));
        params.insert("channel_type".to_string(), serde_json::json!("webhook"));
        params.insert(
            "config".to_string(),
            serde_json::json!({
                "url": "https://example.invalid/hook"
            }),
        );
        params.insert("enabled".to_string(), serde_json::json!(true));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.webhook".to_string(),
                    interaction_id: "create".to_string(),
                    idempotency_key: "idem-notification-create".to_string(),
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
            .expect("allowlisted notification create should execute locally");

        assert!(response.success);
        let result = response
            .result
            .expect("notification create should return a payload");
        assert_eq!(result["channel_type"], "webhook");
        assert_eq!(result["name"], "Ops Hook");

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id()));
        assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
        let details = row.details_json.expect("audit details");
        assert_eq!(details["channel_type"], serde_json::json!("webhook"));
        assert_eq!(
            details["create_source"],
            serde_json::json!("surface_proxy.notification_channel.create")
        );
    }

    #[cfg(feature = "notifications-email")]
    #[tokio::test]
    async fn invoke_notifications_email_configure_smtp_executes_controller_local_path() {
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
                "plugin.email",
                "notifications.email",
                "configure_smtp",
            ))
            .expect("plugin registration should succeed");

        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("host".to_string(), serde_json::json!("smtp.tenant.example"));
        params.insert("port".to_string(), serde_json::json!(2525));
        params.insert(
            "from_address".to_string(),
            serde_json::json!("alerts@example.com"),
        );
        params.insert("tls_mode".to_string(), serde_json::json!("starttls"));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "notifications.email".to_string(),
                    interaction_id: "configure_smtp".to_string(),
                    idempotency_key: "idem-email-configure-smtp".to_string(),
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
            .expect("configure_smtp should execute locally");

        assert!(response.success);
        let result = response
            .result
            .expect("configure_smtp should return a payload");
        assert_eq!(result["host"], "smtp.tenant.example");
        assert_eq!(result["port"], 2525);
        assert_eq!(result["from_address"], "alerts@example.com");

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id()));
        assert_eq!(row.target_type.as_deref(), Some("tenant_setting"));
        assert_eq!(row.target_id.as_deref(), Some("smtp"));
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.notification_settings.configure_smtp")
        );
        assert_eq!(details["setting_scope"], serde_json::json!("tenant"));
        assert_eq!(details["setting_area"], serde_json::json!("smtp"));
        assert_eq!(
            details["requested_keys"],
            serde_json::json!(["from_address", "host", "port", "tls_mode"])
        );
    }

    #[cfg(feature = "notifications-email")]
    #[tokio::test]
    async fn invoke_notifications_email_save_global_smtp_emits_global_setting_update_audit() {
        ensure_master_key();
        let db = setup_notification_db().await;
        let seen = StdArc::new(Mutex::new(Vec::new()));
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(TestPluginInvoker {
                    response: serde_json::json!({"ok": true}),
                    seen: StdArc::clone(&seen),
                }),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(plugin_registration("plugin.email"))
            .expect("plugin registration should succeed");

        let mut params = serde_json::Map::new();
        params.insert("host".to_string(), serde_json::json!("smtp.global.example"));
        params.insert(
            "smtp_password".to_string(),
            serde_json::json!("secret-value"),
        );

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

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::GLOBAL_SETTING_UPDATE,
        )
        .await;
        assert_eq!(row.tenant_id, tenant_id());
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
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
        assert_eq!(
            details["requested_keys"],
            serde_json::json!(["host", "smtp_password"])
        );
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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(ErrorPluginInvoker {
                    error_message: "Internal server error".to_string(),
                }),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(notification_channel_registration(
                "plugin.telegram",
                "notifications.telegram.global_settings",
                "save_global_telegram",
            ))
            .expect("plugin registration should succeed");

        let mut params = serde_json::Map::new();
        params.insert(
            "bot_token".to_string(),
            serde_json::json!("123456:super-secret"),
        );

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

        let row = latest_tenant_audit_row_for_action_and_outcome(
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
        assert_eq!(details["requested_keys"], serde_json::json!(["bot_token"]));
        assert!(
            !details.to_string().contains("123456:super-secret"),
            "audit details must never include raw secret values"
        );
    }

    #[test]
    fn build_notification_channel_create_request_rejects_non_boolean_enabled() {
        let params = serde_json::json!({
            "name": "Ops Hook",
            "config": {"url": "https://example.invalid/hook"},
            "enabled": { "bad": true }
        });
        let params = params.as_object().expect("params should be an object");

        let result = build_notification_channel_create_request("webhook", params);
        let err = result.expect_err("non-boolean enabled must be rejected");
        assert!(
            err.contains("enabled"),
            "expected enabled validation error, got: {err}"
        );
    }

    #[test]
    fn build_notification_channel_update_request_rejects_non_boolean_enabled() {
        let params = serde_json::json!({
            "config": {"url": "https://example.invalid/hook"},
            "enabled": 1
        });
        let params = params.as_object().expect("params should be an object");

        let result = build_notification_channel_update_request(params);
        let err = result.expect_err("non-boolean enabled must be rejected");
        assert!(
            err.contains("enabled"),
            "expected enabled validation error, got: {err}"
        );
    }

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
        let create_params = create_params
            .as_object()
            .expect("create params should be an object");
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
            "id": Uuid::now_v7().to_string(),
            "config": {
                "to_addresses": ["carol@example.com", "dave@example.com"]
            }
        });
        let update_params = update_params
            .as_object()
            .expect("update params should be an object");
        let update_request = build_notification_channel_update_request(update_params)
            .expect("update request should build");
        assert_eq!(
            update_request.config,
            Some(
                serde_json::from_value::<
                    uptrakit_web_api_types::notifications::channels::JsonObjectInput,
                >(serde_json::json!({
                    "to_addresses": ["carol@example.com", "dave@example.com"]
                }),)
                .expect("valid JsonObjectInput")
            ),
            "config JSON object must be passed through unchanged for update"
        );
    }

    #[tokio::test]
    async fn invoke_allowlisted_notification_row_actions_use_controller_owned_path() {
        ensure_master_key();
        let db = setup_notification_db().await;
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("catalog should build"),
        );

        let service_connections = ServiceConnectionRegistry::new();
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));

        for interaction_id in ["edit", "test", "delete"] {
            let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
            registry
                .bootstrap_plugin(notification_channel_registration(
                    "plugin.webhook",
                    "notifications.webhook",
                    interaction_id,
                ))
                .expect("plugin registration should succeed");

            let mut params = serde_json::Map::new();
            params.insert(
                "id".to_string(),
                serde_json::json!(Uuid::now_v7().to_string()),
            );
            params.insert("name".to_string(), serde_json::json!("Updated Hook"));
            params.insert(
                "url".to_string(),
                serde_json::json!("https://example.invalid/updated"),
            );
            params.insert("enabled".to_string(), serde_json::json!(true));

            let err = proxy
                .invoke(
                    &service_connections,
                    &registry,
                    SurfaceInvokeRequest {
                        tenant_id: tenant_id(),
                        surface_id: "notifications.webhook".to_string(),
                        interaction_id: interaction_id.to_string(),
                        idempotency_key: format!("idem-notification-{interaction_id}"),
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
                .expect_err("row action on missing channel should fail");

            let SurfaceProxyError::SchemaValidationFailed(message) = err else {
                panic!("unexpected error type for {interaction_id}: {err:?}");
            };
            assert!(
                message.contains("Channel not found"),
                "expected controller-owned not-found for {interaction_id}, got: {message}"
            );

            let action_type = match interaction_id {
                "edit" => uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_UPDATE,
                "test" => uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_TEST,
                "delete" => uptrakit_audit_log::AuditActionType::NOTIFICATION_CHANNEL_DELETE,
                _ => unreachable!(),
            };
            let expected_outcome = match interaction_id {
                "test" => uptrakit_audit_log::AuditOutcome::Failed,
                _ => uptrakit_audit_log::AuditOutcome::Denied,
            };
            let row =
                latest_tenant_audit_row_for_action_and_outcome(&db, action_type, expected_outcome)
                    .await;
            assert_eq!(row.actor_id, Some(user_id()));
            assert_eq!(row.target_type.as_deref(), Some("notification_channel"));
            let details = row.details_json.expect("audit details");
            assert_eq!(
                details["reason_code"],
                serde_json::json!("channel_not_found")
            );
            assert_eq!(
                details["create_source"],
                serde_json::json!(format!(
                    "surface_proxy.notification_channel.{interaction_id}"
                ))
            );
        }
    }

    #[tokio::test]
    async fn invoke_proxmox_add_config_executes_controller_owned_create_path() {
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

        let proxy =
            SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );
        params.insert("verify_tls".to_string(), serde_json::json!(false));
        params.insert(
            "node_filter".to_string(),
            serde_json::json!(" node-a, , node-b "),
        );

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.hosts".to_string(),
                    interaction_id: "add-config".to_string(),
                    idempotency_key: "idem-proxmox-add-config".to_string(),
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
            .expect("proxmox add-config should execute on the controller-owned create path");

        assert!(response.success);
        let result = response
            .result
            .expect("proxmox add-config should return created plugin-config payload");
        assert_eq!(result["name"], "PVE Cluster");
        assert_eq!(result["plugin_type"], "infrastructure_proxmox");
        assert_eq!(result["enabled"], true);
        assert_eq!(result["config"]["api_url"], "https://pve.local:8006");
        assert_eq!(result["config"]["verify_tls"], false);
        assert_eq!(
            result["config"]["node_filter"],
            serde_json::json!(["node-a", "node-b"])
        );

        let persisted = uptrakit_shared_db::entity::plugin_config::Entity::find()
            .filter(uptrakit_shared_db::entity::plugin_config::Column::TenantId.eq(tenant_id()))
            .one(&db)
            .await
            .expect("plugin config query should succeed")
            .expect("proxmox add-config should create a plugin config row");
        assert_eq!(persisted.name, "PVE Cluster");
        assert_eq!(persisted.plugin_type, "infrastructure_proxmox");
        assert!(persisted.enabled);
    }

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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );
        params.insert("verify_tls".to_string(), serde_json::json!(false));

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
        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id()));
        assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["create_source"],
            serde_json::json!("surface_proxy.proxmox_add_config")
        );
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("infrastructure_proxmox")
        );
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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );
        params.insert(
            "verify_tls".to_string(),
            serde_json::json!("definitely-not-bool"),
        );

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
            .expect_err("invalid verify_tls string should be rejected");
        assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["create_source"],
            serde_json::json!("surface_proxy.proxmox_add_config")
        );
        assert_eq!(
            details["reason_code"],
            serde_json::json!("validation_failed")
        );
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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );

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
            .expect_err("duplicate proxmox add-config create should fail");
        assert!(matches!(err, SurfaceProxyError::Conflict { .. }));

        let row = latest_tenant_audit_row_for_action_and_outcome(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
            uptrakit_audit_log::AuditOutcome::Failed,
        )
        .await;
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["create_source"],
            serde_json::json!("surface_proxy.proxmox_add_config")
        );
        assert_eq!(details["reason_code"], serde_json::json!("duplicate_name"));
        assert_eq!(details["error_kind"], serde_json::json!("conflict"));
    }

    #[tokio::test]
    async fn invoke_proxmox_save_global_defaults_emits_success_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        ensure_proxmox_update_protection_tables(&db).await;
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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert(
            "plugin_config_id".to_string(),
            serde_json::json!(plugin_config_id.to_string()),
        );
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
        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id()));
        assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
        let plugin_config_id_str = plugin_config_id.to_string();
        assert_eq!(
            row.target_id.as_deref(),
            Some(plugin_config_id_str.as_str())
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.proxmox_update_protection.save_global_defaults")
        );
        assert_eq!(
            details["plugin_type"],
            serde_json::json!("infrastructure_proxmox")
        );
    }

    #[tokio::test]
    async fn invoke_proxmox_save_global_defaults_missing_config_emits_denied_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        ensure_proxmox_update_protection_tables(&db).await;
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
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert(
            "plugin_config_id".to_string(),
            serde_json::json!(Uuid::now_v7().to_string()),
        );
        params.insert("mode".to_string(), serde_json::json!("do_nothing"));

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.settings.update-protection".to_string(),
                    interaction_id: "save-global-defaults".to_string(),
                    idempotency_key: "idem-proxmox-save-global-defaults-failure".to_string(),
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
            .expect_err("save-global-defaults should reject unknown plugin config");

        let SurfaceProxyError::SchemaValidationFailed(message) = err else {
            panic!("unexpected error variant: {err:?}");
        };
        assert!(
            message.contains("not found in tenant scope"),
            "expected missing-config message, got: {message}"
        );

        let row = latest_tenant_audit_row_for_action_and_outcome(
            &db,
            uptrakit_audit_log::AuditActionType::TENANT_SETTING_UPDATE,
            uptrakit_audit_log::AuditOutcome::Denied,
        )
        .await;
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.proxmox_update_protection.save_global_defaults")
        );
        assert_eq!(
            details["reason_code"],
            serde_json::json!("resource_not_available")
        );
    }

    #[tokio::test]
    async fn invoke_proxmox_save_item_overrides_emits_success_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        ensure_proxmox_update_protection_tables(&db).await;
        let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
        let software_item_id = insert_software_item(&db).await;
        assign_proxmox_plugin_to_software_item(&db, software_item_id, plugin_config_id).await;
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
                "proxmox.software-item.update-protection",
                "save-item-overrides",
            ))
            .expect("plugin registration should succeed");

        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert(
            "software_item_id".to_string(),
            serde_json::json!(software_item_id.to_string()),
        );
        params.insert(
            "plugin_config_id".to_string(),
            serde_json::json!(plugin_config_id.to_string()),
        );
        params.insert("mode".to_string(), serde_json::json!("inherit_global"));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.software-item.update-protection".to_string(),
                    interaction_id: "save-item-overrides".to_string(),
                    idempotency_key: "idem-proxmox-save-item-overrides-success".to_string(),
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
            .expect("save-item-overrides should succeed");

        assert!(response.success);
        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id()));
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        let software_item_id_str = software_item_id.to_string();
        assert_eq!(
            row.target_id.as_deref(),
            Some(software_item_id_str.as_str())
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.proxmox_update_protection.save_item_overrides")
        );
        assert_eq!(
            details["plugin_config_id"],
            serde_json::json!(plugin_config_id.to_string())
        );
    }

    #[tokio::test]
    async fn invoke_proxmox_save_item_overrides_unassigned_config_emits_denied_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        ensure_proxmox_update_protection_tables(&db).await;
        let plugin_config_id = insert_active_proxmox_plugin_config(&db).await;
        let software_item_id = insert_software_item(&db).await;
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
                "proxmox.software-item.update-protection",
                "save-item-overrides",
            ))
            .expect("plugin registration should succeed");

        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert(
            "software_item_id".to_string(),
            serde_json::json!(software_item_id.to_string()),
        );
        params.insert(
            "plugin_config_id".to_string(),
            serde_json::json!(plugin_config_id.to_string()),
        );
        params.insert("mode".to_string(), serde_json::json!("inherit_global"));

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.software-item.update-protection".to_string(),
                    interaction_id: "save-item-overrides".to_string(),
                    idempotency_key: "idem-proxmox-save-item-overrides-failure".to_string(),
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
            .expect_err("save-item-overrides should reject unassigned plugin config");

        let SurfaceProxyError::SchemaValidationFailed(message) = err else {
            panic!("unexpected error variant: {err:?}");
        };
        assert!(
            message.contains("not assigned to software item"),
            "expected unassigned-config message, got: {message}"
        );

        let row = latest_tenant_audit_row_for_action_and_outcome(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
            uptrakit_audit_log::AuditOutcome::Denied,
        )
        .await;
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.proxmox_update_protection.save_item_overrides")
        );
        assert_eq!(
            details["reason_code"],
            serde_json::json!("resource_not_available")
        );
    }

    #[tokio::test]
    async fn proxmox_update_protection_bootstrap_creates_timeout_columns() {
        use sea_orm::{ConnectionTrait, DbBackend, Statement, TryGetable as _};

        ensure_master_key();
        let db = setup_notification_db().await;
        ensure_proxmox_update_protection_tables(&db).await;

        let rows = db
            .query_all_raw(Statement::from_string(
                DbBackend::Sqlite,
                "PRAGMA table_info(proxmox_protection_defaults)".to_string(),
            ))
            .await
            .unwrap();
        let names: Vec<String> = rows
            .into_iter()
            .map(|row| String::try_get(&row, "", "name").unwrap())
            .collect();

        assert!(names.contains(&"snapshot_timeout_seconds".to_string()));
        assert!(names.contains(&"backup_timeout_seconds".to_string()));
    }

    #[tokio::test]
    async fn invoke_docker_switch_tag_success_emits_software_item_update_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        let software_item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let seen = StdArc::new(Mutex::new(Vec::new()));
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(TestPluginInvoker {
                    response: serde_json::json!({"ok": true}),
                    seen: StdArc::clone(&seen),
                }),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
            .expect("plugin registration should succeed");

        let mut params = serde_json::Map::new();
        params.insert(
            "software_item_id".to_string(),
            serde_json::json!(software_item_id.to_string()),
        );
        params.insert(
            "host_id".to_string(),
            serde_json::json!(host_id.to_string()),
        );
        params.insert(
            "new_image_ref".to_string(),
            serde_json::json!("ghcr.io/example/app:26.2.6"),
        );

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

        let row = latest_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id()));
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(software_item_id.to_string().as_str())
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.docker_switch_tag")
        );
        assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
        assert_eq!(
            details["new_image_ref"],
            serde_json::json!("ghcr.io/example/app:26.2.6")
        );
    }

    #[tokio::test]
    async fn invoke_docker_switch_tag_invalid_image_emits_validation_failed_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        let software_item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(ErrorPluginInvoker {
                    error_message: "invalid image reference: bad tag".to_string(),
                }),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
            .expect("plugin registration should succeed");

        let mut params = serde_json::Map::new();
        params.insert(
            "software_item_id".to_string(),
            serde_json::json!(software_item_id.to_string()),
        );
        params.insert(
            "host_id".to_string(),
            serde_json::json!(host_id.to_string()),
        );
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

        let row = latest_tenant_audit_row_for_action_and_outcome(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
        )
        .await;
        assert_eq!(row.target_type.as_deref(), Some("software_item"));
        assert_eq!(
            row.target_id.as_deref(),
            Some(software_item_id.to_string().as_str())
        );
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.docker_switch_tag")
        );
        assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
        assert_eq!(details["reason_code"], serde_json::json!("invalid_request"));
        assert_eq!(details["new_image_ref"], serde_json::json!("bad ref"));
    }

    #[tokio::test]
    async fn invoke_docker_switch_tag_missing_assignment_emits_denied_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        let software_item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(ErrorPluginInvoker {
                    error_message: "no plugin assignments found for this host".to_string(),
                }),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
            .expect("plugin registration should succeed");

        let mut params = serde_json::Map::new();
        params.insert(
            "software_item_id".to_string(),
            serde_json::json!(software_item_id.to_string()),
        );
        params.insert(
            "host_id".to_string(),
            serde_json::json!(host_id.to_string()),
        );
        params.insert(
            "new_image_ref".to_string(),
            serde_json::json!("ghcr.io/example/app:26.2.6"),
        );

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "docker.item-host-actions".to_string(),
                    interaction_id: "switch-tag".to_string(),
                    idempotency_key: "idem-docker-switch-tag-missing".to_string(),
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
            .expect_err("switch-tag should reject missing assignment");

        assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

        let row = latest_tenant_audit_row_for_action_and_outcome(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
            uptrakit_audit_log::AuditOutcome::Denied,
        )
        .await;
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.docker_switch_tag")
        );
        assert_eq!(
            details["reason_code"],
            serde_json::json!("host_assignment_not_found")
        );
    }

    #[tokio::test]
    async fn invoke_docker_switch_tag_storage_error_emits_failed_audit_row() {
        ensure_master_key();
        let db = setup_notification_db().await;
        let software_item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let proxy = SurfaceProxy::new().with_local_executor(Arc::new(
            PluginSurfaceLocalExecutor::new(
                Arc::new(db.clone()),
                Arc::new(ErrorPluginInvoker {
                    error_message: "failed to commit transaction: locked".to_string(),
                }),
            )
            .with_audit_emitter(test_audit_emitter(db.clone())),
        ));
        let service_connections = ServiceConnectionRegistry::new();
        let registry = SurfaceRegistry::new(SurfaceRegistryConfig::default());
        registry
            .bootstrap_plugin(docker_switch_tag_registration("plugin.releases_docker"))
            .expect("plugin registration should succeed");

        let mut params = serde_json::Map::new();
        params.insert(
            "software_item_id".to_string(),
            serde_json::json!(software_item_id.to_string()),
        );
        params.insert(
            "host_id".to_string(),
            serde_json::json!(host_id.to_string()),
        );
        params.insert(
            "new_image_ref".to_string(),
            serde_json::json!("ghcr.io/example/app:26.2.6"),
        );

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "docker.item-host-actions".to_string(),
                    interaction_id: "switch-tag".to_string(),
                    idempotency_key: "idem-docker-switch-tag-storage-error".to_string(),
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
            .expect_err("switch-tag should fail on storage error");

        assert!(matches!(err, SurfaceProxyError::SchemaValidationFailed(_)));

        let row = latest_tenant_audit_row_for_action_and_outcome(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE,
            uptrakit_audit_log::AuditOutcome::Failed,
        )
        .await;
        let details = row.details_json.expect("audit details");
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("surface_proxy.docker_switch_tag")
        );
        assert_eq!(details["reason_code"], serde_json::json!("storage_error"));
        assert_eq!(details["host_id"], serde_json::json!(host_id.to_string()));
    }

    #[tokio::test]
    async fn invoke_proxmox_add_config_accepts_legacy_string_verify_tls_values() {
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

        let proxy =
            SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
                Arc::new(db),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );
        params.insert("verify_tls".to_string(), serde_json::json!("false"));

        let response = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.hosts".to_string(),
                    interaction_id: "add-config".to_string(),
                    idempotency_key: "idem-proxmox-add-config-invalid-verify".to_string(),
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
            .expect("legacy string verify_tls should remain accepted");

        assert!(response.success);
        let result = response
            .result
            .expect("legacy string verify_tls should return created payload");
        assert_eq!(result["config"]["verify_tls"], false);
    }

    #[tokio::test]
    async fn invoke_proxmox_add_config_rejects_invalid_verify_tls_type() {
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

        let proxy =
            SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
                Arc::new(db),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );
        params.insert(
            "verify_tls".to_string(),
            serde_json::json!("definitely-not-bool"),
        );

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.hosts".to_string(),
                    interaction_id: "add-config".to_string(),
                    idempotency_key: "idem-proxmox-add-config-invalid-verify".to_string(),
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
            .expect_err("invalid verify_tls string should be rejected");

        let SurfaceProxyError::SchemaValidationFailed(message) = err else {
            panic!("unexpected error variant: {err:?}");
        };
        assert!(
            message.contains("verify_tls"),
            "expected verify_tls validation error, got: {message}"
        );
    }

    #[tokio::test]
    async fn invoke_proxmox_add_config_rejects_invalid_node_filter_type() {
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

        let proxy =
            SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
                Arc::new(db),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );
        params.insert("verify_tls".to_string(), serde_json::json!(true));
        params.insert("node_filter".to_string(), serde_json::json!(123));

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.hosts".to_string(),
                    interaction_id: "add-config".to_string(),
                    idempotency_key: "idem-proxmox-add-config-invalid-node-filter".to_string(),
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
            .expect_err("non-string/array node_filter should be rejected");

        let SurfaceProxyError::SchemaValidationFailed(message) = err else {
            panic!("unexpected error variant: {err:?}");
        };
        assert!(
            message.contains("node_filter"),
            "expected node_filter validation error, got: {message}"
        );
    }

    #[tokio::test]
    async fn invoke_proxmox_add_config_preserves_duplicate_name_conflict() {
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

        let proxy =
            SurfaceProxy::new().with_local_executor(Arc::new(PluginSurfaceLocalExecutor::new(
                Arc::new(db),
                Arc::new(PluginOpsSurfaceActionInvoker::new(Arc::clone(&plugin_ops))),
            )));
        let service_connections = ServiceConnectionRegistry::new();

        let mut params = serde_json::Map::new();
        params.insert("name".to_string(), serde_json::json!("PVE Cluster"));
        params.insert(
            "api_url".to_string(),
            serde_json::json!("https://pve.local:8006"),
        );
        params.insert(
            "api_token".to_string(),
            serde_json::json!("root@pam!uptrakit=secret-token"),
        );

        proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.hosts".to_string(),
                    interaction_id: "add-config".to_string(),
                    idempotency_key: "idem-proxmox-add-config-1".to_string(),
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
            .expect("first proxmox add-config create should succeed");

        let err = proxy
            .invoke(
                &service_connections,
                &registry,
                SurfaceInvokeRequest {
                    tenant_id: tenant_id(),
                    surface_id: "proxmox.hosts".to_string(),
                    interaction_id: "add-config".to_string(),
                    idempotency_key: "idem-proxmox-add-config-2".to_string(),
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
            .expect_err("duplicate proxmox add-config create should fail");

        let SurfaceProxyError::Conflict { code, message } = err else {
            panic!("unexpected error variant: {err:?}");
        };
        assert_eq!(code, "duplicate_name");
        assert!(
            message.contains("already exists"),
            "expected duplicate-name conflict message, got: {message}"
        );
    }
}
