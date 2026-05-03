use super::SurfaceProxyError;
use async_trait::async_trait;
use rootcause::report;

use uptrakit_plugin_infrastructure_registry::{
    PluginError, ProxmoxApproveMatchRequest, ProxmoxGlobalDefaultsSaveRequest,
    ProxmoxHostInfoRequest, ProxmoxHostMappingsRequest, ProxmoxItemOverridePreloadRequest,
    ProxmoxItemOverrideSaveRequest, ProxmoxManualMatchRequest, ProxmoxMappingRequest,
    ProxmoxPluginConfigRequest, ProxmoxScopeSelectionRequest, ProxmoxSurfaceStore,
    ProxmoxUnmatchedGuestsRequest, SurfaceActionController, SurfaceActionError,
    execute_proxmox_controller_approve_match, execute_proxmox_controller_discover_hosts,
    execute_proxmox_controller_get_host_info, execute_proxmox_controller_list_all_unmatched,
    execute_proxmox_controller_list_host_mappings,
    execute_proxmox_controller_load_backup_target_options, execute_proxmox_controller_manual_match,
    execute_proxmox_controller_preload_global_defaults,
    execute_proxmox_controller_preload_item_overrides,
    execute_proxmox_controller_save_global_defaults,
    execute_proxmox_controller_save_item_overrides, execute_proxmox_controller_test_connection,
    execute_proxmox_controller_unmatch_host,
};
use uuid::Uuid;

mod docker;
mod notification_settings;
mod notifications;
mod params;
mod proxmox_add_config;
mod proxmox_update_protection;

pub(crate) use docker::{
    allowlisted_docker_switch_tag_controller_local_action, emit_docker_switch_tag_audit_event,
};
pub(crate) use notification_settings::{
    allowlisted_notification_settings_controller_local_action,
    emit_notification_settings_audit_event,
};
pub(crate) use notifications::{
    allowlisted_notification_channel_controller_local_action,
    emit_notification_channel_audit_event, execute_allowlisted_notification_channel_action,
    notification_channel_type_for_surface_id,
};
#[cfg(test)]
pub(crate) use notifications::{
    build_notification_channel_create_request, build_notification_channel_update_request,
};
pub(crate) use proxmox_add_config::{
    allowlisted_proxmox_add_config_controller_local_action, allowlisted_proxmox_provider,
    emit_proxmox_add_config_audit_event, execute_allowlisted_proxmox_add_config_action,
};
pub(crate) use proxmox_update_protection::{
    allowlisted_proxmox_update_protection_controller_local_action,
    emit_proxmox_update_protection_audit_event,
};

pub fn map_surface_action_error(err: SurfaceActionError) -> SurfaceProxyError {
    match err {
        SurfaceActionError::InvalidInput(message) => {
            SurfaceProxyError::SchemaValidationFailed(message)
        }
        SurfaceActionError::ControllerIntegration(message)
        | SurfaceActionError::PluginInternal(message) => {
            tracing::error!(error = %message, "controller-local surface action failed");
            SurfaceProxyError::SendFailed
        }
        other => {
            tracing::error!(error = ?other, "unexpected controller-local surface action failure");
            SurfaceProxyError::SendFailed
        }
    }
}

fn plugin_internal_error(error: impl std::fmt::Display) -> rootcause::Report<PluginError> {
    report!(PluginError::PluginInternal(error.to_string()))
}

pub struct AppStateSurfaceActionController<'a> {
    db: &'a sea_orm::DatabaseConnection,
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
    tenant_db: uptrakit_shared_db::TenantDb,
}

impl<'a> AppStateSurfaceActionController<'a> {
    pub fn from_database_connection(
        db: &'a sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        caller_user_id: Option<Uuid>,
    ) -> Self {
        Self {
            db,
            tenant_id,
            caller_user_id,
            tenant_db: uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id),
        }
    }

    fn db(&self) -> &sea_orm::DatabaseConnection {
        self.db
    }
}

impl SurfaceActionController for AppStateSurfaceActionController<'_> {
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    fn user_id(&self) -> Option<Uuid> {
        self.caller_user_id
    }

    fn tenant_db(&self) -> &uptrakit_shared_db::TenantDb {
        &self.tenant_db
    }

    fn proxmox_surface_store(&self) -> Option<&dyn ProxmoxSurfaceStore> {
        Some(self)
    }
}

#[async_trait]
impl ProxmoxSurfaceStore for AppStateSurfaceActionController<'_> {
    async fn list_host_mappings(
        &self,
        request: ProxmoxHostMappingsRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        let params = serde_json::to_value(request).map_err(plugin_internal_error)?;
        execute_proxmox_controller_list_host_mappings(self.db(), Some(self.tenant_id), params)
            .await
            .map_err(plugin_internal_error)
    }

    async fn discover_hosts(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_discover_hosts(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn test_connection(
        &self,
        request: ProxmoxPluginConfigRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_test_connection(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn match_host(
        &self,
        request: ProxmoxManualMatchRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_manual_match(self.db(), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn approve_match(
        &self,
        request: ProxmoxApproveMatchRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_approve_match(self.db(), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn unmatch_host(
        &self,
        request: ProxmoxMappingRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_unmatch_host(self.db(), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn list_all_unmatched(
        &self,
        request: ProxmoxUnmatchedGuestsRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_list_all_unmatched(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn get_host_info(
        &self,
        request: ProxmoxHostInfoRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_get_host_info(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn preload_global_defaults(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_preload_global_defaults(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn save_global_defaults(
        &self,
        request: ProxmoxGlobalDefaultsSaveRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_save_global_defaults(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn preload_item_overrides(
        &self,
        request: ProxmoxItemOverridePreloadRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_preload_item_overrides(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn save_item_overrides(
        &self,
        request: ProxmoxItemOverrideSaveRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_save_item_overrides(self.db(), Some(self.tenant_id), request)
            .await
            .map_err(plugin_internal_error)
    }

    async fn load_backup_target_options(
        &self,
        request: ProxmoxScopeSelectionRequest,
    ) -> uptrakit_plugin_infrastructure_registry::PluginResult<serde_json::Value> {
        execute_proxmox_controller_load_backup_target_options(
            self.db(),
            Some(self.tenant_id),
            request,
        )
        .await
        .map_err(plugin_internal_error)
    }
}
