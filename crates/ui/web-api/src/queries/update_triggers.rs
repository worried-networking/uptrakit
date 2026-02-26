//! Shared update-trigger logic used by the REST handler and the MQTT WS handler.

use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, PluginAssignment, ReleaseInfo};
use uptrakit_shared_db::entity::{
    host, host_software_item_plugin, plugin_config, prelude::*, service, service_host,
    update_history,
};
use uuid::Uuid;

use crate::auth::token::generate_uuid;
use crate::notification_service::NotificationService;
use crate::queries::software_items::find_active_item;

/// Error returned by [`trigger_update_for_host`].
#[derive(Debug, thiserror::Error)]
pub enum TriggerUpdateError {
    /// Software item not found or deactivated.
    #[error("software item not found")]
    SoftwareItemNotFound,
    /// Host not found, deactivated, or not scoped to the tenant.
    #[error("host not found")]
    HostNotFound,
    /// The host does not have an assignment record for this software item.
    #[error("host is not assigned to this software item")]
    HostNotAssigned,
    /// No execute_update role plugin assigned for this host-software pair.
    #[error("no execute_update plugin assigned")]
    NoExecuteUpdatePlugin,
    /// No `service_host` link exists for the host.
    #[error("no agent linked to host")]
    NoAgent,
    /// The linked agent exists but is not in `Approved` status.
    #[error("agent is not approved")]
    AgentNotApproved,
    /// A `Pending` or `InProgress` update already exists for this
    /// (host_id, software_item_id) pair.
    #[error("an update is already pending or in progress")]
    UpdateAlreadyActive,
    /// The plugin config referenced by the role assignment was not found.
    #[error("plugin config not found")]
    PluginConfigNotFound,
    /// The plugin type stored in the config could not be deserialized.
    #[error("unknown plugin type: {0}")]
    UnknownPluginType(String),
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(#[from] sea_orm::DbErr),
}

/// Result returned by a successful [`trigger_update_for_host`] call.
pub struct TriggerUpdateResult {
    /// The newly-created `update_history` record ID.
    pub update_history_id: Uuid,
    /// Whether the target agent was locally connected at dispatch time.
    pub agent_connected: bool,
}

/// Parameters for [`trigger_update_for_host`].
pub struct TriggerUpdateParams<'a> {
    pub tenant_id: Uuid,
    pub item_id: Uuid,
    pub host_id: Uuid,
    pub to_version: String,
    /// `"user"`, `"mqtt"`, or `"scheduler"`.
    pub actor_type: &'a str,
    /// User UUID string, MQTT client UUID string, or empty string.
    pub actor_id: &'a str,
    /// Optional release metadata supplied by the REST caller.
    /// `None` when triggered from MQTT or a scheduler.
    pub release_info: Option<ReleaseInfo>,
}

/// Load a role-specific plugin assignment for a host-software pair.
///
/// Returns `None` if no assignment with the given role exists.
async fn load_role_plugin(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    role: &str,
) -> Result<Option<(host_software_item_plugin::Model, plugin_config::Model)>, TriggerUpdateError> {
    let assignment = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .one(db)
        .await?;

    let Some(assignment) = assignment else {
        return Ok(None);
    };

    let config = PluginConfig::find_by_id(assignment.plugin_config_id)
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await?
        .ok_or(TriggerUpdateError::PluginConfigNotFound)?;

    Ok(Some((assignment, config)))
}

/// Build a [`PluginAssignment`] from a role plugin row and its config.
fn build_plugin_assignment(
    assignment: &host_software_item_plugin::Model,
    config: &plugin_config::Model,
) -> Result<PluginAssignment, TriggerUpdateError> {
    let plugin_type: uptrakit_internal_wire::PluginType =
        serde_json::from_value(serde_json::Value::String(config.plugin_type.clone()))
            .map_err(|_| TriggerUpdateError::UnknownPluginType(config.plugin_type.clone()))?;

    let merged_config =
        crate::update_hooks::merge_config(&config.config, assignment.config_override.as_ref());

    Ok(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}

/// Core update-trigger logic shared by the REST handler and the MQTT WS handler.
///
/// Validates the software item, host, agent, and existing-update constraints,
/// then creates an `update_history` record and dispatches an `ExecuteUpdate`
/// message to the agent via `notifier`.
///
/// # Errors
///
/// Returns a [`TriggerUpdateError`] describing the first validation failure or
/// database error encountered.
pub async fn trigger_update_for_host(
    db: &DatabaseConnection,
    notifier: &NotificationService,
    params: TriggerUpdateParams<'_>,
) -> Result<TriggerUpdateResult, TriggerUpdateError> {
    let TriggerUpdateParams {
        tenant_id,
        item_id,
        host_id,
        to_version,
        actor_type,
        actor_id,
        release_info,
    } = params;
    // 1. Verify software item exists and is active.
    let item = find_active_item(db, tenant_id, item_id)
        .await
        .ok_or(TriggerUpdateError::SoftwareItemNotFound)?;

    // 2. Verify host exists, is active, and belongs to the tenant.
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await?
        .ok_or(TriggerUpdateError::HostNotFound)?;

    // 3. Verify host is assigned to the software item.
    let _link = HostSoftwareItem::find_by_id((host_id, item_id))
        .one(db)
        .await?
        .ok_or(TriggerUpdateError::HostNotAssigned)?;

    // 4. Find the agent linked to this host.
    let agent_link = ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .one(db)
        .await?
        .ok_or(TriggerUpdateError::NoAgent)?;

    // 5. Verify agent exists and is approved.
    let agent = Service::find_by_id(agent_link.service_id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(db)
        .await?
        .ok_or(TriggerUpdateError::NoAgent)?;

    if agent.status != service::ServiceStatus::Approved {
        return Err(TriggerUpdateError::AgentNotApproved);
    }

    // 6. Check for pending/in_progress updates for this (host_id, software_item_id).
    let existing_update = UpdateHistory::find()
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::SoftwareItemId.eq(item_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Pending,
            update_history::UpdateStatus::InProgress,
        ]))
        .one(db)
        .await?;

    if existing_update.is_some() {
        return Err(TriggerUpdateError::UpdateAlreadyActive);
    }

    // 7. Load role-specific plugin assignments.
    let execute_update_data = load_role_plugin(db, host_id, item_id, "execute_update")
        .await?
        .ok_or(TriggerUpdateError::NoExecuteUpdatePlugin)?;

    let detect_version_data = load_role_plugin(db, host_id, item_id, "detect_version").await?;

    let execute_update_plugin =
        build_plugin_assignment(&execute_update_data.0, &execute_update_data.1)?;

    let detect_version_plugin = detect_version_data
        .as_ref()
        .map(|(a, c)| build_plugin_assignment(a, c))
        .transpose()?;

    // 8. Resolve hooks from the execute_update plugin config + per-role override.
    let resolved_hooks = crate::update_hooks::resolve_hooks(
        &execute_update_data.1.config,
        execute_update_data.0.config_override.as_ref(),
    );

    // 9. Create update_history record with status = Pending.
    let now = OffsetDateTime::now_utc();
    let update_history_id = generate_uuid();
    let update_record = update_history::ActiveModel {
        id: Set(update_history_id),
        host_id: Set(host_id),
        software_item_id: Set(item_id),
        from_version: Set(None),
        to_version: Set(to_version.clone()),
        status: Set(update_history::UpdateStatus::Pending),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set(actor_type.to_string()),
        actor_id: Set(actor_id.to_string()),
        started_at: Set(now),
        completed_at: Set(None),
        created_at: Set(now),
    };

    update_record.insert(db).await?;

    // 10. Build ExecuteUpdatePayload and dispatch to the agent.
    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        host_machine_id: host_record.machine_id.clone(),
        update_history_id,
        software_item_id: item_id,
        software_item_name: item.name.clone(),
        to_version,
        detect_version_plugin,
        execute_update_plugin,
        pre_update_hooks: resolved_hooks.pre_update_hooks,
        post_update_hooks: resolved_hooks.post_update_hooks,
        release_info,
        timeout_seconds: 300,
    };

    let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
    let agent_connected = notifier.send(&agent.id, msg).await;

    if agent_connected {
        tracing::info!(
            update_id = %update_history_id,
            agent_id = %agent.id,
            host = %host_record.friendly_name,
            software = %item.name,
            "update sent to connected agent"
        );
    } else {
        tracing::info!(
            update_id = %update_history_id,
            agent_id = %agent.id,
            host = %host_record.friendly_name,
            software = %item.name,
            "agent not connected locally, update queued (outbox written for cross-controller delivery)"
        );
    }

    Ok(TriggerUpdateResult {
        update_history_id,
        agent_connected,
    })
}
