use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_registry::{
    PluginOps, SurfaceActionContext, SurfaceActionError,
};
use uptrakit_wire::surfaces;
use uuid::Uuid;

use super::controller_local::{
    allowlisted_docker_switch_tag_controller_local_action,
    allowlisted_notification_channel_controller_local_action,
    allowlisted_notification_settings_controller_local_action,
    allowlisted_proxmox_add_config_controller_local_action, allowlisted_proxmox_provider,
    allowlisted_proxmox_update_protection_controller_local_action,
    emit_docker_switch_tag_audit_event, emit_notification_channel_audit_event,
    emit_notification_settings_audit_event, emit_proxmox_add_config_audit_event,
    emit_proxmox_update_protection_audit_event, execute_allowlisted_notification_channel_action,
    execute_allowlisted_proxmox_add_config_action, map_surface_action_error,
    notification_channel_type_for_surface_id,
};
use super::{AppStateSurfaceActionController, SurfaceProxyError};

#[async_trait]
pub trait SurfaceLocalActionExecutor: Send + Sync {
    async fn execute(
        &self,
        _resolved: &crate::registry::ResolvedSurfaceAction,
        _request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError>;
}

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

pub(super) struct PluginOpsSurfaceActionInvoker {
    plugin_ops: Arc<dyn PluginOps>,
}

impl PluginOpsSurfaceActionInvoker {
    pub(super) fn new(plugin_ops: Arc<dyn PluginOps>) -> Self {
        Self { plugin_ops }
    }
}

#[async_trait]
impl PluginSurfaceActionInvoker for PluginOpsSurfaceActionInvoker {
    async fn invoke(
        &self,
        db: Option<&DatabaseConnection>,
        tenant_id: Option<Uuid>,
        caller_user_id: Option<Uuid>,
        surface_id: &str,
        interaction_id: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, SurfaceActionError> {
        let tenant_id = tenant_id.ok_or_else(|| {
            SurfaceActionError::InvalidInput(
                "tenant_id is required for controller-local surface actions".to_string(),
            )
        })?;
        let db = db.ok_or_else(|| {
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
}

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
            let channel_type =
                notification_channel_type_for_surface_id(resolved.descriptor.surface_id.as_str())
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

pub(super) struct NoopSurfaceLocalExecutor;

#[async_trait]
impl SurfaceLocalActionExecutor for NoopSurfaceLocalExecutor {
    async fn execute(
        &self,
        resolved: &crate::registry::ResolvedSurfaceAction,
        _request: &surfaces::SurfaceActionRequest,
    ) -> Result<serde_json::Value, SurfaceProxyError> {
        Err(SurfaceProxyError::SchemaValidationFailed(format!(
            "local surface transport is not implemented for provider `{}`",
            resolved.provider_id
        )))
    }
}
