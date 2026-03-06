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
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, Set,
};
use time::OffsetDateTime;
use uptrakit_internal_wire::{AttestationStatus, ControllerMessage, PluginAssignment, ReleaseInfo};
use uptrakit_shared_db::entity::{
    host, host_package_update_history, host_software_item, host_software_item_plugin,
    plugin_config, prelude::*, service, service_host, software_item, update_history,
};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use crate::notifier::ServiceNotifier;
use crate::queries::software_items::find_active_item;
use crate::queries::update_types::ActorType;
use crate::token_utils::generate_uuid;

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
    /// (host_id, software_item_id) pair. This is the belt-and-suspenders path
    /// when the partial unique DB index rejects a concurrent INSERT.
    #[error("an update is already pending or in progress")]
    UpdateAlreadyActive,
    /// Another update (software-item or host-package batch) is already running
    /// for this host. Wait for it to complete before triggering another.
    #[error("another update is already in progress for this host")]
    HostUpdateInProgress,
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
    /// Who initiated the update.
    pub actor_type: ActorType,
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
#[derive(Debug)]
pub struct ValidatedUpdateTarget {
    pub item: software_item::Model,
    pub host: host::Model,
    pub hsi_link: host_software_item::Model,
    pub agent: service::Model,
    pub execute_update_data: (host_software_item_plugin::Model, plugin_config::Model),
    pub detect_version_data: Option<(host_software_item_plugin::Model, plugin_config::Model)>,
    /// The merged config for the `fetch_releases` role plugin, if assigned.
    ///
    /// Used to extract `require_attestation` at dispatch time without a
    /// hard dependency on any specific plugin crate.
    pub fetch_releases_config: Option<serde_json::Value>,
}

/// Parameters for [`create_update_history_record`].
pub struct CreateUpdateRecordParams<'a> {
    pub host_id: Uuid,
    pub item_id: Uuid,
    pub to_version: &'a str,
    /// The currently installed version at the time the update was triggered.
    ///
    /// Populated from `host_software_items.installed_version` so the history
    /// record shows the "before" version even while the update is still
    /// pending or in progress.
    pub from_version: Option<String>,
    /// Who initiated the update.
    pub actor_type: ActorType,
    pub actor_id: &'a str,
    pub update_category: &'a str,
    /// Set when the update belongs to a batch.
    pub batch_id: Option<Uuid>,
    /// Initial status of the record. Non-batch callers always use
    /// [`update_history::UpdateStatus::Pending`]. Batch callers use
    /// [`update_history::UpdateStatus::Queued`] for non-first items on a
    /// host so that only one active record exists per host at a time.
    pub initial_status: update_history::UpdateStatus,
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

/// Loads all data needed for dispatch without performing the per-host lock check.
///
/// Used by [`dispatch_next_in_batch`](super::update_batches::dispatch_next_in_batch)
/// which performs the CAS transition instead, and by
/// [`validate_update_preconditions`] which calls this and then adds the lock check.
///
/// Steps:
/// 1. Verify software item exists and is active.
/// 2. Verify host exists, is active, and belongs to the tenant.
/// 3. Verify host is assigned to the software item.
/// 4. Find the agent linked to this host.
/// 5. Verify agent exists, belongs to the tenant, and is approved.
/// 6. Load role-specific plugin assignments (`execute_update`, `detect_version`,
///    `fetch_releases`).
pub(crate) async fn load_target_for_dispatch(
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

    // 5. Verify agent exists, belongs to the tenant, and is approved.
    let agent = Service::find_by_id(agent_link.service_id)
        .filter(service::Column::TenantId.eq(tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::NoAgent))?;

    if agent.status != service::ServiceStatus::Approved {
        bail!(TriggerUpdateError::AgentNotApproved);
    }

    // 6. Load role-specific plugin assignments.
    let execute_update_data = load_role_plugin(db, host_id, item_id, "execute_update")
        .await?
        .ok_or_else(|| report!(TriggerUpdateError::NoExecuteUpdatePlugin))?;

    let detect_version_data = load_role_plugin(db, host_id, item_id, "detect_version").await?;

    // Load fetch_releases plugin config (optional). Used at dispatch time to
    // extract `require_attestation` from the GitHub plugin config.
    let fetch_releases_config = load_role_plugin(db, host_id, item_id, "fetch_releases")
        .await?
        .map(|(assignment, config)| {
            uptrakit_update_hooks::merge_config(&config.config, assignment.config_override.as_ref())
        });

    Ok(ValidatedUpdateTarget {
        item,
        host: host_record,
        hsi_link,
        agent,
        execute_update_data,
        detect_version_data,
        fetch_releases_config,
    })
}

/// Validates all preconditions for triggering an update on a single
/// (host, software_item) pair:
///
/// - Software item exists and is active.
/// - Host exists, is active, and belongs to the tenant.
/// - Host is assigned to the software item.
/// - An agent is linked to the host and is approved.
/// - **No pending/in-progress update exists for this host** (across all update
///   types: software items and host-package batches).
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
    // Steps 1–5 and 7 (data loading, no lock check).
    let target = load_target_for_dispatch(db, tenant_id, host_id, item_id).await?;

    // 6. Combined per-host lock check: reject if any active update exists for
    //    this host in either `update_history` (software items) OR
    //    `host_package_update_history` (host package batches).
    //    This enforces the invariant: at most one update runs per host at a time.
    let sw_active = UpdateHistory::find()
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Pending,
            update_history::UpdateStatus::InProgress,
        ]))
        .count(db)
        .await
        .context_to()?;

    if sw_active > 0 {
        bail!(TriggerUpdateError::HostUpdateInProgress);
    }

    let pkg_active = HostPackageUpdateHistory::find()
        .filter(host_package_update_history::Column::HostId.eq(host_id))
        .filter(
            Condition::any()
                .add(host_package_update_history::Column::Status.eq("pending"))
                .add(host_package_update_history::Column::Status.eq("in_progress")),
        )
        .count(db)
        .await
        .context_to()?;

    if pkg_active > 0 {
        bail!(TriggerUpdateError::HostUpdateInProgress);
    }

    Ok(target)
}

// ---------------------------------------------------------------------------
// Layer 2: Create update_history record
// ---------------------------------------------------------------------------

/// Inserts an `update_history` row with the given `initial_status` and returns its ID.
///
/// If `batch_id` is `Some`, the record is associated with a batch for
/// sequential per-host dispatch.
///
/// Non-batch callers pass `initial_status: UpdateStatus::Pending`. Batch
/// callers pass `UpdateStatus::Queued` for non-first items on a host.
///
/// Accepts any `ConnectionTrait` implementor (bare `DatabaseConnection` or a
/// SeaORM transaction) so callers can run this inside or outside a transaction.
pub async fn create_update_history_record<C: ConnectionTrait>(
    db: &C,
    params: &CreateUpdateRecordParams<'_>,
) -> Result<Uuid> {
    let now = OffsetDateTime::now_utc();
    let update_history_id = generate_uuid();
    let record = update_history::ActiveModel {
        id: Set(update_history_id),
        host_id: Set(params.host_id),
        software_item_id: Set(params.item_id),
        from_version: Set(params.from_version.clone()),
        to_version: Set(params.to_version.to_string()),
        status: Set(params.initial_status),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set(params.actor_type.as_str().to_string()),
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
    notifier: &dyn ServiceNotifier,
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

    let enriched_release_info = enrich_release_info_with_attestation(
        params.release_info,
        target.hsi_link.latest_release_metadata.as_ref(),
        target.fetch_releases_config.as_ref(),
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
        release_info: enriched_release_info,
        timeout_seconds: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT_SECS,
    };

    let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
    let agent_connected = notifier.send_to_service(&target.agent.id, msg).await;

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
    notifier: &dyn ServiceNotifier,
    params: TriggerUpdateParams<'_>,
) -> Result<TriggerUpdateResult> {
    let target =
        validate_update_preconditions(db, params.tenant_id, params.host_id, params.item_id).await?;

    let update_history_id = create_update_history_record(
        db,
        &CreateUpdateRecordParams {
            host_id: params.host_id,
            item_id: params.item_id,
            to_version: &params.to_version,
            from_version: target.hsi_link.installed_version.clone(),
            actor_type: params.actor_type,
            actor_id: params.actor_id,
            update_category: &target.hsi_link.update_category,
            batch_id: None,
            initial_status: update_history::UpdateStatus::Pending,
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

// ---------------------------------------------------------------------------
// Attestation enrichment
// ---------------------------------------------------------------------------

/// Minimal projection of `latest_release_metadata` used to extract attestation
/// fields without depending on the full `UpstreamRelease` type.
#[derive(serde::Deserialize)]
struct MetadataAttestation {
    #[serde(default)]
    attestation_status: Option<AttestationStatus>,
    #[serde(default)]
    assets: Vec<MetadataAsset>,
}

#[derive(serde::Deserialize)]
struct MetadataAsset {
    name: String,
    #[serde(default)]
    sha256_digest: Option<String>,
}

/// Enrich a `ReleaseInfo` with attestation data sourced from the DB.
///
/// - `attestation_status` and per-asset `sha256_digest` are read from
///   `latest_release_metadata` (the JSON blob stored after `fetch_releases`).
/// - `require_attestation` is extracted from the `fetch_releases` plugin
///   config as a generic `{require_attestation: bool}` field so that this
///   crate does not need to depend on any specific plugin crate.
///
/// If `release_info` is `None`, returns `None` unchanged.
fn enrich_release_info_with_attestation(
    release_info: Option<ReleaseInfo>,
    metadata: Option<&serde_json::Value>,
    fetch_config: Option<&serde_json::Value>,
) -> Option<ReleaseInfo> {
    let mut ri = release_info?;

    // Apply attestation_status and sha256_digest from the last fetch_releases run.
    if let Some(meta) = metadata
        && let Ok(meta_ri) = serde_json::from_value::<MetadataAttestation>(meta.clone())
    {
        ri.attestation_status = meta_ri.attestation_status;
        for asset in &mut ri.assets {
            if let Some(ma) = meta_ri.assets.iter().find(|a| a.name == asset.name) {
                asset.sha256_digest = ma.sha256_digest.clone();
            }
        }
    }

    // Apply require_attestation from the fetch_releases plugin config.
    if let Some(config) = fetch_config {
        #[derive(serde::Deserialize, Default)]
        struct RequireAttestation {
            #[serde(default)]
            require_attestation: bool,
        }
        if let Ok(parsed) = serde_json::from_value::<RequireAttestation>(config.clone()) {
            ri.require_attestation = parsed.require_attestation;
        }
    }

    Some(ri)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, ModelTrait,
        QueryFilter, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{
        host, host_package, host_package_update_history, host_software_item,
        host_software_item_plugin, plugin_config, service, service_host, software_item, tenant,
        update_history,
    };
    use uptrakit_shared_types::ServiceStatus;
    use uuid::Uuid;

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    struct Fixture {
        tenant_id: Uuid,
        item_id: Uuid,
        host_id: Uuid,
        service_id: Uuid,
        plugin_config_id: Uuid,
    }

    async fn insert_base_fixture(db: &DatabaseConnection) -> Fixture {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let service_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();

        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(format!("test-{tenant_id}")),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-app".to_string()),
            enabled: Set(true),
            discovery_state: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set("machine-001".to_string()),
            hostname: Set("host-001".to_string()),
            friendly_name: Set("Host 001".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("agent-host".to_string()),
            friendly_name: Set("Agent 001".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item::ActiveModel {
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            latest_version: Set(Some("1.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("feature".to_string()),
        }
        .insert(db)
        .await
        .unwrap();

        plugin_config::ActiveModel {
            id: Set(plugin_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-plugin".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            plugin_config_id: Set(plugin_config_id),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo".to_string()),
            config_override: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        Fixture {
            tenant_id,
            item_id,
            host_id,
            service_id,
            plugin_config_id,
        }
    }

    // ── validate_update_preconditions ───────────────────────────────────

    #[tokio::test]
    async fn validate_preconditions_success() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        let target = result.unwrap();
        assert_eq!(target.item.id, f.item_id);
        assert_eq!(target.host.id, f.host_id);
        assert_eq!(target.agent.id, f.service_id);
    }

    #[tokio::test]
    async fn validate_preconditions_item_not_found() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let missing_id = Uuid::now_v7();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, missing_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::SoftwareItemNotFound
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_item_deactivated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item = SoftwareItem::find_by_id(f.item_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: software_item::ActiveModel = item.into();
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::SoftwareItemNotFound
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_host_wrong_tenant() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        // Create a second tenant and a host belonging to it.
        let other_tenant_id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(other_tenant_id),
            name: Set("other-tenant".to_string()),
            slug: Set(format!("other-{other_tenant_id}")),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        let other_host_id = Uuid::now_v7();
        host::ActiveModel {
            id: Set(other_host_id),
            tenant_id: Set(other_tenant_id),
            machine_id: Set("machine-other".to_string()),
            hostname: Set("host-other".to_string()),
            friendly_name: Set("Other Host".to_string()),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        // Query with f.tenant_id — the other host belongs to other_tenant_id, so it
        // passes the software item check but fails the host tenant check.
        let result =
            validate_update_preconditions(&db, f.tenant_id, other_host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostNotFound
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_host_not_assigned() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let hsi = HostSoftwareItem::find_by_id((f.host_id, f.item_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        hsi.delete(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostNotAssigned
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_no_service_host() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let sh = ServiceHost::find()
            .filter(service_host::Column::HostId.eq(f.host_id))
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        sh.delete(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::NoAgent
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_agent_not_approved() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let svc = Service::find_by_id(f.service_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: service::ActiveModel = svc.into();
        active.status = Set(ServiceStatus::Pending);
        active.update(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::AgentNotApproved
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_update_already_active_same_item() {
        // A Pending update for the same (host, item) pair must be rejected.
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(f.host_id),
            software_item_id: Set(f.item_id),
            from_version: Set(None),
            to_version: Set("1.1.0".to_string()),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            started_at: Set(now),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("feature".to_string()),
            batch_id: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostUpdateInProgress
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_host_update_in_progress_different_item() {
        // A Pending update for a DIFFERENT item on the same host must also be rejected.
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // Insert a second software item (not assigned to the host — that doesn't
        // matter for the lock check which is host-scoped).
        let other_item_id = Uuid::now_v7();
        uptrakit_shared_db::entity::software_item::ActiveModel {
            id: Set(other_item_id),
            tenant_id: Set(f.tenant_id),
            name: Set("other-app".to_string()),
            enabled: Set(true),
            discovery_state: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        // Insert a Pending update_history row for the OTHER item on the same host.
        update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(f.host_id),
            software_item_id: Set(other_item_id),
            from_version: Set(None),
            to_version: Set("2.0.0".to_string()),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            started_at: Set(now),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("feature".to_string()),
            batch_id: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        // Triggering an update for f.item_id on the same host must be rejected.
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostUpdateInProgress
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_host_package_update_blocks_sw_item() {
        // An active host-package batch update must block a software-item update.
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let now = OffsetDateTime::now_utc();

        // Insert a host_package row (required by FK on host_package_update_history).
        let pkg_id = Uuid::now_v7();
        host_package::ActiveModel {
            id: Set(pkg_id),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            plugin_config_id: Set(f.plugin_config_id),
            package_identifier: Set("curl".to_string()),
            name: Set("curl".to_string()),
            installed_version: Set(Some("7.0.0".to_string())),
            installed_version_detected_at: Set(None),
            latest_version: Set(Some("8.0.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            update_category: Set("security".to_string()),
            enabled: Set(true),
            last_checked_at: Set(None),
            last_updated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        // Insert a pending host_package_update_history row for the same host.
        host_package_update_history::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(f.tenant_id),
            host_id: Set(f.host_id),
            host_package_id: Set(pkg_id),
            from_version: Set(Some("7.0.0".to_string())),
            to_version: Set(Some("8.0.0".to_string())),
            status: Set("pending".to_string()),
            output: Set(None),
            output_bytes: Set(0),
            actor_type: Set("mqtt".to_string()),
            actor_id: Set(String::new()),
            update_category: Set("security".to_string()),
            started_at: Set(None),
            completed_at: Set(None),
            created_at: Set(now),
            batch_id: Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::HostUpdateInProgress
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_no_execute_plugin() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        HostSoftwareItemPlugin::delete_many()
            .filter(host_software_item_plugin::Column::HostId.eq(f.host_id))
            .filter(host_software_item_plugin::Column::Role.eq("execute_update"))
            .exec(&db)
            .await
            .unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::NoExecuteUpdatePlugin
        ));
    }

    #[tokio::test]
    async fn validate_preconditions_plugin_config_deactivated() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let pc = PluginConfig::find_by_id(f.plugin_config_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        let mut active: plugin_config::ActiveModel = pc.into();
        active.deactivated_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&db).await.unwrap();
        let result = validate_update_preconditions(&db, f.tenant_id, f.host_id, f.item_id).await;
        assert!(matches!(
            result.unwrap_err().current_context(),
            TriggerUpdateError::PluginConfigNotFound
        ));
    }

    // ── enrich_release_info_with_attestation ────────────────────────────

    fn make_release_info() -> ReleaseInfo {
        ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://github.com/owner/repo/releases/tag/v1.0.0".to_string(),
            assets: vec![uptrakit_internal_wire::ReleaseAsset {
                name: "app-amd64.tar.gz".to_string(),
                download_url:
                    "https://github.com/owner/repo/releases/download/v1.0.0/app-amd64.tar.gz"
                        .to_string(),
                size: Some(1024),
                content_type: None,
                sha256_digest: None,
            }],
            attestation_status: None,
            require_attestation: false,
        }
    }

    #[test]
    fn enrich_release_info_none_returns_none() {
        let result = enrich_release_info_with_attestation(None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn enrich_release_info_no_metadata_leaves_unchanged() {
        let ri = make_release_info();
        let result = enrich_release_info_with_attestation(Some(ri), None, None).unwrap();
        assert!(result.attestation_status.is_none());
        assert!(!result.require_attestation);
        assert!(result.assets[0].sha256_digest.is_none());
    }

    #[test]
    fn enrich_release_info_sets_attestation_status_and_digest() {
        let ri = make_release_info();
        let meta = serde_json::json!({
            "attestation_status": "Verified",
            "assets": [
                { "name": "app-amd64.tar.gz", "sha256_digest": "a".repeat(64) }
            ]
        });
        let result = enrich_release_info_with_attestation(Some(ri), Some(&meta), None).unwrap();
        assert_eq!(
            result.attestation_status,
            Some(uptrakit_internal_wire::AttestationStatus::Verified)
        );
        assert_eq!(result.assets[0].sha256_digest, Some("a".repeat(64)));
    }

    #[test]
    fn enrich_release_info_sets_require_attestation_from_config() {
        let ri = make_release_info();
        let config = serde_json::json!({ "require_attestation": true });
        let result = enrich_release_info_with_attestation(Some(ri), None, Some(&config)).unwrap();
        assert!(result.require_attestation);
    }

    #[test]
    fn enrich_release_info_asset_name_mismatch_leaves_digest_none() {
        let ri = make_release_info();
        let meta = serde_json::json!({
            "attestation_status": "NotFound",
            "assets": [
                { "name": "other-asset.tar.gz", "sha256_digest": "b".repeat(64) }
            ]
        });
        let result = enrich_release_info_with_attestation(Some(ri), Some(&meta), None).unwrap();
        assert_eq!(
            result.attestation_status,
            Some(uptrakit_internal_wire::AttestationStatus::NotFound)
        );
        assert!(result.assets[0].sha256_digest.is_none());
    }
}
