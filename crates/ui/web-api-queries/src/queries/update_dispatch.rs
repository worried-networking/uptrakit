//! Shared update-dispatch primitives used by both `update_triggers` and
//! `update_batches`.
//!
//! This module contains the types, error definitions, and core functions that
//! form the composable building blocks of the update pipeline. By housing them
//! in a neutral module, `update_triggers` and `update_batches` can each depend
//! on `update_dispatch` without creating a circular dependency between each
//! other.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, RelationTrait, Set,
};
use time::OffsetDateTime;
use uptrakit_internal_wire::{
    AttestationStatus, ControllerMessage, PluginAssignment, ReleaseAsset, ReleaseInfo,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
    service_host, software_item, update_history,
};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use crate::notifier::ServiceNotifier;
use crate::queries::software_items::find_active_item;
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
    pub execute_update_data: (
        host_software_item_plugin::Model,
        Option<plugin_config::Model>,
    ),
    pub detect_version_data: Option<(
        host_software_item_plugin::Model,
        Option<plugin_config::Model>,
    )>,
    /// The merged config for the `fetch_releases` role plugin, if assigned.
    ///
    /// Used to extract `require_attestation` at dispatch time without a
    /// hard dependency on any specific plugin crate.
    pub fetch_releases_config: Option<serde_json::Value>,
    /// Pre-update hook plugin assignments, ordered by `ordinal`.
    pub pre_update_hook_plugins: Vec<PluginAssignment>,
    /// Post-update hook plugin assignments, ordered by `ordinal`.
    pub post_update_hook_plugins: Vec<PluginAssignment>,
}

/// Parameters for [`create_update_history_record`].
pub struct CreateUpdateRecordParams<'a> {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub item_id: Uuid,
    pub host_software_item_id: Option<Uuid>,
    pub to_version: &'a str,
    /// The currently installed version at the time the update was triggered.
    ///
    /// Populated from `host_software_items.installed_version` so the history
    /// record shows the "before" version even while the update is still
    /// pending or in progress.
    pub from_version: Option<String>,
    /// Who initiated the update.
    pub actor_type: &'a str,
    pub actor_id: &'a str,
    pub update_category: &'a str,
    /// Set when the update belongs to a batch.
    pub batch_id: Option<Uuid>,
    /// Initial status of the record. Non-batch callers always use
    /// [`update_history::UpdateStatus::Pending`]. Batch callers use
    /// [`update_history::UpdateStatus::Queued`] for non-first items on a
    /// host so that only one active record exists per host at a time.
    pub initial_status: update_history::UpdateStatus,
    /// Whether the update is dispatched in interactive mode (PTY allocated).
    ///
    /// Must reflect the fully-resolved value (i.e. `params.interactive ||
    /// config_prefers_interactive(...)`) so the persisted column accurately
    /// records how the agent was instructed to run.
    pub interactive: bool,
}

/// Parameters for [`dispatch_update_to_agent`].
pub struct DispatchUpdateParams {
    pub update_history_id: Uuid,
    pub to_version: String,
    pub release_info: Option<ReleaseInfo>,
    /// When true, the agent allocates a PTY and keeps stdin open.
    pub interactive: bool,
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
) -> Result<
    Option<(
        host_software_item_plugin::Model,
        Option<plugin_config::Model>,
    )>,
> {
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

    let config = if let Some(pc_id) = assignment.plugin_config_id {
        Some(
            PluginConfig::find_by_id(pc_id)
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?
                .ok_or_else(|| report!(TriggerUpdateError::PluginConfigNotFound))?,
        )
    } else {
        None
    };

    Ok(Some((assignment, config)))
}

/// Load all plugin assignments for a given role, ordered by `ordinal ASC`.
///
/// Used for hook roles (`pre_update_hook`, `post_update_hook`) where multiple
/// plugins can be assigned with ordering semantics.
async fn load_role_plugins_ordered(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    role: &str,
) -> Result<Vec<PluginAssignment>> {
    let assignments = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .order_by_asc(host_software_item_plugin::Column::Ordinal)
        .all(db)
        .await
        .context_to()?;

    let mut result = Vec::with_capacity(assignments.len());
    for assignment in &assignments {
        let config = if let Some(pc_id) = assignment.plugin_config_id {
            PluginConfig::find_by_id(pc_id)
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?
        } else {
            None
        };
        result.push(build_plugin_assignment(assignment, config.as_ref())?);
    }

    Ok(result)
}

/// Build a [`PluginAssignment`] from a role plugin row and its optional config.
///
/// When `config` is `None` (no `plugin_config` linked to the assignment), the
/// plugin type is read from `assignment.plugin_type` and the effective config
/// is built from the assignment-level override alone.
pub(crate) fn build_plugin_assignment(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> Result<PluginAssignment> {
    let plugin_type = uptrakit_internal_wire::PluginTypeId::new(&assignment.plugin_type);

    let merged_config = uptrakit_config_merge::resolve_effective_config(
        None,
        config.map(|c| &c.config),
        assignment.config.as_ref(),
    );

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
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
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
    let hsi_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotAssigned))?;

    // 4. Find services linked to this host (tenant-scoped via join on service).
    let tenant_db_local = crate::TenantDb::new(db.clone(), tenant_id);
    let agent_links = tenant_db_local
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|link| link.service_id)
        .collect::<Vec<_>>();

    if agent_links.is_empty() {
        bail!(TriggerUpdateError::NoAgent);
    }

    // 5. Prefer a non-deactivated approved service when multiple historical
    // links exist for the same host (for example after re-enrolling agent-ssh).
    let agents = Service::find()
        .filter(service::Column::Id.is_in(agent_links))
        .filter(service::Column::TenantId.eq(tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

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
        })
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
            uptrakit_config_merge::resolve_effective_config(
                None,
                config.as_ref().map(|c| &c.config),
                assignment.config.as_ref(),
            )
        });

    // Load hook plugin assignments (ordered by ordinal).
    let pre_update_hook_plugins =
        load_role_plugins_ordered(db, host_id, item_id, "pre_update_hook").await?;
    let post_update_hook_plugins =
        load_role_plugins_ordered(db, host_id, item_id, "post_update_hook").await?;

    Ok(ValidatedUpdateTarget {
        item,
        host: host_record,
        hsi_link,
        agent,
        execute_update_data,
        detect_version_data,
        fetch_releases_config,
        pre_update_hook_plugins,
        post_update_hook_plugins,
    })
}

/// Validates all preconditions for triggering an update on a single
/// (host, software_item) pair:
///
/// - Software item exists and is active.
/// - Host exists, is active, and belongs to the tenant.
/// - Host is assigned to the software item.
/// - An agent is linked to the host and is approved.
/// - The execute_update role plugin is assigned.
///
/// Returns a [`ValidatedUpdateTarget`] containing all loaded data needed for
/// the subsequent record creation and dispatch steps.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn validate_update_preconditions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    item_id: Uuid,
) -> Result<ValidatedUpdateTarget> {
    load_target_for_dispatch(db, tenant_id, host_id, item_id).await
}

/// Returns `true` if a `Pending` or `InProgress` update already exists for
/// the given host (across all software items and batches).
pub async fn has_active_update_for_host(db: &DatabaseConnection, host_id: Uuid) -> Result<bool> {
    let count = UpdateHistory::find()
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Pending,
            update_history::UpdateStatus::InProgress,
        ]))
        .count(db)
        .await
        .context_to()?;
    Ok(count > 0)
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
#[tracing::instrument(skip_all)]
pub async fn create_update_history_record<C: ConnectionTrait>(
    db: &C,
    params: &CreateUpdateRecordParams<'_>,
) -> Result<Uuid> {
    let now = OffsetDateTime::now_utc();
    let update_history_id = generate_uuid();
    let record = update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(params.tenant_id),
        host_id: Set(params.host_id),
        software_item_id: Set(params.item_id),
        host_software_item_id: Set(params.host_software_item_id),
        from_version: Set(params.from_version.clone()),
        to_version: Set(Some(params.to_version.to_string())),
        status: Set(params.initial_status),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set(params.actor_type.to_string()),
        actor_id: Set(params.actor_id.to_string()),
        started_at: Set(Some(now)),
        completed_at: Set(None),
        created_at: Set(now),
        update_category: Set(params.update_category.to_string()),
        batch_id: Set(params.batch_id),
        interactive: Set(params.interactive),
        output_truncated: Set(false),
    };

    record.insert(db).await.context_to()?;

    Ok(update_history_id)
}

// ---------------------------------------------------------------------------
// Layer 3: Dispatch to agent
// ---------------------------------------------------------------------------

/// Returns `true` when the execute-update plugin config opts into interactive
/// PTY mode via the `"prefer_interactive": true` field.
///
/// Currently only meaningful for `GenericShell` configs (where the field is
/// defined on `ShellConfig`), but the check is intentionally kept generic —
/// any plugin type that stores `"prefer_interactive": true` in its JSON config
/// will trigger interactive dispatch.
pub(crate) fn config_prefers_interactive(
    plugin_type: &uptrakit_internal_wire::PluginTypeId,
    config: &serde_json::Value,
) -> bool {
    plugin_type == uptrakit_shared_types::plugin_ids::GENERIC_SHELL.as_str()
        && config
            .get("prefer_interactive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

/// Builds the `ExecuteUpdate` payload from the validated target and sends it
/// to the agent via `NotificationService`.
///
/// The `interactive` flag in the payload is set to `true` when either:
/// - The caller explicitly requests it via [`DispatchUpdateParams::interactive`], or
/// - The execute-update plugin config carries `"prefer_interactive": true`
///   (set automatically for Proxmox Helper Scripts targets so that the PHS
///   `/usr/bin/update` script can read from `/dev/tty` during updates).
///
/// Returns `true` if the agent was locally connected at dispatch time.
#[tracing::instrument(skip_all)]
pub async fn dispatch_update_to_agent(
    notifier: &dyn ServiceNotifier,
    target: &ValidatedUpdateTarget,
    params: DispatchUpdateParams,
) -> Result<bool> {
    let execute_update_plugin = build_plugin_assignment(
        &target.execute_update_data.0,
        target.execute_update_data.1.as_ref(),
    )?;

    let detect_version_plugin = target
        .detect_version_data
        .as_ref()
        .map(|(a, c)| build_plugin_assignment(a, c.as_ref()))
        .transpose()?;

    let enriched_release_info = enrich_release_info_with_attestation(
        params.release_info,
        target.hsi_link.latest_release_metadata.as_ref(),
        target.fetch_releases_config.as_ref(),
    );

    // Auto-enable interactive mode when the plugin config opts in, regardless
    // of whether the caller explicitly requested it.
    let interactive = params.interactive
        || config_prefers_interactive(
            &execute_update_plugin.plugin_type,
            &execute_update_plugin.config,
        );

    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        host_machine_id: target.host.machine_id.clone(),
        update_history_id: params.update_history_id,
        software_item_id: target.item.id,
        software_item_name: target.item.name.clone(),
        to_version: params.to_version,
        detect_version_plugin,
        execute_update_plugin,
        pre_update_hook_plugins: target.pre_update_hook_plugins.clone(),
        post_update_hook_plugins: target.post_update_hook_plugins.clone(),
        release_info: enriched_release_info,
        timeout: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT,
        interactive,
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
// Attestation enrichment
// ---------------------------------------------------------------------------

/// Projection of `latest_release_metadata` covering all fields needed for
/// both reconstruction and attestation enrichment.
///
/// Matches the shape of `UpstreamRelease` (and indirectly `ReleaseInfo`) as
/// stored by `FetchReleasesExecutor` after a controller-side `fetch_releases` run.
#[derive(serde::Deserialize)]
struct MetadataRelease {
    #[serde(default)]
    tag: String,
    #[serde(default)]
    release_url: String,
    #[serde(default)]
    attestation_status: Option<AttestationStatus>,
    #[serde(default)]
    assets: Vec<MetadataAsset>,
}

#[derive(serde::Deserialize)]
struct MetadataAsset {
    name: String,
    /// Required for reconstructing a complete `ReleaseAsset` when `release_info`
    /// is absent. Empty string indicates the field was not present in older metadata.
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    sha256_digest: Option<String>,
}

/// Build or enrich a `ReleaseInfo` using data sourced from the DB.
///
/// **Reconstruction path** (`release_info = None`): when the caller does not
/// supply `release_info` (e.g. the frontend omits it, or a reconnect replay
/// occurs), the function reconstructs `ReleaseInfo` from
/// `latest_release_metadata` stored after the last `fetch_releases` run. This
/// makes GitHub-asset-based updates work even when the client sends only
/// `to_version`.
///
/// **Enrichment path** (`release_info = Some`): when the caller supplies
/// `release_info` (e.g. CLI `--release-tag`), the function enriches it with
/// `attestation_status` and per-asset `sha256_digest` from `metadata`.
///
/// In both cases, `require_attestation` is extracted from the `fetch_releases`
/// plugin config JSON so that this crate does not need to depend on any
/// specific plugin crate.
///
/// Returns `None` only when both `release_info` is `None` and metadata cannot
/// be deserialized into a usable `ReleaseInfo` (e.g. `tag` is absent).
pub fn enrich_release_info_with_attestation(
    release_info: Option<ReleaseInfo>,
    metadata: Option<&serde_json::Value>,
    fetch_config: Option<&serde_json::Value>,
) -> Option<ReleaseInfo> {
    let mut ri = match release_info {
        Some(mut ri) => {
            // Enrichment path: apply attestation_status and sha256_digest from
            // the last fetch_releases run.
            if let Some(meta) = metadata
                && let Ok(meta_ri) = serde_json::from_value::<MetadataRelease>(meta.clone())
            {
                ri.attestation_status = meta_ri.attestation_status;
                for asset in &mut ri.assets {
                    if let Some(ma) = meta_ri.assets.iter().find(|a| a.name == asset.name) {
                        asset.sha256_digest = ma.sha256_digest.clone();
                    }
                }
            }
            ri
        }
        None => {
            // Reconstruction path: build ReleaseInfo entirely from the stored
            // metadata when the trigger request did not include release_info.
            let meta = metadata?;
            let meta_ri = serde_json::from_value::<MetadataRelease>(meta.clone()).ok()?;
            if meta_ri.tag.is_empty() {
                return None;
            }
            ReleaseInfo {
                tag: meta_ri.tag,
                release_url: meta_ri.release_url,
                assets: meta_ri
                    .assets
                    .into_iter()
                    .filter(|a| !a.download_url.is_empty())
                    .map(|a| ReleaseAsset {
                        name: a.name,
                        download_url: a.download_url,
                        size: a.size,
                        content_type: a.content_type,
                        sha256_digest: a.sha256_digest,
                    })
                    .collect(),
                attestation_status: meta_ri.attestation_status,
                require_attestation: false,
            }
        }
    };

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
