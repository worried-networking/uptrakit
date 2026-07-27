use super::SurfaceProxyError;
use uptrakit_plugin_infrastructure_registry::{SurfaceActionController, SurfaceActionError};
use uptrakit_wire::surfaces::InteractionHttpMethod;
use uuid::Uuid;

mod docker;
mod notification_settings;
mod notifications;
mod params;
mod proxmox_update_protection;

pub(crate) use docker::{
    allowlisted_docker_switch_tag_controller_local_action, emit_docker_switch_tag_audit_event,
};
pub(crate) use notification_settings::{
    allowlisted_notification_settings_controller_local_action,
    emit_notification_settings_audit_event,
};
pub(crate) use notifications::{
    allowlisted_notification_channel_controller_local_action, channel_op_name,
    emit_notification_channel_audit_event, execute_allowlisted_notification_channel_action,
    notification_channel_type_for_surface_id,
};
#[cfg(test)]
pub(crate) use notifications::{
    build_notification_channel_create_request, build_notification_channel_update_request,
};
pub(crate) use proxmox_update_protection::{
    allowlisted_proxmox_update_protection_controller_local_action,
    emit_proxmox_update_protection_audit_event,
};

/// Which executor tier owns a `(surface_id, interaction_id)` pair
/// (`local_executor.rs` tier ladder). `ControllerExecutes` = Tier 1
/// (controller-side code + audit, no plugin call, delivery
/// `ControllerExecutor`); `PluginWithAudit` = Tier 2 (plugin invoke + audit,
/// delivery `PluginHandled`). Tier 3 (plugin invoke, no audit) is the
/// fallthrough and has no rows. Guarded bidirectionally by
/// `interaction_executor_guard` in web-api (spec D5).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorTier {
    /// Tier 1: executed by controller-side code in `controller_local/`.
    ControllerExecutes,
    /// Tier 2: dispatched to the plugin, audited by the executor.
    PluginWithAudit,
}

/// Single source for every allowlisted controller-local executor pair. Rows
/// name `MutationAction`/`FormSubmit`/`ConfirmableAction` interactions (none
/// are `DataLoad`), keyed by their declared REST method: CRUD collections
/// collapse onto one noun with distinct methods (`channels` Post/Put/Delete),
/// while `test` and the settings writes stay `Post`/`Put`; verified against
/// each interaction's actual registration, not guessed from the name. Re-keyed
/// and checked bidirectionally by `interaction_executor_guard` in web-api
/// (spec D5).
pub const CONTROLLER_LOCAL_EXECUTOR_TABLE: &[(&str, &str, InteractionHttpMethod, ExecutorTier)] = &[
    // Tier 1a — notification channel CRUD (collapsed onto the `channels`
    // collection noun, disambiguated by REST method; `test` keeps its own id).
    (
        "notifications.webhook",
        "channels",
        InteractionHttpMethod::Post,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.webhook",
        "channels",
        InteractionHttpMethod::Put,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.webhook",
        "test",
        InteractionHttpMethod::Post,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.webhook",
        "channels",
        InteractionHttpMethod::Delete,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.telegram",
        "channels",
        InteractionHttpMethod::Post,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.telegram",
        "channels",
        InteractionHttpMethod::Put,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.telegram",
        "test",
        InteractionHttpMethod::Post,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.telegram",
        "channels",
        InteractionHttpMethod::Delete,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.email",
        "channels",
        InteractionHttpMethod::Post,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.email",
        "channels",
        InteractionHttpMethod::Put,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.email",
        "test",
        InteractionHttpMethod::Post,
        ExecutorTier::ControllerExecutes,
    ),
    (
        "notifications.email",
        "channels",
        InteractionHttpMethod::Delete,
        ExecutorTier::ControllerExecutes,
    ),
    // Tier 2a — notification settings saves
    (
        "notifications.email",
        "smtp",
        InteractionHttpMethod::Put,
        ExecutorTier::PluginWithAudit,
    ),
    (
        "notifications.email.global-smtp",
        "smtp",
        InteractionHttpMethod::Put,
        ExecutorTier::PluginWithAudit,
    ),
    (
        "notifications.telegram.global-settings",
        "settings",
        InteractionHttpMethod::Put,
        ExecutorTier::PluginWithAudit,
    ),
    // Tier 2b — docker switch-tag
    (
        "docker.item-host-actions",
        "switch-tag",
        InteractionHttpMethod::Post,
        ExecutorTier::PluginWithAudit,
    ),
    // Tier 2c — proxmox update-protection / scaling saves
    (
        "proxmox.settings.update-hooks",
        "save-global-defaults",
        InteractionHttpMethod::Post,
        ExecutorTier::PluginWithAudit,
    ),
    (
        "proxmox.software-item.update-hooks",
        "save-item-overrides",
        InteractionHttpMethod::Post,
        ExecutorTier::PluginWithAudit,
    ),
    (
        "proxmox.settings.resource-scaling",
        "save-scaling-global-defaults",
        InteractionHttpMethod::Post,
        ExecutorTier::PluginWithAudit,
    ),
    (
        "proxmox.software-item.resource-scaling",
        "save-scaling-item-overrides",
        InteractionHttpMethod::Post,
        ExecutorTier::PluginWithAudit,
    ),
];

/// Looks up the tier for a `(surface, interaction, method)` triple. Linear
/// scan over a 20-row const — a map would be complexity without a consumer.
pub(crate) fn table_tier(
    surface_id: &str,
    interaction_id: &str,
    method: &InteractionHttpMethod,
) -> Option<ExecutorTier> {
    CONTROLLER_LOCAL_EXECUTOR_TABLE
        .iter()
        .find(|(surface, interaction, row_method, _)| {
            *surface == surface_id && *interaction == interaction_id && row_method == method
        })
        .map(|(_, _, _, tier)| *tier)
}

// TODO(adr-0018): The collapse of ControllerIntegration/PluginInternal → SendFailed is only
// acceptable because the tracing::error! call below preserves controller-side failure detail in
// logs. A subscriber-capture test asserting that error event would lock in this guarantee, but no
// exported tracing-capture helper exists in the workspace without adding a new dev-dependency
// (tracing-subscriber or tracing-test). See `docs/adr/0018-plugin-extension-typed-boundary.md`
// (Consequences § observability gap) for the two resolution paths: export a capture helper from
// uptrakit-tracing-init under a `test-support` feature, or approve tracing-test as a
// surface-proxy dev-dependency so this can be promoted to an assertion.
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

#[cfg(test)]
mod table_tests {
    use super::*;

    #[test]
    fn executor_table_has_no_duplicate_pairs() {
        // `InteractionHttpMethod` derives no `Ord`/`Hash` (wire-safe enum
        // convention); key on its wire string instead of adding either.
        let mut seen = std::collections::BTreeSet::new();
        for (surface, interaction, method, _) in CONTROLLER_LOCAL_EXECUTOR_TABLE {
            assert!(
                seen.insert((*surface, *interaction, method.as_str())),
                "duplicate row ({surface}, {interaction}, {method})"
            );
        }
        assert_eq!(seen.len(), CONTROLLER_LOCAL_EXECUTOR_TABLE.len());
    }
}
