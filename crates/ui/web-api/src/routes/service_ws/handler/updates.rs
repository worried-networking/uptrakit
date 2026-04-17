//! Update delivery, ownership validation, and update-lifecycle message handlers.
//!
//! Contains host-link visibility checks, reconnect recovery, pending replay preparation,
//! and the per-message handlers
//! `handle_update_started`, `handle_update_output`, and `handle_update_result`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use time::OffsetDateTime;

use super::shared_types::{ProcessorResponse, load_linked_host_ids};
use super::{HandlerError, HandlerResult, MAX_UPDATE_OUTPUT_BYTES};
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use rootcause::prelude::*;
use uptrakit_internal_wire::{
    BatchUpdateResultPayload, ControllerMessage, ExecuteUpdatePayload, PluginAssignment,
    UpdateFinalStatus, UpdateOutputPayload, UpdateResultPayload, UpdateStartedPayload,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, software_item,
    update_history, update_output_line,
};
use uptrakit_web_api_types::events::AdminEvent;

const RECOVERY_FINALIZATION_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
pub(super) enum ReconnectSuccessorDispatchMode {
    Immediate,
    ReplayPrepared,
}

struct ReplayPreparationNotifier;

#[async_trait::async_trait]
impl crate::ServiceNotifier for ReplayPreparationNotifier {
    async fn send_to_service(&self, _service_id: &uuid::Uuid, _msg: ControllerMessage) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// validate_host_link_visibility
// ---------------------------------------------------------------------------

/// Validate that an `update_history` record belongs to a host linked to the
/// current service. Returns the record on success, logs a warning and returns
/// an error if the service does not own the record.
#[tracing::instrument(skip_all, fields(%service_id, %update_history_id))]
pub(super) async fn validate_host_link_visibility(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    update_history_id: uuid::Uuid,
    linked_host_ids: impl std::borrow::Borrow<HashSet<uuid::Uuid>>,
) -> HandlerResult<update_history::Model> {
    let linked_host_ids = linked_host_ids.borrow();
    let record = uptrakit_shared_db::entity::prelude::UpdateHistory::find_by_id(update_history_id)
        .one(db)
        .await
        .context_to::<HandlerError>()?
        .ok_or_else(|| {
            tracing::warn!(
                %service_id,
                update_id = %update_history_id,
                "update_history record not found"
            );
            report!(HandlerError::WebSocketSend)
        })?;

    if !linked_host_ids.contains(&record.host_id) {
        tracing::warn!(
            %service_id,
            update_id = %update_history_id,
            host_id = %record.host_id,
            "service attempted to update record for unlinked host"
        );
        bail!(HandlerError::WebSocketSend);
    }

    Ok(record)
}

async fn finalize_post_update_best_effort(state: &Arc<AppState>, record: &update_history::Model) {
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update(
        state.db(),
        state.controller_update_protection(),
        record,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update finalization failed"
        );
    }
}

async fn finalize_post_update_with_recovery_timeout_best_effort(
    state: &Arc<AppState>,
    record: &update_history::Model,
) {
    if let Err(error) = crate::queries::update_dispatch::finalize_post_update_with_timeout(
        state.db(),
        state.controller_update_protection(),
        record,
        RECOVERY_FINALIZATION_TIMEOUT,
    )
    .await
    {
        tracing::warn!(
            error = %error,
            update_id = %record.id,
            "post-update finalization failed during reconnect recovery"
        );
    }
}

// ---------------------------------------------------------------------------
// pending replay preparation helpers
// ---------------------------------------------------------------------------

/// All data loaded from the DB that is needed to dispatch pending updates for
/// a set of hosts.
pub(super) struct PendingUpdateRecords {
    pending_updates: Vec<update_history::Model>,
    sw_items_map: HashMap<uuid::Uuid, software_item::Model>,
    hosts_map: HashMap<uuid::Uuid, host::Model>,
    assignments_map: HashMap<(uuid::Uuid, uuid::Uuid, String), host_software_item_plugin::Model>,
    hook_assignments_map:
        HashMap<(uuid::Uuid, uuid::Uuid, String), Vec<host_software_item_plugin::Model>>,
    configs_map: HashMap<uuid::Uuid, plugin_config::Model>,
    hsi_metadata_map: HashMap<(uuid::Uuid, uuid::Uuid), Option<serde_json::Value>>,
}

/// Load all pending update records and their auxiliary data for hosts linked to
/// `service_id`.
///
/// Returns `None` when there are no host links or no pending records (nothing
/// to dispatch).
pub(super) async fn load_pending_update_records(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
) -> HandlerResult<Option<(Vec<uuid::Uuid>, PendingUpdateRecords)>> {
    // 1. Find host_ids linked to this service.
    let host_ids: Vec<uuid::Uuid> = load_linked_host_ids(state.db(), service_id)
        .await?
        .into_iter()
        .collect();

    if host_ids.is_empty() {
        return Ok(None);
    }

    // 2. Query pending update_history records for those hosts.
    //    Ordered by ID (UUIDv7 = chronological) so batch-aware filtering
    //    below picks the oldest pending update per (batch_id, host_id).
    let pending_updates = update_history::Entity::find()
        .filter(update_history::Column::HostId.is_in(host_ids.clone()))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .order_by_asc(update_history::Column::Id)
        .all(state.db())
        .await
        .context_to::<HandlerError>()?;

    if pending_updates.is_empty() {
        return Ok(None);
    }

    // Collect unique IDs needed for batch queries.
    let sw_ids: Vec<uuid::Uuid> = pending_updates
        .iter()
        .map(|u| u.software_item_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    // Batch 1: software items.
    let sw_items_map: HashMap<uuid::Uuid, software_item::Model> = software_item::Entity::find()
        .filter(software_item::Column::Id.is_in(sw_ids.clone()))
        .filter(software_item::Column::DeactivatedAt.is_null())
        .all(state.db())
        .await
        .context_to::<HandlerError>()?
        .into_iter()
        .map(|i| (i.id, i))
        .collect();

    // Batch 2: hosts.
    let hosts_map: HashMap<uuid::Uuid, host::Model> = host::Entity::find()
        .filter(host::Column::Id.is_in(host_ids.clone()))
        .all(state.db())
        .await
        .context_to::<HandlerError>()?
        .into_iter()
        .map(|h| (h.id, h))
        .collect();

    // Batch 3: plugin assignments for all relevant roles across all
    // (host_id, software_item_id) combinations that appear in pending_updates.
    // The cross-product filter may include extra rows for pairs not in
    // pending_updates; those are silently ignored during the join below.
    let assignments: Vec<host_software_item_plugin::Model> =
        host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.is_in(host_ids.clone()))
            .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(sw_ids.clone()))
            .filter(host_software_item_plugin::Column::Role.is_in([
                "execute_update",
                "detect_version",
                "fetch_releases",
                "pre_update_hook",
                "post_update_hook",
            ]))
            .order_by_asc(host_software_item_plugin::Column::Ordinal)
            .all(state.db())
            .await
            .context_to::<HandlerError>()?;

    // Index single-valued assignments by (host_id, software_item_id, role).
    // Hook roles are collected separately as Vec since multiple can exist.
    let mut assignments_map: HashMap<
        (uuid::Uuid, uuid::Uuid, String),
        host_software_item_plugin::Model,
    > = HashMap::new();
    let mut hook_assignments_map: HashMap<
        (uuid::Uuid, uuid::Uuid, String),
        Vec<host_software_item_plugin::Model>,
    > = HashMap::new();
    for a in assignments {
        let key = (a.host_id, a.software_item_id, a.role.clone());
        if a.role == "pre_update_hook" || a.role == "post_update_hook" {
            hook_assignments_map.entry(key).or_default().push(a);
        } else {
            assignments_map.insert(key, a);
        }
    }

    // Batch 4: plugin configs referenced by the assignments above.
    let plugin_config_ids: Vec<uuid::Uuid> = assignments_map
        .values()
        .filter_map(|a| a.plugin_config_id)
        .chain(
            hook_assignments_map
                .values()
                .flatten()
                .filter_map(|a| a.plugin_config_id),
        )
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let configs_map: HashMap<uuid::Uuid, plugin_config::Model> = plugin_config::Entity::find()
        .filter(plugin_config::Column::Id.is_in(plugin_config_ids))
        .filter(plugin_config::Column::DeactivatedAt.is_null())
        .all(state.db())
        .await
        .context_to::<HandlerError>()?
        .into_iter()
        .map(|c| (c.id, c))
        .collect();

    // Batch 5: host_software_item rows for `latest_release_metadata`.
    //
    // The cross-product filter may return extra rows; they are silently
    // ignored during the join below (same pattern as batch 3).
    // This allows `enrich_release_info_with_attestation` to reconstruct
    // `release_info` for plugins like GitHub that require asset download URLs.
    let hsi_metadata_map: HashMap<(uuid::Uuid, uuid::Uuid), Option<serde_json::Value>> =
        host_software_item::Entity::find()
            .filter(host_software_item::Column::HostId.is_in(host_ids.clone()))
            .filter(host_software_item::Column::SoftwareItemId.is_in(sw_ids))
            .all(state.db())
            .await
            .context_to::<HandlerError>()?
            .into_iter()
            .map(|m| ((m.host_id, m.software_item_id), m.latest_release_metadata))
            .collect();

    Ok(Some((
        host_ids,
        PendingUpdateRecords {
            pending_updates,
            sw_items_map,
            hosts_map,
            assignments_map,
            hook_assignments_map,
            configs_map,
            hsi_metadata_map,
        },
    )))
}

pub(super) async fn recover_owned_updates_on_connect_with_dispatch_mode(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    runtime_instance_id: Option<uuid::Uuid>,
    successor_dispatch_mode: ReconnectSuccessorDispatchMode,
) -> HandlerResult<()> {
    let failed =
        match crate::queries::update_batches::mark_owned_in_progress_as_failed_on_reconnect(
            state.db(),
            service_id,
            runtime_instance_id,
        )
        .await
        {
            Ok(failed) => failed,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    %service_id,
                    "failed owner-aware reconnect cleanup query"
                );
                bail!(HandlerError::WebSocketSend);
            }
        };

    if failed.is_empty() {
        return Ok(());
    }

    let reason = "Update interrupted: agent restarted".to_string();
    for record in &failed {
        finalize_post_update_with_recovery_timeout_best_effort(state, record).await;
        notify_failed_reconnect_update(
            state,
            service_id,
            record.tenant_id,
            record,
            &reason,
            successor_dispatch_mode,
        )
        .await;
    }

    state
        .notification
        .notification_service
        .push_software_states_for_tenant(state.db(), failed[0].tenant_id)
        .await;

    Ok(())
}

pub(super) async fn prepare_pending_replay_messages(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
) -> HandlerResult<Vec<ControllerMessage>> {
    loop {
        let Some((_host_ids, records)) = load_pending_update_records(state, service_id).await?
        else {
            return Ok(Vec::new());
        };

        tracing::info!(
            %service_id,
            count = records.pending_updates.len(),
            "preparing pending updates on reconnect"
        );

        let mut dispatched_batch_hosts: HashSet<(uuid::Uuid, uuid::Uuid)> = HashSet::new();
        let mut failed_any = false;
        let mut messages = Vec::new();

        for update_record in &records.pending_updates {
            if let Some(batch_id) = update_record.batch_id {
                let key = (batch_id, update_record.host_id);
                if !dispatched_batch_hosts.insert(key) {
                    continue;
                }
            }

            let Some(execute_payload) = build_execute_payload(update_record, &records) else {
                if fail_unreplayable_pending_update(state, service_id, update_record).await? {
                    failed_any = true;
                }
                continue;
            };

            messages.push(ControllerMessage::ExecuteUpdate(Box::new(execute_payload)));

            tracing::info!(
                update_id = %update_record.id,
                %service_id,
                software = %records
                    .sw_items_map
                    .get(&update_record.software_item_id)
                    .map(|i| i.name.as_str())
                    .unwrap_or("?"),
                "prepared pending update on reconnect"
            );
        }

        if !failed_any {
            return Ok(messages);
        }
    }
}

async fn fail_unreplayable_pending_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    update_record: &update_history::Model,
) -> HandlerResult<bool> {
    let reason =
        "Update replay failed: controller could not reconstruct the pending update after reconnect"
            .to_string();
    let completed_at = OffsetDateTime::now_utc();
    let update_result = update_history::Entity::update_many()
        .filter(update_history::Column::Id.eq(update_record.id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Failed),
        )
        .col_expr(
            update_history::Column::CompletedAt,
            Expr::value(Some(completed_at)),
        )
        .col_expr(update_history::Column::Output, Expr::value(reason.clone()))
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(reason.len() as i64),
        )
        .exec(state.db())
        .await
        .context_to::<HandlerError>()?;

    if update_result.rows_affected == 0 {
        return Ok(false);
    }

    let mut failed_record = update_record.clone();
    failed_record.status = update_history::UpdateStatus::Failed;
    failed_record.completed_at = Some(completed_at);
    failed_record.output = reason.clone();
    failed_record.output_bytes = reason.len() as i64;

    finalize_post_update_best_effort(state, &failed_record).await;

    let tenant_id = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
        .context_to::<HandlerError>()?
        .map(|svc| svc.tenant_id)
        .unwrap_or(update_record.tenant_id);

    notify_failed_reconnect_update(
        state,
        service_id,
        tenant_id,
        &failed_record,
        &reason,
        ReconnectSuccessorDispatchMode::ReplayPrepared,
    )
    .await;

    state
        .notification
        .notification_service
        .push_software_states_for_tenant(state.db(), tenant_id)
        .await;

    Ok(true)
}

/// Build an [`ExecuteUpdatePayload`] for a single pending update record using
/// the preloaded lookup maps.
///
/// Returns `None` and logs a warning for any unresolvable dependency (missing
/// software item, missing host, unknown plugin type, etc.) so the caller can
/// skip the record.
fn build_execute_payload(
    update_record: &update_history::Model,
    records: &PendingUpdateRecords,
) -> Option<ExecuteUpdatePayload> {
    let item = match records.sw_items_map.get(&update_record.software_item_id) {
        Some(i) => i,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                software_item_id = %update_record.software_item_id,
                "software item not found or deactivated, skipping pending update"
            );
            return None;
        }
    };

    // Resolve execute_update assignment.
    let exec_key = (update_record.host_id, item.id, "execute_update".to_string());
    let exec_assignment = match records.assignments_map.get(&exec_key) {
        Some(a) => a,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                host_id = %update_record.host_id,
                software_item_id = %item.id,
                "no execute_update plugin assigned, skipping pending update"
            );
            return None;
        }
    };
    let exec_config = exec_assignment
        .plugin_config_id
        .and_then(|pc_id| records.configs_map.get(&pc_id));

    let execute_update_plugin = match build_plugin_assignment_nullable(exec_assignment, exec_config)
    {
        Some(a) => a,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                "unknown plugin type for execute_update, skipping pending update"
            );
            return None;
        }
    };

    // Resolve optional detect_version assignment.
    let detect_key = (update_record.host_id, item.id, "detect_version".to_string());
    let detect_version_plugin = records.assignments_map.get(&detect_key).and_then(|a| {
        let c = a
            .plugin_config_id
            .and_then(|pc_id| records.configs_map.get(&pc_id));
        build_plugin_assignment_nullable(a, c)
    });

    // Resolve hook plugin assignments.
    let pre_hook_key = (
        update_record.host_id,
        update_record.software_item_id,
        "pre_update_hook".to_string(),
    );
    let pre_update_hook_plugins: Vec<PluginAssignment> = records
        .hook_assignments_map
        .get(&pre_hook_key)
        .map(|assignments| {
            assignments
                .iter()
                .filter_map(|a| {
                    let c = a
                        .plugin_config_id
                        .and_then(|pc_id| records.configs_map.get(&pc_id));
                    build_plugin_assignment_nullable(a, c)
                })
                .collect()
        })
        .unwrap_or_default();
    let post_hook_key = (
        update_record.host_id,
        update_record.software_item_id,
        "post_update_hook".to_string(),
    );
    let post_update_hook_plugins: Vec<PluginAssignment> = records
        .hook_assignments_map
        .get(&post_hook_key)
        .map(|assignments| {
            assignments
                .iter()
                .filter_map(|a| {
                    let c = a
                        .plugin_config_id
                        .and_then(|pc_id| records.configs_map.get(&pc_id));
                    build_plugin_assignment_nullable(a, c)
                })
                .collect()
        })
        .unwrap_or_default();

    let host = match records.hosts_map.get(&update_record.host_id) {
        Some(h) => h,
        None => {
            tracing::warn!(
                update_id = %update_record.id,
                host_id = %update_record.host_id,
                "host not found for pending update, skipping"
            );
            return None;
        }
    };

    // Reconstruct release_info from latest_release_metadata so that
    // asset-download plugins (e.g. GitHub) receive the download URLs on
    // reconnect replay — same enrichment used in dispatch_update_to_agent.
    let hsi_metadata = records
        .hsi_metadata_map
        .get(&(update_record.host_id, item.id))
        .and_then(|m| m.as_ref());
    let fetch_key = (update_record.host_id, item.id, "fetch_releases".to_string());
    let fetch_config = records.assignments_map.get(&fetch_key).map(|assignment| {
        let config = assignment
            .plugin_config_id
            .and_then(|pc_id| records.configs_map.get(&pc_id));
        merged_plugin_config(assignment, config)
    });
    let release_info = crate::queries::update_dispatch::enrich_release_info_with_attestation(
        None,
        hsi_metadata,
        fetch_config.as_ref(),
    );

    Some(ExecuteUpdatePayload {
        host_machine_id: host.machine_id.clone(),
        update_history_id: update_record.id,
        software_item_id: item.id,
        software_item_name: item.name.clone(),
        to_version: update_record.to_version.clone().unwrap_or_default(),
        detect_version_plugin,
        execute_update_plugin,
        pre_update_hook_plugins,
        post_update_hook_plugins,
        release_info,
        timeout: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT,
        // Preserve the interactive flag that was set at original dispatch time
        // so that a reconnecting agent receives a PTY when expected.
        interactive: update_record.interactive,
    })
}

// ---------------------------------------------------------------------------
// handle_update_started
// ---------------------------------------------------------------------------

/// Metadata extracted from an `update_history` record when marking it
/// in-progress. Used by the subsequent broadcast phase.
struct UpdateStartedInfo {
    batch_id: Option<uuid::Uuid>,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
}

/// Push MQTT state updates, broadcast `AdminEvent::UpdateStarted`, and emit
/// optional batch progress — all fire-and-forget side-effects.
async fn broadcast_update_started_events(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateStartedPayload,
    info: &UpdateStartedInfo,
) {
    // Push updated software states to MQTT services so that the
    // in_progress flag transitions to true immediately.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    // Broadcast AdminEvent::UpdateStarted so the history-list SSE subscribers
    // can update the "Input Required" badge in real-time without reloading.
    state
        .notification
        .event_broadcaster
        .send(
            info.tenant_id,
            AdminEvent::UpdateStarted {
                update_history_id: payload.update_history_id,
                host_id: info.host_id,
                software_item_id: info.software_item_id,
                interactive: payload.interactive,
            },
        )
        .await;

    // Emit batch progress event if this update is part of a batch.
    if let Some(batch_id) = info.batch_id {
        emit_batch_progress_event(
            state,
            batch_id,
            crate::batch_progress_broadcaster::BatchProgressEvent::UpdateStarted {
                update_history_id: payload.update_history_id,
                software_item_name: resolve_software_item_name(state, info.software_item_id).await,
                host_name: resolve_host_name(state, info.host_id).await,
            },
        )
        .await;
    }
}

/// Handle an `UpdateStarted` message: validate ownership, set status to
/// `InProgress`, clear previous output lines.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id))]
pub(super) async fn handle_update_started(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateStartedPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::info!(
        update_id = %payload.update_history_id,
        from_version = ?payload.from_version,
        "update started"
    );

    if validate_host_link_visibility(
        state.db(),
        service_id,
        payload.update_history_id,
        &linked_host_ids,
    )
    .await
    .is_err()
    {
        return ProcessorResponse::cont();
    }

    let claim = match crate::queries::update_batches::claim_or_replay_update_start_db(
        state.db(),
        payload.update_history_id,
        service_id,
        runtime_instance_id,
        payload.interactive,
    )
    .await
    {
        Ok(claim) => claim,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_history_id = %payload.update_history_id,
                "failed to claim or replay update start"
            );
            return ProcessorResponse::cont();
        }
    };

    match claim {
        crate::queries::update_batches::ClaimExecutionOutcome::Claimed(info) => {
            let info = UpdateStartedInfo {
                batch_id: info.batch_id,
                host_id: info.host_id,
                software_item_id: info.software_item_id,
                tenant_id: info.tenant_id,
            };
            state
                .broadcast
                .update_output_broadcaster
                .create_channel(payload.update_history_id)
                .await;
            broadcast_update_started_events(state, service_id, payload, &info).await;
        }
        crate::queries::update_batches::ClaimExecutionOutcome::Replay(_) => {
            state
                .broadcast
                .update_output_broadcaster
                .get_or_create_channel(payload.update_history_id)
                .await;
        }
        crate::queries::update_batches::ClaimExecutionOutcome::Rejected => {
            return ProcessorResponse::cont();
        }
    }

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// handle_update_output
// ---------------------------------------------------------------------------

/// Handle an `UpdateOutput` message: validate ownership and persist output in
/// one owner-safe step before broadcasting it.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id))]
pub(super) async fn handle_update_output(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateOutputPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::trace!(
        update_id = %payload.update_history_id,
        stream = ?payload.stream,
        "update output"
    );
    if validate_host_link_visibility(
        state.db(),
        service_id,
        payload.update_history_id,
        &linked_host_ids,
    )
    .await
    .is_err()
    {
        return ProcessorResponse::cont();
    }

    let outcome = match crate::queries::update_batches::append_update_output_if_owned(
        state.db(),
        payload.update_history_id,
        service_id,
        runtime_instance_id,
        payload.stream,
        &payload.output,
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_id = %payload.update_history_id,
                "failed to persist update output"
            );
            return ProcessorResponse::cont();
        }
    };

    let persisted_lines = outcome.into_persisted_lines();
    if persisted_lines.is_empty() {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "ignoring stale UpdateOutput"
        );
        return ProcessorResponse::cont();
    }

    for line in persisted_lines {
        state
            .broadcast
            .update_output_broadcaster
            .send_line(
                payload.update_history_id,
                line.id,
                line.output,
                line.stream,
                line.created_at,
            )
            .await;
    }

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// handle_update_result
// ---------------------------------------------------------------------------

/// Map [`UpdateFinalStatus`] to a status string used by SSE events.
fn final_status_str(status: &UpdateFinalStatus) -> &'static str {
    match status {
        UpdateFinalStatus::Completed => "completed",
        _ => "failed",
    }
}

fn truncate_to_char_boundary(output: &str, max_bytes: usize) -> &str {
    if output.len() <= max_bytes {
        return output;
    }

    let mut boundary = max_bytes;
    while boundary > 0 && !output.is_char_boundary(boundary) {
        boundary -= 1;
    }
    &output[..boundary]
}

/// Compare controller-side streaming output against the agent payload and
/// return `(best_output, was_agent_truncated)`.
///
/// On timeout the agent's `accumulated_output` is often incomplete, whereas
/// the controller-side lines were collected in real time.
async fn select_best_output(
    state: &Arc<AppState>,
    update_history_id: uuid::Uuid,
    agent_output: String,
) -> (String, bool) {
    let db_output = {
        let lines = update_output_line::Entity::find()
            .filter(update_output_line::Column::UpdateHistoryId.eq(update_history_id))
            .order_by_asc(update_output_line::Column::CreatedAt)
            .order_by_asc(update_output_line::Column::Id)
            .all(state.db())
            .await
            .unwrap_or_default();
        let mut buf = String::new();
        for line in lines {
            if buf.len() + line.output.len() > MAX_UPDATE_OUTPUT_BYTES {
                break;
            }
            buf.push_str(&line.output);
        }
        buf
    };

    if db_output.len() > agent_output.len() {
        tracing::info!(
            update_id = %update_history_id,
            agent_bytes = agent_output.len(),
            db_bytes = db_output.len(),
            "using controller-side streaming output (more complete than agent payload)"
        );
        (db_output, false)
    } else if agent_output.len() > MAX_UPDATE_OUTPUT_BYTES {
        (
            truncate_to_char_boundary(&agent_output, MAX_UPDATE_OUTPUT_BYTES).to_string(),
            true,
        )
    } else {
        (agent_output, false)
    }
}

/// Update `host_software_item.installed_version` on successful completion.
async fn update_installed_version_on_success(
    state: &Arc<AppState>,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
    to_version: &str,
) {
    let now = time::OffsetDateTime::now_utc();
    if let Err(e) = host_software_item::Entity::update_many()
        .col_expr(
            host_software_item::Column::InstalledVersion,
            sea_orm::sea_query::Expr::value(Some(to_version.to_string())),
        )
        .col_expr(
            host_software_item::Column::InstalledVersionDetectedAt,
            sea_orm::sea_query::Expr::value(Some(now)),
        )
        .col_expr(
            host_software_item::Column::LastUpdatedAt,
            sea_orm::sea_query::Expr::value(Some(now)),
        )
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to update host_software_item installed_version"
        );
    }
}

/// Emit `AdminEvent::UpdateCompleted` for SSE subscribers.
async fn emit_update_completed_event(
    state: &Arc<AppState>,
    tenant_id: uuid::Uuid,
    update_history_id: uuid::Uuid,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
    status: &UpdateFinalStatus,
) {
    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::UpdateCompleted {
                update_history_id,
                host_id,
                software_item_id,
                status: final_status_str(status).to_string(),
            },
        )
        .await;
}

/// Dispatch a notification event for an update result.
async fn dispatch_update_notification(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    record: &update_history::Model,
    payload: &UpdateResultPayload,
) {
    let sw_name = software_item::Entity::find_by_id(record.software_item_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|sw| sw.name.clone());
    let host_name = host::Entity::find_by_id(record.host_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|h| h.hostname.clone());

    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        let resolved_to_version = payload
            .to_version
            .clone()
            .or_else(|| record.to_version.clone())
            .unwrap_or_default();
        let details = match payload.status {
            UpdateFinalStatus::Completed => NotificationEventDetails::UpdateCompleted {
                from_version: record.from_version.clone(),
                to_version: resolved_to_version,
                update_history_id: payload.update_history_id,
            },
            _ => NotificationEventDetails::UpdateFailed {
                from_version: record.from_version.clone(),
                to_version: resolved_to_version,
                error: payload.error.clone(),
                update_history_id: payload.update_history_id,
            },
        };

        state
            .notification
            .notification_dispatcher
            .dispatch(NotificationEvent {
                tenant_id: svc.tenant_id,
                host_id: Some(record.host_id),
                host_name,
                software_item_id: Some(record.software_item_id),
                software_item_name: sw_name,
                plugin_type: None,
                details,
            });
    }
}

/// Handle an `UpdateResult` message: validate ownership, set final status,
/// store output, update installed version on success, push software states.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id, status = ?payload.status))]
pub(super) async fn handle_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: UpdateResultPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::info!(
        update_id = %payload.update_history_id,
        status = ?payload.status,
        error = ?payload.error,
        "update result"
    );
    let record = match validate_host_link_visibility(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return ProcessorResponse::cont(),
    };

    let (final_output, agent_truncated) =
        select_best_output(state, payload.update_history_id, payload.output.clone()).await;

    let final_status = payload.status.clone();
    let updated = match crate::queries::update_batches::finalize_update_result_if_owned(
        state.db(),
        crate::queries::update_batches::FinalizeUpdateResultIfOwnedArgs {
            update_history_id: payload.update_history_id,
            service_id,
            runtime_instance_id,
            status: final_status.clone(),
            error: payload.error.clone(),
            output: final_output.clone(),
            from_version: payload.from_version.clone(),
            to_version: payload.to_version.clone(),
        },
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_history_id = %payload.update_history_id,
                "failed to finalize update result"
            );
            return ProcessorResponse::cont();
        }
    };

    if updated == 0 {
        // The record was not InProgress with this service as owner.  This
        // happens when the agent failed *before* sending UpdateStarted (e.g.
        // SSH connection failure before the update task was spawned): the
        // record stays Pending with no owner, so the owned-InProgress guard
        // above matches nothing.  For failure results, attempt to fail the
        // Pending record directly so it does not remain stuck indefinitely.
        if !matches!(final_status, UpdateFinalStatus::Completed) {
            match crate::queries::update_batches::fail_pending_unowned_update(
                state.db(),
                state.controller_update_protection(),
                payload.update_history_id,
                payload.error.clone(),
                final_output.clone(),
            )
            .await
            {
                Ok(0) => {
                    tracing::debug!(
                        update_id = %payload.update_history_id,
                        "ignoring stale UpdateResult from non-owner"
                    );
                    return ProcessorResponse::cont();
                }
                Ok(_) => {
                    tracing::info!(
                        update_id = %payload.update_history_id,
                        "failed pending unowned update (agent pre-start failure)"
                    );
                    // fall through to post-finalization side-effects
                }
                Err(error) => {
                    tracing::warn!(
                        error = %error,
                        update_id = %payload.update_history_id,
                        "failed to fail pending unowned update"
                    );
                    return ProcessorResponse::cont();
                }
            }
        } else {
            tracing::debug!(
                update_id = %payload.update_history_id,
                "ignoring stale UpdateResult from non-owner"
            );
            return ProcessorResponse::cont();
        }
    }

    if updated > 0 {
        let mut finalized_record = record.clone();
        finalized_record.status = match final_status {
            UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
            _ => update_history::UpdateStatus::Failed,
        };
        finalized_record.completed_at = Some(OffsetDateTime::now_utc());
        finalized_record.output = final_output.clone();
        finalized_record.output_bytes = final_output.len() as i64;
        finalize_post_update_best_effort(state, &finalized_record).await;
    }

    if agent_truncated
        && let Err(error) = update_history::Entity::update_many()
            .filter(update_history::Column::Id.eq(payload.update_history_id))
            .col_expr(update_history::Column::OutputTruncated, Expr::value(true))
            .exec(state.db())
            .await
    {
        tracing::warn!(error = %error, "failed to mark output_truncated");
    }

    // Notify SSE subscribers and clean up streaming output lines.
    state
        .broadcast
        .update_output_broadcaster
        .send_completed(
            payload.update_history_id,
            final_status_str(&final_status).to_string(),
            payload.error.clone(),
        )
        .await;

    if let Err(e) = update_output_line::Entity::delete_many()
        .filter(update_output_line::Column::UpdateHistoryId.eq(payload.update_history_id))
        .exec(state.db())
        .await
    {
        tracing::warn!(error = %e, "failed to clear update output lines");
    }

    // Update installed version on success.
    if payload.status == UpdateFinalStatus::Completed
        && let Some(ref to_version) = payload.to_version
    {
        update_installed_version_on_success(
            state,
            record.host_id,
            record.software_item_id,
            to_version,
        )
        .await;
    }

    // Push updated software states to MQTT services.
    let svc_tenant_id = if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
        Some(svc.tenant_id)
    } else {
        None
    };

    // Batch or queue dispatch.
    if let Some(batch_id) = record.batch_id {
        let event = match payload.status {
            UpdateFinalStatus::Completed => {
                crate::batch_progress_broadcaster::BatchProgressEvent::UpdateCompleted {
                    update_history_id: payload.update_history_id,
                    software_item_name: resolve_software_item_name(state, record.software_item_id)
                        .await,
                    host_name: resolve_host_name(state, record.host_id).await,
                }
            }
            _ => crate::batch_progress_broadcaster::BatchProgressEvent::UpdateFailed {
                update_history_id: payload.update_history_id,
                software_item_name: resolve_software_item_name(state, record.software_item_id)
                    .await,
                host_name: resolve_host_name(state, record.host_id).await,
                error: payload.error.clone(),
            },
        };
        emit_batch_progress_event(state, batch_id, event).await;
        dispatch_next_batch_update(state, service_id, batch_id, record.host_id).await;
    } else {
        dispatch_next_queued_update(state, service_id, record.host_id).await;
    }

    // Emit SSE admin event and notification.
    if let Some(tenant_id) = svc_tenant_id {
        emit_update_completed_event(
            state,
            tenant_id,
            payload.update_history_id,
            record.host_id,
            record.software_item_id,
            &payload.status,
        )
        .await;
    }

    dispatch_update_notification(state, service_id, &record, &payload).await;

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// Reconnect recovery
// ---------------------------------------------------------------------------

/// Notify all subscribers about a single update that was marked failed on reconnect.
///
/// Sends the output-stream completion, broadcasts the SSE event, and dispatches
/// the next update in the queue (batch or standalone).
async fn notify_failed_reconnect_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    record: &uptrakit_shared_db::entity::update_history::Model,
    reason: &str,
    successor_dispatch_mode: ReconnectSuccessorDispatchMode,
) {
    tracing::warn!(
        update_id = %record.id,
        host_id = %record.host_id,
        "in-progress update marked failed due to agent restart"
    );

    state
        .broadcast
        .update_output_broadcaster
        .send_completed(record.id, "failed".to_string(), Some(reason.to_string()))
        .await;

    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::UpdateCompleted {
                update_history_id: record.id,
                host_id: record.host_id,
                software_item_id: record.software_item_id,
                status: "failed".to_string(),
            },
        )
        .await;

    match successor_dispatch_mode {
        ReconnectSuccessorDispatchMode::Immediate => {
            if let Some(batch_id) = record.batch_id {
                dispatch_next_batch_update(state, service_id, batch_id, record.host_id).await;
            } else {
                dispatch_next_queued_update(state, service_id, record.host_id).await;
            }
        }
        ReconnectSuccessorDispatchMode::ReplayPrepared => {
            if let Some(batch_id) = record.batch_id {
                dispatch_next_batch_update_for_replay(state, service_id, batch_id, record.host_id)
                    .await;
            } else {
                dispatch_next_queued_update_for_replay(state, service_id, record.host_id).await;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Batch dispatch helper
// ---------------------------------------------------------------------------

/// Dispatch the next pending update within a batch for the given host.
///
/// Resolves the service's tenant_id, calls `dispatch_next_in_batch`, and logs
/// any errors without failing the calling handler. If the batch just completed,
/// dispatches a notification event.
async fn dispatch_next_batch_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    dispatch_next_batch_update_with_notifier(
        state,
        service_id,
        batch_id,
        host_id,
        &state.notification.notification_service,
        state.controller_update_protection(),
    )
    .await;
}

async fn dispatch_next_batch_update_for_replay(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    let notifier = ReplayPreparationNotifier;
    dispatch_next_batch_update_with_notifier(
        state,
        service_id,
        batch_id,
        host_id,
        &notifier,
        state.controller_update_protection(),
    )
    .await;
}

async fn dispatch_next_batch_update_with_notifier(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    host_id: uuid::Uuid,
    notifier: &dyn crate::ServiceNotifier,
    protection: Option<
        Arc<dyn uptrakit_plugin_infrastructure_registry::ControllerUpdateProtection>,
    >,
) {
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    match crate::queries::update_batches::dispatch_next_in_batch(
        state.db(),
        crate::queries::update_dispatch::DispatchContext {
            notifier,
            protection,
        },
        batch_id,
        host_id,
        tenant_id,
    )
    .await
    {
        Ok(Some(completion)) => {
            handle_batch_completion(state, batch_id, &completion).await;
        }
        Ok(None) => {
            // Batch still in progress — emit updated progress summary.
            emit_batch_progress_from_db(state, batch_id).await;
        }
        Err(e) => {
            tracing::warn!(
                %batch_id,
                %host_id,
                error = %e,
                "failed to dispatch next batch update or update batch status"
            );
        }
    }
}

/// Handle a completed batch: emit progress events, send completion, and
/// dispatch a notification if the batch finished or partially finished.
async fn handle_batch_completion(
    state: &Arc<AppState>,
    batch_id: uuid::Uuid,
    completion: &crate::queries::update_batches::BatchCompletionInfo,
) {
    use uptrakit_shared_types::BatchStatus;

    // Emit final progress summary via broadcaster.
    emit_batch_progress_event(
        state,
        batch_id,
        crate::batch_progress_broadcaster::BatchProgressEvent::Progress {
            completed: completion.completed_count,
            failed: completion.failed_count,
            pending: 0,
            total: completion.total_count,
        },
    )
    .await;

    // Send batch completed event via broadcaster (removes the channel).
    state
        .broadcast
        .batch_progress_broadcaster
        .send_batch_completed(batch_id, completion.status.as_str().to_string())
        .await;

    let details = match completion.status {
        BatchStatus::Completed => NotificationEventDetails::BatchUpdateCompleted {
            batch_id: completion.batch_id,
            total_count: completion.total_count,
            completed_count: completion.completed_count,
        },
        BatchStatus::PartiallyCompleted => {
            NotificationEventDetails::BatchUpdatePartiallyCompleted {
                batch_id: completion.batch_id,
                total_count: completion.total_count,
                completed_count: completion.completed_count,
                failed_count: completion.failed_count,
            }
        }
        _ => return,
    };

    state
        .notification
        .notification_dispatcher
        .dispatch(NotificationEvent {
            tenant_id: completion.tenant_id,
            host_id: None,
            host_name: None,
            software_item_id: None,
            software_item_name: None,
            plugin_type: None,
            details,
        });
}

/// Dispatch the next queued update for the given host after a non-batch
/// update completes.
///
/// Resolves the service's tenant_id, calls `dispatch_next_queued_for_host`,
/// and logs any errors without failing the calling handler.
async fn dispatch_next_queued_update(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    dispatch_next_queued_update_with_notifier(
        state,
        service_id,
        host_id,
        &state.notification.notification_service,
        state.controller_update_protection(),
    )
    .await;
}

async fn dispatch_next_queued_update_for_replay(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
) {
    let notifier = ReplayPreparationNotifier;
    dispatch_next_queued_update_with_notifier(
        state,
        service_id,
        host_id,
        &notifier,
        state.controller_update_protection(),
    )
    .await;
}

async fn dispatch_next_queued_update_with_notifier(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
    notifier: &dyn crate::ServiceNotifier,
    protection: Option<
        Arc<dyn uptrakit_plugin_infrastructure_registry::ControllerUpdateProtection>,
    >,
) {
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    if let Err(e) = crate::queries::update_batches::dispatch_next_queued_for_host(
        state.db(),
        crate::queries::update_dispatch::DispatchContext {
            notifier,
            protection,
        },
        host_id,
        tenant_id,
    )
    .await
    {
        tracing::warn!(
            %host_id,
            error = %e,
            "failed to dispatch next queued update for host"
        );
    }
}

// ---------------------------------------------------------------------------
// Batch progress helpers
// ---------------------------------------------------------------------------

/// Send a batch progress event to all SSE subscribers.
async fn emit_batch_progress_event(
    state: &Arc<AppState>,
    batch_id: uuid::Uuid,
    event: crate::batch_progress_broadcaster::BatchProgressEvent,
) {
    state
        .broadcast
        .batch_progress_broadcaster
        .send(batch_id, event)
        .await;
}

/// Compute and emit a progress summary from the DB for an in-progress batch.
async fn emit_batch_progress_from_db(state: &Arc<AppState>, batch_id: uuid::Uuid) {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

    let batch = match update_history::Entity::find()
        .filter(update_history::Column::BatchId.eq(batch_id))
        .all(state.db())
        .await
    {
        Ok(records) => records,
        Err(_) => return,
    };

    let total = batch.len() as i32;
    let mut completed: i64 = 0;
    let mut failed: i64 = 0;
    let mut pending: i64 = 0;

    for r in &batch {
        match r.status {
            update_history::UpdateStatus::Completed => completed += 1,
            update_history::UpdateStatus::Failed => failed += 1,
            update_history::UpdateStatus::Pending | update_history::UpdateStatus::InProgress => {
                pending += 1;
            }
            _ => {
                tracing::warn!("Unknown update status {:?}, counting as pending", r.status);
                pending += 1;
            }
        }
    }

    emit_batch_progress_event(
        state,
        batch_id,
        crate::batch_progress_broadcaster::BatchProgressEvent::Progress {
            completed,
            failed,
            pending,
            total,
        },
    )
    .await;
}

/// Resolve a software item name by ID (for batch progress events).
async fn resolve_software_item_name(state: &Arc<AppState>, item_id: uuid::Uuid) -> String {
    software_item::Entity::find_by_id(item_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|sw| sw.name)
        .unwrap_or_else(|| "Unknown Software".to_string())
}

/// Resolve a host name by ID (for batch progress events).
async fn resolve_host_name(state: &Arc<AppState>, host_id: uuid::Uuid) -> String {
    host::Entity::find_by_id(host_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|h| h.friendly_name)
        .unwrap_or_else(|| "Unknown Host".to_string())
}

// ---------------------------------------------------------------------------
// handle_batch_update_result
// ---------------------------------------------------------------------------

/// Process a single item result within a batch: validate ownership, persist
/// status/output, and update the installed version on success.
async fn process_single_batch_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    result: &uptrakit_internal_wire::BatchUpdateItemResult,
    linked_host_ids: &HashSet<uuid::Uuid>,
    runtime_instance_id: Option<uuid::Uuid>,
) {
    let history_record = match update_history::Entity::find_by_id(result.update_history_id)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                update_history_id = %result.update_history_id,
                "update_history record not found"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                update_history_id = %result.update_history_id,
                "failed to look up update_history"
            );
            return;
        }
    };

    if !linked_host_ids.contains(&history_record.host_id) {
        tracing::warn!(
            %service_id,
            update_history_id = %result.update_history_id,
            host_id = %history_record.host_id,
            "service attempted to update update_history for unlinked host"
        );
        return;
    }

    let finalized = match crate::queries::update_batches::finalize_batch_item_if_owned(
        state.db(),
        crate::queries::update_batches::FinalizeBatchItemIfOwnedArgs {
            update_history_id: result.update_history_id,
            service_id,
            runtime_instance_id,
            status: result.status.clone(),
            error: result.error.clone(),
            output: result.output.clone(),
            installed_version: result.installed_version.clone(),
        },
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                error = %error,
                update_history_id = %result.update_history_id,
                "failed to finalize batch item"
            );
            return;
        }
    };

    if finalized == 0 {
        tracing::debug!(
            update_history_id = %result.update_history_id,
            "ignoring stale BatchUpdateResult item"
        );
        return;
    }

    let mut finalized_record = history_record.clone();
    finalized_record.status = match result.status {
        UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
        _ => update_history::UpdateStatus::Failed,
    };
    finalized_record.completed_at = Some(OffsetDateTime::now_utc());
    finalized_record.output = if result.output.is_empty() {
        result.error.clone().unwrap_or_default()
    } else {
        result.output.clone()
    };
    finalized_record.output_bytes = finalized_record.output.len() as i64;
    finalize_post_update_best_effort(state, &finalized_record).await;

    // On success, update installed version by host_software_item ID.
    if result.status == UpdateFinalStatus::Completed
        && let Some(ref new_version) = result.installed_version
    {
        let now = time::OffsetDateTime::now_utc();
        if let Err(e) = host_software_item::Entity::update_many()
            .col_expr(
                host_software_item::Column::InstalledVersion,
                sea_orm::sea_query::Expr::value(Some(new_version.clone())),
            )
            .col_expr(
                host_software_item::Column::InstalledVersionDetectedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .col_expr(
                host_software_item::Column::LastUpdatedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(host_software_item::Column::Id.eq(result.host_software_item_id))
            .exec(state.db())
            .await
        {
            tracing::warn!(
                error = %e,
                host_software_item_id = %result.host_software_item_id,
                "failed to update host_software_item installed_version"
            );
        }
    }
}

/// Handle a `BatchUpdateResult` message: update per-item
/// `update_history` rows and `host_software_item.installed_version`
/// for successful items.
#[tracing::instrument(skip_all, fields(%service_id, batch_id = %payload.batch_id))]
pub(super) async fn handle_batch_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: BatchUpdateResultPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::info!(
        batch_id = %payload.batch_id,
        results = payload.results.len(),
        "batch update result"
    );

    for result in &payload.results {
        process_single_batch_result(
            state,
            service_id,
            result,
            &linked_host_ids,
            runtime_instance_id,
        )
        .await;
    }

    // Push updated software states to MQTT so that `in_progress = false`
    // and the new `installed_version` are reflected immediately after the batch
    // completes.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// Role-based plugin helpers
// ---------------------------------------------------------------------------

/// Build a [`PluginAssignment`] from a role plugin row and its optional config.
///
/// When `config` is `None` (no stored `plugin_config` linked to the assignment),
/// the plugin type is read from `assignment.plugin_type` and the effective config
/// is built from the assignment-level override alone.
///
/// Returns `None` if the plugin type string cannot be deserialized.
fn merged_plugin_config(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> serde_json::Value {
    uptrakit_config_merge::resolve_effective_config(
        None,
        config.map(|c| &c.config),
        assignment.config.as_ref(),
    )
}

fn build_plugin_assignment_nullable(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> Option<PluginAssignment> {
    let plugin_type_str = config
        .map(|c| c.plugin_type.clone())
        .unwrap_or_else(|| assignment.plugin_type.clone());
    let plugin_type = uptrakit_shared_types::PluginTypeId::new(plugin_type_str);
    let merged_config = merged_plugin_config(assignment, config);

    Some(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}

/// Handle a `StdinAttention` message from the agent.
///
/// Broadcasts a stdin attention event to all SSE subscribers of the update.
#[tracing::instrument(skip_all, fields(%service_id, update_history_id = %payload.update_history_id))]
pub(crate) async fn handle_stdin_attention(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &uptrakit_internal_wire::StdinAttentionPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    runtime_instance_id: Option<uuid::Uuid>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    // Validate that this service owns the update
    if let Err(e) = validate_host_link_visibility(
        state.db(),
        service_id,
        payload.update_history_id,
        &linked_host_ids,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            "StdinAttention ownership validation failed"
        );
        return ProcessorResponse::cont();
    }

    let updated = match crate::queries::update_batches::touch_stdin_attention_if_owned(
        state.db(),
        payload.update_history_id,
        service_id,
        runtime_instance_id,
        payload.hint.clone(),
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(error = %error, "StdinAttention ownership validation failed");
            return ProcessorResponse::cont();
        }
    };

    if updated == 0 {
        tracing::debug!(
            update_history_id = %payload.update_history_id,
            "ignoring stale StdinAttention from non-owner"
        );
        return ProcessorResponse::cont();
    }

    state
        .broadcast
        .update_output_broadcaster
        .send_stdin_attention(payload.update_history_id, payload.hint.clone())
        .await;

    // Fire notification so admins can be alerted that input is needed.
    if let Ok(Some(record)) = update_history::Entity::find_by_id(payload.update_history_id)
        .one(state.db())
        .await
    {
        let host_name = host::Entity::find_by_id(record.host_id)
            .one(state.db())
            .await
            .ok()
            .flatten()
            .map(|h| h.friendly_name);

        let sw_name =
            uptrakit_shared_db::entity::software_item::Entity::find_by_id(record.software_item_id)
                .one(state.db())
                .await
                .ok()
                .flatten()
                .map(|s| s.name);

        state.notification.notification_dispatcher.dispatch(
            crate::notifications::events::NotificationEvent {
                tenant_id: record.tenant_id,
                host_id: Some(record.host_id),
                host_name,
                software_item_id: Some(record.software_item_id),
                software_item_name: sw_name,
                plugin_type: None,
                details: crate::notifications::events::NotificationEventDetails::StdinAttention {
                    update_history_id: payload.update_history_id,
                    hint: payload.hint.clone(),
                },
            },
        );
    }

    tracing::debug!(
        hint = ?payload.hint,
        "broadcast StdinAttention for update"
    );
    ProcessorResponse::cont()
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use sea_orm::{ActiveModelTrait, EntityTrait, Set};
    use std::{future::Future, pin::Pin, sync::Arc};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_registry::{
        CatalogConfig, ControllerPostUpdateContext, ControllerProtectionContext,
        ControllerProtectionDecision, ControllerUpdateProtection, ControllerUpdateProtectionOps,
        NotificationOps, NotificationTransport, PluginConfigOps, PluginError, PluginMetadataOps,
        PluginOps, PluginResult, PluginSurfaceActionOps, PluginSurfaceOps, PostUpdateOutcome,
        SoftwareItemCreatedEvent, SoftwareItemLifecycle, SoftwareItemLifecycleContext,
        SoftwareItemLifecycleOps, SoftwareItemPatch, SurfaceActionContext, build_catalog,
    };
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, service_host, software_item,
        update_history,
    };
    use uptrakit_shared_types::{PluginTypeId, ServiceStatus};
    use uuid::Uuid;

    struct ReplayFailProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for ReplayFailProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_replay_fail_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for ReplayFailProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "replay protection failure".to_string()
            )))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            Ok(PostUpdateOutcome::default())
        }
    }

    struct FinalizeErrorProtection;

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for FinalizeErrorProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_finalize_error_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for FinalizeErrorProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            Ok(ControllerProtectionDecision::skipped(None))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            Err(rootcause::report!(PluginError::PluginInternal(
                "finalize failure".to_string()
            )))
        }
    }

    struct ProtectionOverridePluginOps {
        inner: Arc<dyn PluginOps>,
        protection: Arc<dyn ControllerUpdateProtection>,
    }

    impl PluginMetadataOps for ProtectionOverridePluginOps {
        fn get(
            &self,
            id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            self.inner.get(id)
        }

        fn all(&self) -> Vec<&uptrakit_plugin_infrastructure_registry::PluginDescriptor> {
            self.inner.all()
        }
    }

    impl PluginConfigOps for ProtectionOverridePluginOps {}

    impl PluginSurfaceActionOps for ProtectionOverridePluginOps {
        fn handle_surface_action<'a>(
            &'a self,
            ctx: &'a SurfaceActionContext<'a>,
            surface_id: &'a str,
            action_id: &'a str,
            params: serde_json::Value,
        ) -> Pin<Box<dyn Future<Output = std::result::Result<serde_json::Value, String>> + Send + 'a>>
        {
            self.inner
                .handle_surface_action(ctx, surface_id, action_id, params)
        }
    }

    impl PluginSurfaceOps for ProtectionOverridePluginOps {
        fn surface_registrations(
            &self,
        ) -> Vec<uptrakit_internal_wire::surfaces::SurfaceRegistration> {
            self.inner.surface_registrations()
        }
    }

    impl NotificationOps for ProtectionOverridePluginOps {
        fn transport(
            &self,
            id: &uptrakit_shared_types::PluginTypeId,
        ) -> Option<Arc<dyn NotificationTransport>> {
            self.inner.transport(id)
        }

        fn notification_supported_types(&self) -> Vec<uptrakit_shared_types::PluginTypeId> {
            self.inner.notification_supported_types()
        }
    }

    impl SoftwareItemLifecycleOps for ProtectionOverridePluginOps {
        fn on_software_item_created<'a>(
            &'a self,
            event: &'a SoftwareItemCreatedEvent,
            ctx: &'a SoftwareItemLifecycleContext,
        ) -> Pin<Box<dyn Future<Output = Option<SoftwareItemPatch>> + Send + 'a>> {
            self.inner.on_software_item_created(event, ctx)
        }

        fn software_item_lifecycle_plugins(&self) -> &[Arc<dyn SoftwareItemLifecycle>] {
            self.inner.software_item_lifecycle_plugins()
        }
    }

    impl ControllerUpdateProtectionOps for ProtectionOverridePluginOps {
        fn controller_update_protection(&self) -> Option<Arc<dyn ControllerUpdateProtection>> {
            Some(self.protection.clone())
        }
    }

    async fn build_test_state_with_protection(
        db: sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        protection: Arc<dyn ControllerUpdateProtection>,
    ) -> Arc<AppState> {
        let base_plugin_ops: Arc<dyn PluginOps> = Arc::new(
            build_catalog(&CatalogConfig::default()).expect("catalog should build in tests"),
        );
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(ProtectionOverridePluginOps {
            inner: base_plugin_ops,
            protection,
        });
        let (state, _jwt) =
            crate::test_harness::build_test_state_with_plugin_ops(db, tenant_id, Some(plugin_ops))
                .await;
        state
    }

    async fn insert_service_row(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        service_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("svc-{service_id}")),
            friendly_name: Set(format!("Service {service_id}")),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some("uptrakit-agent".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
    }

    async fn insert_linked_host(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        service_id: Uuid,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let host_id = Uuid::now_v7();

        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{service_id}-{host_id}")),
            hostname: Set(format!("host-{service_id}-{host_id}")),
            friendly_name: Set(format!("Host {service_id} {host_id}")),
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
        .unwrap();

        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        host_id
    }

    async fn insert_software_item(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        name: &str,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap()
        .id
    }

    async fn insert_pending_update_without_assignment(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let update_history_id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Pending),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(None),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap();
        update_history_id
    }

    async fn insert_replayable_queued_update(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let host_software_item_id = host_software_item::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(Some("demo".to_string())),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("security".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap()
        .id;

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(host_software_item_id),
            plugin_config_id: Set(None),
            plugin_type: Set("generic_shell".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("demo".to_string()),
            config: Set(None),
            execution_site: Set("agent".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        let update_history_id = Uuid::now_v7();
        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(Some(host_software_item_id)),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Queued),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(None),
            completed_at: Set(None),
            created_at: Set(now),
            update_category: Set("security".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        update_history_id
    }

    #[tokio::test]
    async fn prepare_pending_replay_messages_fails_unreplayable_rows_and_unblocks_successors() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = Uuid::now_v7();

        insert_service_row(state.db(), tenant_id, service_id).await;
        let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
        let broken_item_id = insert_software_item(state.db(), tenant_id, "broken").await;
        let queued_item_id = insert_software_item(state.db(), tenant_id, "queued").await;

        let broken_update_id = insert_pending_update_without_assignment(
            state.db(),
            tenant_id,
            host_id,
            broken_item_id,
        )
        .await;
        let queued_update_id =
            insert_replayable_queued_update(state.db(), tenant_id, host_id, queued_item_id).await;

        let messages = prepare_pending_replay_messages(&state, service_id)
            .await
            .unwrap();

        assert_eq!(
            messages.len(),
            1,
            "a promoted successor should be prepared once the broken pending row is failed"
        );
        match &messages[0] {
            ControllerMessage::ExecuteUpdate(payload) => {
                assert_eq!(payload.update_history_id, queued_update_id);
            }
            other => panic!("unexpected replay message: {other:?}"),
        }

        let broken_row = update_history::Entity::find_by_id(broken_update_id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(broken_row.status, update_history::UpdateStatus::Failed);
        assert!(
            broken_row.output.contains("replay failed"),
            "failed pending row should record why reconnect replay could not continue"
        );

        let queued_row = update_history::Entity::find_by_id(queued_update_id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queued_row.status, update_history::UpdateStatus::Pending);
    }

    #[tokio::test]
    async fn prepare_pending_replay_messages_skips_replay_dispatch_when_successor_protection_fails()
    {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db, tenant_id, Arc::new(ReplayFailProtection)).await;
        let service_id = Uuid::now_v7();

        insert_service_row(state.db(), tenant_id, service_id).await;
        let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
        let broken_item_id = insert_software_item(state.db(), tenant_id, "broken").await;
        let queued_item_id = insert_software_item(state.db(), tenant_id, "queued").await;

        insert_pending_update_without_assignment(state.db(), tenant_id, host_id, broken_item_id)
            .await;
        let queued_update_id =
            insert_replayable_queued_update(state.db(), tenant_id, host_id, queued_item_id).await;

        let messages = prepare_pending_replay_messages(&state, service_id)
            .await
            .unwrap();
        assert!(
            messages.is_empty(),
            "successor protection failure must prevent replay ExecuteUpdate payloads"
        );

        let queued_row = update_history::Entity::find_by_id(queued_update_id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(queued_row.status, update_history::UpdateStatus::Failed);
        assert_eq!(
            queued_row.pre_update_protection_status.as_deref(),
            Some("failed")
        );
    }

    #[tokio::test]
    async fn handle_update_result_unowned_failure_finalization_error_still_promotes_successor() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let state =
            build_test_state_with_protection(db, tenant_id, Arc::new(FinalizeErrorProtection))
                .await;
        let service_id = Uuid::now_v7();

        insert_service_row(state.db(), tenant_id, service_id).await;
        let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
        let failed_item_id = insert_software_item(state.db(), tenant_id, "failed-item").await;
        let queued_item_id = insert_software_item(state.db(), tenant_id, "queued-item").await;

        let pending_unowned_id = insert_pending_update_without_assignment(
            state.db(),
            tenant_id,
            host_id,
            failed_item_id,
        )
        .await;
        let queued_update_id =
            insert_replayable_queued_update(state.db(), tenant_id, host_id, queued_item_id).await;

        let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::from([host_id])));
        let _ = handle_update_result(
            &state,
            service_id,
            UpdateResultPayload {
                update_history_id: pending_unowned_id,
                status: UpdateFinalStatus::Failed,
                error: Some("ssh pre-start failure".to_string()),
                output: String::new(),
                from_version: None,
                to_version: None,
            },
            &linked_host_ids,
            Some(Uuid::now_v7()),
        )
        .await;

        let failed_row = update_history::Entity::find_by_id(pending_unowned_id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(failed_row.status, update_history::UpdateStatus::Failed);

        let queued_row = update_history::Entity::find_by_id(queued_update_id)
            .one(state.db())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            queued_row.status,
            update_history::UpdateStatus::Pending,
            "finalization errors for pre-start failures must not block queue progression"
        );
    }

    #[tokio::test]
    async fn load_pending_update_records_skips_deactivated_hosts() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;
        let service_id = Uuid::now_v7();

        insert_service_row(state.db(), tenant_id, service_id).await;
        let host_id = insert_linked_host(state.db(), tenant_id, service_id).await;
        let software_item_id = insert_software_item(state.db(), tenant_id, "deactivated").await;
        insert_replayable_queued_update(state.db(), tenant_id, host_id, software_item_id).await;

        host::ActiveModel {
            id: Set(host_id),
            deactivated_at: Set(Some(OffsetDateTime::now_utc())),
            ..Default::default()
        }
        .update(state.db())
        .await
        .unwrap();

        let records = load_pending_update_records(&state, service_id)
            .await
            .unwrap();

        assert!(
            records.is_none(),
            "deactivated hosts must not produce pending replay work"
        );
    }

    #[tokio::test]
    async fn select_best_output_truncates_agent_output_on_utf8_boundary() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db, tenant_id).await;

        let agent_output = format!("{}étail", "a".repeat(MAX_UPDATE_OUTPUT_BYTES - 1));

        let (output, truncated) = select_best_output(&state, Uuid::now_v7(), agent_output).await;

        assert!(truncated);
        assert_eq!(output, "a".repeat(MAX_UPDATE_OUTPUT_BYTES - 1));
    }
}
