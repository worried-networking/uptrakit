use super::SurfaceProxyError;
use uptrakit_plugin_infrastructure_registry::{SurfaceActionController, SurfaceActionError};
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

pub struct AppStateSurfaceActionController {
    tenant_id: Uuid,
    caller_user_id: Option<Uuid>,
    tenant_db: uptrakit_shared_db::TenantDb,
}

impl AppStateSurfaceActionController {
    pub fn from_database_connection(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        caller_user_id: Option<Uuid>,
    ) -> Self {
        Self {
            tenant_id,
            caller_user_id,
            tenant_db: uptrakit_shared_db::TenantDb::new(db.clone(), tenant_id),
        }
    }
}

impl SurfaceActionController for AppStateSurfaceActionController {
    fn tenant_id(&self) -> Uuid {
        self.tenant_id
    }

    fn user_id(&self) -> Option<Uuid> {
        self.caller_user_id
    }

    fn tenant_db(&self) -> &uptrakit_shared_db::TenantDb {
        &self.tenant_db
    }
}
