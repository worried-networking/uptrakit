//! Shared update-trigger logic used by the REST handler and the MQTT WS handler.

use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, ReleaseInfo};
use uptrakit_shared_db::entity::{
    host, prelude::*, service, service_host, update_history,
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
    /// The provider config referenced by the host assignment was not found.
    #[error("provider config not found")]
    ProviderConfigNotFound,
    /// The provider type stored in the config could not be deserialized.
    #[error("unknown provider type: {0}")]
    UnknownProviderType(String),
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

    // 3. Verify host is assigned to the software item and load per-host provider info.
    let link = HostSoftwareItem::find_by_id((host_id, item_id))
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

    // 7. Load provider config from the host-specific assignment.
    let provider_config = uptrakit_shared_db::entity::prelude::ProviderConfig::find_by_id(
        link.provider_config_id,
    )
    .filter(
        uptrakit_shared_db::entity::provider_config::Column::DeactivatedAt.is_null(),
    )
    .one(db)
    .await?
    .ok_or(TriggerUpdateError::ProviderConfigNotFound)?;

    // 8. Create update_history record with status = Pending.
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

    // 9. Resolve hooks from provider config + per-host config_override.
    let resolved_hooks = crate::update_hooks::resolve_hooks(
        &provider_config.config,
        link.config_override.as_ref(),
    );

    // 10. Merge config.
    let merged_config =
        crate::update_hooks::merge_config(&provider_config.config, link.config_override.as_ref());

    // 11. Convert provider type.
    let provider_type: uptrakit_internal_wire::ProviderType = serde_json::from_value(
        serde_json::Value::String(provider_config.provider_type.clone()),
    )
    .map_err(|_| TriggerUpdateError::UnknownProviderType(provider_config.provider_type.clone()))?;

    // 12. Build ExecuteUpdatePayload and dispatch to the agent.
    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        host_machine_id: host_record.machine_id.clone(),
        update_history_id,
        software_item_id: item_id,
        software_item_name: item.name.clone(),
        package_identifier: link.package_identifier.clone(),
        to_version,
        provider_type,
        provider_config: merged_config,
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
