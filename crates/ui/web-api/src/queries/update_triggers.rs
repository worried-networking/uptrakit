//! Shared update-trigger logic used by the REST handler and the MQTT WS handler.
//!
//! The trigger pipeline is split into three composable layers:
//!
//! 1. [`validate_update_preconditions`] — verifies all preconditions and loads
//!    the data needed for record creation and dispatch.
//! 2. [`create_update_history_record`] — inserts a Pending `update_history` row.
//! 3. [`dispatch_update_to_agent`] — builds the `ExecuteUpdate` payload and
//!    sends it to the agent via `NotificationService`.
//!
//! [`trigger_update_for_host`] is a convenience wrapper that calls all three
//! sequentially. The batch update code path calls them independently (bulk
//! validation, bulk insert, selective dispatch).

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, PluginAssignment, ReleaseInfo};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
    service_host, software_item, update_history,
};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use crate::auth::token::generate_uuid;
use crate::notification_service::NotificationService;
use crate::queries::software_items::find_active_item;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error returned by the update-trigger pipeline.
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
    Database(sea_orm::DbErr),
}

pub type Result<T> = std::result::Result<T, rootcause::Report<TriggerUpdateError>>;
impl_report_conversion!(sea_orm::DbErr => TriggerUpdateError::Database);

// ---------------------------------------------------------------------------
// Public structs
// ---------------------------------------------------------------------------

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

/// All data loaded and validated during [`validate_update_preconditions`].
///
/// Carries everything needed for record creation and dispatch so that
/// callers do not need to repeat any DB lookups.
pub struct ValidatedUpdateTarget {
    pub item: software_item::Model,
    pub host: host::Model,
    pub hsi_link: host_software_item::Model,
    pub agent: service::Model,
    pub execute_update_data: (host_software_item_plugin::Model, plugin_config::Model),
    pub detect_version_data: Option<(host_software_item_plugin::Model, plugin_config::Model)>,
}

/// Parameters for [`create_update_history_record`].
pub struct CreateUpdateRecordParams<'a> {
    pub host_id: Uuid,
    pub item_id: Uuid,
    pub to_version: &'a str,
    pub actor_type: &'a str,
    pub actor_id: &'a str,
    pub update_category: &'a str,
    /// Set when the update belongs to a batch.
    pub batch_id: Option<Uuid>,
}

/// Parameters for [`dispatch_update_to_agent`].
pub struct DispatchUpdateParams {
    pub update_history_id: Uuid,
    pub to_version: String,
    pub release_info: Option<ReleaseInfo>,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Load a role-specific plugin assignment for a host-software pair.
///
/// Returns `None` if no assignment with the given role exists.
async fn load_role_plugin(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    role: &str,
) -> Result<Option<(host_software_item_plugin::Model, plugin_config::Model)>> {
    let assignment = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .one(db)
        .await
        .context_to()?;

    let Some(assignment) = assignment else {
        return Ok(None);
    };

    let config = PluginConfig::find_by_id(assignment.plugin_config_id)
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::PluginConfigNotFound))?;

    Ok(Some((assignment, config)))
}

/// Build a [`PluginAssignment`] from a role plugin row and its config.
fn build_plugin_assignment(
    assignment: &host_software_item_plugin::Model,
    config: &plugin_config::Model,
) -> Result<PluginAssignment> {
    let plugin_type: uptrakit_internal_wire::PluginType =
        serde_json::from_value(serde_json::Value::String(config.plugin_type.clone()))
            .map_err(|_| TriggerUpdateError::UnknownPluginType(config.plugin_type.clone()))?;

    let merged_config =
        uptrakit_update_hooks::merge_config(&config.config, assignment.config_override.as_ref());

    Ok(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}

// ---------------------------------------------------------------------------
// Layer 1: Validate preconditions
// ---------------------------------------------------------------------------

/// Validates all preconditions for triggering an update on a single
/// (host, software_item) pair:
///
/// - Software item exists and is active.
/// - Host exists, is active, and belongs to the tenant.
/// - Host is assigned to the software item.
/// - An agent is linked to the host and is approved.
/// - No pending/in-progress update exists for this pair.
/// - The execute_update role plugin is assigned.
///
/// Returns a [`ValidatedUpdateTarget`] containing all loaded data needed for
/// the subsequent record creation and dispatch steps.
pub async fn validate_update_preconditions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    item_id: Uuid,
) -> Result<ValidatedUpdateTarget> {
    // 1. Verify software item exists and is active.
    let item = find_active_item(db, tenant_id, item_id)
        .await
        .ok_or_else(|| report!(TriggerUpdateError::SoftwareItemNotFound))?;

    // 2. Verify host exists, is active, and belongs to the tenant.
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotFound))?;

    // 3. Verify host is assigned to the software item.
    let hsi_link = HostSoftwareItem::find_by_id((host_id, item_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotAssigned))?;

    // 4. Find the agent linked to this host.
    let agent_link = ServiceHost::find()
        .filter(service_host::Column::HostId.eq(host_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::NoAgent))?;

    // 5. Verify agent exists and is approved.
    let agent = Service::find_by_id(agent_link.service_id)
        .filter(service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::NoAgent))?;

    if agent.status != service::ServiceStatus::Approved {
        bail!(TriggerUpdateError::AgentNotApproved);
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
        .await
        .context_to()?;

    if existing_update.is_some() {
        bail!(TriggerUpdateError::UpdateAlreadyActive);
    }

    // 7. Load role-specific plugin assignments.
    let execute_update_data = load_role_plugin(db, host_id, item_id, "execute_update")
        .await?
        .ok_or_else(|| report!(TriggerUpdateError::NoExecuteUpdatePlugin))?;

    let detect_version_data = load_role_plugin(db, host_id, item_id, "detect_version").await?;

    Ok(ValidatedUpdateTarget {
        item,
        host: host_record,
        hsi_link,
        agent,
        execute_update_data,
        detect_version_data,
    })
}

// ---------------------------------------------------------------------------
// Layer 2: Create update_history record
// ---------------------------------------------------------------------------

/// Inserts a Pending `update_history` row and returns its ID.
///
/// If `batch_id` is `Some`, the record is associated with a batch for
/// sequential per-host dispatch.
pub async fn create_update_history_record(
    db: &DatabaseConnection,
    params: &CreateUpdateRecordParams<'_>,
) -> Result<Uuid> {
    let now = OffsetDateTime::now_utc();
    let update_history_id = generate_uuid();
    let record = update_history::ActiveModel {
        id: Set(update_history_id),
        host_id: Set(params.host_id),
        software_item_id: Set(params.item_id),
        from_version: Set(None),
        to_version: Set(params.to_version.to_string()),
        status: Set(update_history::UpdateStatus::Pending),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set(params.actor_type.to_string()),
        actor_id: Set(params.actor_id.to_string()),
        started_at: Set(now),
        completed_at: Set(None),
        created_at: Set(now),
        update_category: Set(params.update_category.to_string()),
        batch_id: Set(params.batch_id),
    };

    record.insert(db).await.context_to()?;

    Ok(update_history_id)
}

// ---------------------------------------------------------------------------
// Layer 3: Dispatch to agent
// ---------------------------------------------------------------------------

/// Builds the `ExecuteUpdate` payload from the validated target and sends it
/// to the agent via `NotificationService`.
///
/// Returns `true` if the agent was locally connected at dispatch time.
pub async fn dispatch_update_to_agent(
    notifier: &NotificationService,
    target: &ValidatedUpdateTarget,
    params: DispatchUpdateParams,
) -> Result<bool> {
    let execute_update_plugin =
        build_plugin_assignment(&target.execute_update_data.0, &target.execute_update_data.1)?;

    let detect_version_plugin = target
        .detect_version_data
        .as_ref()
        .map(|(a, c)| build_plugin_assignment(a, c))
        .transpose()?;

    let resolved_hooks = uptrakit_update_hooks::resolve_hooks(
        &target.execute_update_data.1.config,
        target.execute_update_data.0.config_override.as_ref(),
    );

    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        host_machine_id: target.host.machine_id.clone(),
        update_history_id: params.update_history_id,
        software_item_id: target.item.id,
        software_item_name: target.item.name.clone(),
        to_version: params.to_version,
        detect_version_plugin,
        execute_update_plugin,
        pre_update_hooks: resolved_hooks.pre_update_hooks,
        post_update_hooks: resolved_hooks.post_update_hooks,
        release_info: params.release_info,
        timeout_seconds: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT_SECS,
    };

    let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
    let agent_connected = notifier.send(&target.agent.id, msg).await;

    if agent_connected {
        tracing::info!(
            update_id = %params.update_history_id,
            agent_id = %target.agent.id,
            host = %target.host.friendly_name,
            software = %target.item.name,
            "update sent to connected agent"
        );
    } else {
        tracing::info!(
            update_id = %params.update_history_id,
            agent_id = %target.agent.id,
            host = %target.host.friendly_name,
            software = %target.item.name,
            "agent not connected locally, update queued (outbox written for cross-controller delivery)"
        );
    }

    Ok(agent_connected)
}

// ---------------------------------------------------------------------------
// Convenience wrapper
// ---------------------------------------------------------------------------

/// Core update-trigger logic shared by the REST handler and the MQTT WS handler.
///
/// Validates preconditions, creates a Pending `update_history` record, and
/// dispatches the `ExecuteUpdate` message to the agent — all in one call.
///
/// For batch operations, call the three layers independently instead.
///
/// # Errors
///
/// Returns a [`TriggerUpdateError`] describing the first validation failure or
/// database error encountered.
pub async fn trigger_update_for_host(
    db: &DatabaseConnection,
    notifier: &NotificationService,
    params: TriggerUpdateParams<'_>,
) -> Result<TriggerUpdateResult> {
    let target = validate_update_preconditions(db, params.tenant_id, params.host_id, params.item_id)
        .await?;

    let update_history_id = create_update_history_record(
        db,
        &CreateUpdateRecordParams {
            host_id: params.host_id,
            item_id: params.item_id,
            to_version: &params.to_version,
            actor_type: params.actor_type,
            actor_id: params.actor_id,
            update_category: &target.hsi_link.update_category,
            batch_id: None,
        },
    )
    .await?;

    let agent_connected = dispatch_update_to_agent(
        notifier,
        &target,
        DispatchUpdateParams {
            update_history_id,
            to_version: params.to_version,
            release_info: params.release_info,
        },
    )
    .await?;

    Ok(TriggerUpdateResult {
        update_history_id,
        agent_connected,
    })
}
