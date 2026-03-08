//! Update delivery, ownership validation, and update-lifecycle message handlers.
//!
//! Contains `validate_update_ownership`, `load_linked_host_ids`,
//! `deliver_pending_updates`, and the per-message handlers
//! `handle_update_started`, `handle_update_output`, and `handle_update_result`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Set};

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    BatchHostPackageUpdateResultPayload, ControllerMessage, ExecuteUpdatePayload, OutgoingSeq,
    PluginAssignment, UpdateFinalStatus, UpdateOutputPayload, UpdateResultPayload,
    UpdateStartedPayload,
};
use uptrakit_shared_db::entity::{
    host, host_package, host_package_update_history, host_software_item, host_software_item_plugin,
    plugin_config, service, service_host, software_item, update_history, update_output_line,
};

use super::{HandlerError, HandlerResult, LoopAction, MAX_UPDATE_OUTPUT_BYTES};
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::routes::service_ws::protocol::serialize_controller_msg;

// ---------------------------------------------------------------------------
// load_linked_host_ids
// ---------------------------------------------------------------------------

/// Load the set of host IDs linked to the given service.
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn load_linked_host_ids(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
) -> HandlerResult<HashSet<uuid::Uuid>> {
    let links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(db)
        .await
        .context_to::<HandlerError>()?;

    Ok(links.into_iter().map(|l| l.host_id).collect())
}

// ---------------------------------------------------------------------------
// validate_update_ownership
// ---------------------------------------------------------------------------

/// Validate that an `update_history` record belongs to a host linked to the
/// current service. Returns the record on success, logs a warning and returns
/// an error if the service does not own the record.
#[tracing::instrument(skip_all, fields(%service_id, %update_history_id))]
pub(super) async fn validate_update_ownership(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    update_history_id: uuid::Uuid,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> HandlerResult<update_history::Model> {
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

// ---------------------------------------------------------------------------
// deliver_pending_updates
// ---------------------------------------------------------------------------

/// Deliver pending updates for hosts linked to this service.
///
/// On service reconnect, we check for any `update_history` records with
/// `status = Pending` for hosts linked to this service and send them.
///
/// All auxiliary data (software items, hosts, plugin assignments, plugin configs)
/// is loaded in four batched queries and joined in memory to avoid N+1 round-trips.
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn deliver_pending_updates(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
) -> HandlerResult<()> {
    // 1. Find host_ids linked to this service.
    let host_links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(state.db())
        .await
        .context_to::<HandlerError>()?;

    if host_links.is_empty() {
        return Ok(());
    }

    let host_ids: Vec<uuid::Uuid> = host_links.iter().map(|l| l.host_id).collect();

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
        return Ok(());
    }

    tracing::info!(
        %service_id,
        count = pending_updates.len(),
        "delivering pending updates on reconnect"
    );

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

    // Batch 3: plugin assignments for the three relevant roles across all
    // (host_id, software_item_id) combinations that appear in pending_updates.
    // The cross-product filter may include extra rows for pairs not in
    // pending_updates; those are silently ignored during the join below.
    // `fetch_releases` is included so its plugin config can be used to extract
    // the `require_attestation` flag when enriching `release_info`.
    let assignments: Vec<host_software_item_plugin::Model> =
        host_software_item_plugin::Entity::find()
            .filter(host_software_item_plugin::Column::HostId.is_in(host_ids.clone()))
            .filter(host_software_item_plugin::Column::SoftwareItemId.is_in(sw_ids.clone()))
            .filter(host_software_item_plugin::Column::Role.is_in([
                "execute_update",
                "detect_version",
                "fetch_releases",
            ]))
            .all(state.db())
            .await
            .context_to::<HandlerError>()?;

    // Index assignments by (host_id, software_item_id, role).
    let assignments_map: HashMap<
        (uuid::Uuid, uuid::Uuid, String),
        host_software_item_plugin::Model,
    > = assignments
        .into_iter()
        .map(|a| ((a.host_id, a.software_item_id, a.role.clone()), a))
        .collect();

    // Batch 4: plugin configs referenced by the assignments above.
    let plugin_config_ids: Vec<uuid::Uuid> = assignments_map
        .values()
        .map(|a| a.plugin_config_id)
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
            .filter(host_software_item::Column::HostId.is_in(host_ids))
            .filter(host_software_item::Column::SoftwareItemId.is_in(sw_ids))
            .all(state.db())
            .await
            .context_to::<HandlerError>()?
            .into_iter()
            .map(|m| ((m.host_id, m.software_item_id), m.latest_release_metadata))
            .collect();

    // 3. Build ExecuteUpdatePayload for each pending update using HashMap lookups.
    //
    // Batch-aware filtering: for updates within a batch, only dispatch the
    // first pending update per (batch_id, host_id) — the rest are dispatched
    // sequentially as each completes via dispatch_next_in_batch.
    let mut dispatched_batch_hosts: HashSet<(uuid::Uuid, uuid::Uuid)> = HashSet::new();

    for update_record in pending_updates {
        if let Some(batch_id) = update_record.batch_id {
            let key = (batch_id, update_record.host_id);
            if !dispatched_batch_hosts.insert(key) {
                // Already dispatching the first update for this (batch, host);
                // skip subsequent ones — they will be dispatched on completion.
                continue;
            }
        }
        let Some(item) = sw_items_map.get(&update_record.software_item_id) else {
            tracing::warn!(
                update_id = %update_record.id,
                software_item_id = %update_record.software_item_id,
                "software item not found or deactivated, skipping pending update"
            );
            continue;
        };

        // Resolve execute_update assignment.
        let exec_key = (update_record.host_id, item.id, "execute_update".to_string());
        let Some(exec_assignment) = assignments_map.get(&exec_key) else {
            tracing::warn!(
                update_id = %update_record.id,
                host_id = %update_record.host_id,
                software_item_id = %item.id,
                "no execute_update plugin assigned, skipping pending update"
            );
            continue;
        };
        let Some(exec_config) = configs_map.get(&exec_assignment.plugin_config_id) else {
            tracing::warn!(
                update_id = %update_record.id,
                plugin_config_id = %exec_assignment.plugin_config_id,
                "execute_update plugin config not found or deactivated, skipping"
            );
            continue;
        };

        let execute_update_plugin = match build_plugin_assignment(exec_assignment, exec_config) {
            Some(a) => a,
            None => {
                tracing::warn!(
                    update_id = %update_record.id,
                    "unknown plugin type for execute_update, skipping pending update"
                );
                continue;
            }
        };

        // Resolve optional detect_version assignment.
        let detect_key = (update_record.host_id, item.id, "detect_version".to_string());
        let detect_version_plugin = assignments_map
            .get(&detect_key)
            .and_then(|a| configs_map.get(&a.plugin_config_id).map(|c| (a, c)))
            .and_then(|(a, c)| build_plugin_assignment(a, c));

        // Resolve hooks from the execute_update plugin config + per-role override.
        let resolved_hooks = uptrakit_update_hooks::resolve_hooks(
            &exec_config.config,
            exec_assignment.config_override.as_ref(),
        );

        let Some(host) = hosts_map.get(&update_record.host_id) else {
            tracing::warn!(
                update_id = %update_record.id,
                host_id = %update_record.host_id,
                "host not found for pending update, skipping"
            );
            continue;
        };

        // Reconstruct release_info from latest_release_metadata so that
        // asset-download plugins (e.g. GitHub) receive the download URLs on
        // reconnect replay — same enrichment used in dispatch_update_to_agent.
        let hsi_metadata = hsi_metadata_map
            .get(&(update_record.host_id, item.id))
            .and_then(|m| m.as_ref());
        let fetch_key = (update_record.host_id, item.id, "fetch_releases".to_string());
        let fetch_config = assignments_map
            .get(&fetch_key)
            .and_then(|a| configs_map.get(&a.plugin_config_id))
            .map(|c| &c.config);
        let release_info = crate::queries::update_triggers::enrich_release_info_with_attestation(
            None,
            hsi_metadata,
            fetch_config,
        );

        let execute_payload = ExecuteUpdatePayload {
            host_machine_id: host.machine_id.clone(),
            update_history_id: update_record.id,
            software_item_id: item.id,
            software_item_name: item.name.clone(),
            to_version: update_record.to_version.clone(),
            detect_version_plugin,
            execute_update_plugin,
            pre_update_hooks: resolved_hooks.pre_update_hooks,
            post_update_hooks: resolved_hooks.post_update_hooks,
            release_info,
            timeout_seconds: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT_SECS,
        };

        let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
        let Some(json) = serialize_controller_msg(out_seq, msg) else {
            continue;
        };

        if sink.send(Message::Text(json.into())).await.is_err() {
            bail!(HandlerError::WebSocketSend);
        }

        tracing::info!(
            update_id = %update_record.id,
            %service_id,
            software = %item.name,
            "delivered pending update on reconnect"
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// handle_update_started
// ---------------------------------------------------------------------------

/// Handle an `UpdateStarted` message: validate ownership, set status to
/// `InProgress`, clear previous output lines.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id))]
pub(super) async fn handle_update_started(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateStartedPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::info!(
        update_id = %payload.update_history_id,
        from_version = ?payload.from_version,
        "update started"
    );
    let record = match validate_update_ownership(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return LoopAction::Continue,
    };
    let record_batch_id = record.batch_id;
    let record_host_id = record.host_id;
    let record_software_item_id = record.software_item_id;
    let mut active: update_history::ActiveModel = record.into();
    active.status = Set(update_history::UpdateStatus::InProgress);
    active.started_at = Set(time::OffsetDateTime::now_utc());
    if payload.from_version.is_some() {
        active.from_version = Set(payload.from_version.clone());
    }
    active.output = Set(String::new());
    active.output_bytes = Set(0);
    if let Err(e) = active.update(state.db()).await {
        tracing::warn!(
            error = %e,
            "failed to update update_history status"
        );
    }
    if let Err(e) = update_output_line::Entity::delete_many()
        .filter(update_output_line::Column::UpdateHistoryId.eq(payload.update_history_id))
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to clear update output lines"
        );
    }

    // Create a broadcast channel so SSE subscribers can receive live output.
    state
        .update_output_broadcaster
        .create_channel(payload.update_history_id)
        .await;

    // Push updated software states to MQTT services so that the
    // in_progress flag transitions to true immediately.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    // Emit batch progress event if this update is part of a batch.
    if let Some(batch_id) = record_batch_id {
        emit_batch_progress_event(
            state,
            batch_id,
            crate::batch_progress_broadcaster::BatchProgressEvent::UpdateStarted {
                update_history_id: payload.update_history_id,
                software_item_name: resolve_software_item_name(state, record_software_item_id)
                    .await,
                host_name: resolve_host_name(state, record_host_id).await,
            },
        )
        .await;
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_update_output
// ---------------------------------------------------------------------------

/// Handle an `UpdateOutput` message: validate ownership, append output line
/// (capped at `MAX_UPDATE_OUTPUT_BYTES`).
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id))]
pub(super) async fn handle_update_output(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &UpdateOutputPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::trace!(
        update_id = %payload.update_history_id,
        stream = ?payload.stream,
        "update output"
    );
    if validate_update_ownership(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    .is_err()
    {
        return LoopAction::Continue;
    }

    let output_line = format!("{}\n", payload.output);
    let line_len = output_line.len() as i64;
    let updated = update_history::Entity::update_many()
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::col(update_history::Column::OutputBytes).add(line_len),
        )
        .filter(update_history::Column::Id.eq(payload.update_history_id))
        .filter(update_history::Column::OutputBytes.lt(MAX_UPDATE_OUTPUT_BYTES as i64))
        .exec(state.db())
        .await;

    let Ok(updated) = updated else {
        tracing::warn!(
            update_id = %payload.update_history_id,
            "failed to update output bytes"
        );
        return LoopAction::Continue;
    };

    if updated.rows_affected == 0 {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "update output exceeded {MAX_UPDATE_OUTPUT_BYTES} byte cap, dropping"
        );
        return LoopAction::Continue;
    }

    let line_id = uuid::Uuid::now_v7();
    let created_at = time::OffsetDateTime::now_utc();
    let line = update_output_line::ActiveModel {
        id: Set(line_id),
        update_history_id: Set(payload.update_history_id),
        stream: Set(payload.stream),
        output: Set(output_line.clone()),
        created_at: Set(created_at),
    };
    if let Err(e) = update_output_line::Entity::insert(line)
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to insert update output line"
        );
    }

    // Fan out to SSE subscribers.
    state
        .update_output_broadcaster
        .send_line(
            payload.update_history_id,
            line_id,
            output_line,
            payload.stream,
            created_at,
        )
        .await;

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_update_result
// ---------------------------------------------------------------------------

/// Handle an `UpdateResult` message: validate ownership, set final status,
/// store output, update installed version on success, push software states.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id, status = ?payload.status))]
pub(super) async fn handle_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: UpdateResultPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::info!(
        update_id = %payload.update_history_id,
        status = ?payload.status,
        error = ?payload.error,
        "update result"
    );
    let record = match validate_update_ownership(
        state.db(),
        service_id,
        payload.update_history_id,
        linked_host_ids,
    )
    .await
    {
        Ok(r) => r,
        Err(_) => return LoopAction::Continue,
    };
    let mut active: update_history::ActiveModel = record.clone().into();
    active.status = Set(match payload.status {
        UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
        UpdateFinalStatus::Failed => update_history::UpdateStatus::Failed,
        _ => update_history::UpdateStatus::Failed,
    });
    active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));

    // Load controller-side streaming output lines and compare against the
    // agent payload. On timeout the agent's accumulated_output is often
    // incomplete, whereas the controller-side lines were collected in real
    // time. Use whichever source is longer to preserve the most data.
    let db_output = {
        let lines = update_output_line::Entity::find()
            .filter(update_output_line::Column::UpdateHistoryId.eq(payload.update_history_id))
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

    let final_output = if db_output.len() > payload.output.len() {
        tracing::info!(
            update_id = %payload.update_history_id,
            agent_bytes = payload.output.len(),
            db_bytes = db_output.len(),
            "using controller-side streaming output (more complete than agent payload)"
        );
        db_output
    } else if payload.output.len() > MAX_UPDATE_OUTPUT_BYTES {
        payload.output[..MAX_UPDATE_OUTPUT_BYTES].to_string()
    } else {
        payload.output
    };

    active.output = Set(final_output.clone());
    active.output_bytes = Set(final_output.len() as i64);
    if payload.from_version.is_some() {
        active.from_version = Set(payload.from_version);
    }
    if let Err(e) = active.update(state.db()).await {
        tracing::warn!(
            error = %e,
            "failed to update update_history result"
        );
    }

    // Notify SSE subscribers that this update has completed.
    {
        let status_str = match payload.status {
            UpdateFinalStatus::Completed => "completed",
            UpdateFinalStatus::Failed => "failed",
            _ => "failed",
        };
        state
            .update_output_broadcaster
            .send_completed(
                payload.update_history_id,
                status_str.to_string(),
                payload.error.clone(),
            )
            .await;
    }

    if let Err(e) = update_output_line::Entity::delete_many()
        .filter(update_output_line::Column::UpdateHistoryId.eq(payload.update_history_id))
        .exec(state.db())
        .await
    {
        tracing::warn!(
            error = %e,
            "failed to clear update output lines"
        );
    }

    if payload.status == UpdateFinalStatus::Completed
        && let Some(ref to_version) = payload.to_version
        && let Ok(Some(link)) =
            host_software_item::Entity::find_by_id((record.host_id, record.software_item_id))
                .one(state.db())
                .await
    {
        let mut link_active: host_software_item::ActiveModel = link.into();
        link_active.installed_version = Set(Some(to_version.clone()));
        link_active.installed_version_detected_at = Set(Some(time::OffsetDateTime::now_utc()));
        link_active.last_updated_at = Set(Some(time::OffsetDateTime::now_utc()));
        if let Err(e) = link_active.update(state.db()).await {
            tracing::warn!(
                error = %e,
                "failed to update host_software_item installed_version"
            );
        }
    }

    // Push updated software states to MQTT services.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    // If this update is part of a batch, emit batch progress events and
    // dispatch the next pending update for this host.
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
    }

    // Dispatch notification event for update result.
    {
        // Look up names for the notification message.
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
            let details = match payload.status {
                UpdateFinalStatus::Completed => NotificationEventDetails::UpdateCompleted {
                    from_version: record.from_version.clone(),
                    to_version: payload
                        .to_version
                        .clone()
                        .unwrap_or_else(|| record.to_version.clone()),
                    update_history_id: payload.update_history_id,
                },
                _ => NotificationEventDetails::UpdateFailed {
                    from_version: record.from_version.clone(),
                    to_version: payload
                        .to_version
                        .clone()
                        .unwrap_or_else(|| record.to_version.clone()),
                    error: payload.error.clone(),
                    update_history_id: payload.update_history_id,
                },
            };

            state.notification_dispatcher.dispatch(NotificationEvent {
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

    LoopAction::Continue
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
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    match crate::queries::update_batches::dispatch_next_in_batch(
        state.db(),
        &state.notification_service,
        batch_id,
        host_id,
        tenant_id,
    )
    .await
    {
        Ok(Some(completion)) => {
            use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
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

            state.notification_dispatcher.dispatch(NotificationEvent {
                tenant_id: completion.tenant_id,
                host_id: None,
                host_name: None,
                software_item_id: None,
                software_item_name: None,
                plugin_type: None,
                details,
            });
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

// ---------------------------------------------------------------------------
// Batch progress helpers
// ---------------------------------------------------------------------------

/// Send a batch progress event to all SSE subscribers.
async fn emit_batch_progress_event(
    state: &Arc<AppState>,
    batch_id: uuid::Uuid,
    event: crate::batch_progress_broadcaster::BatchProgressEvent,
) {
    state.batch_progress_broadcaster.send(batch_id, event).await;
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
// handle_batch_host_package_update_result
// ---------------------------------------------------------------------------

/// Handle a `BatchHostPackageUpdateResult` message: update per-package
/// `host_package_update_history` rows and `host_package.installed_version`
/// for successful packages.
#[tracing::instrument(skip_all, fields(%service_id, batch_id = %payload.batch_id))]
pub(super) async fn handle_batch_host_package_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: BatchHostPackageUpdateResultPayload,
    linked_host_ids: &HashSet<uuid::Uuid>,
) -> LoopAction {
    tracing::info!(
        batch_id = %payload.batch_id,
        results = payload.results.len(),
        "batch host package update result"
    );

    let now = time::OffsetDateTime::now_utc();

    for result in &payload.results {
        // Validate ownership: the host_package_update_history record must
        // belong to a host linked to this service.
        let history_record =
            match host_package_update_history::Entity::find_by_id(result.update_history_id)
                .one(state.db())
                .await
            {
                Ok(Some(r)) => r,
                Ok(None) => {
                    tracing::warn!(
                        update_history_id = %result.update_history_id,
                        "host_package_update_history record not found"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        update_history_id = %result.update_history_id,
                        "failed to look up host_package_update_history"
                    );
                    continue;
                }
            };

        if !linked_host_ids.contains(&history_record.host_id) {
            tracing::warn!(
                %service_id,
                update_history_id = %result.update_history_id,
                host_id = %history_record.host_id,
                "service attempted to update host_package_update_history for unlinked host"
            );
            continue;
        }

        // Update the history record.
        let status_str = match result.status {
            UpdateFinalStatus::Completed => "completed",
            UpdateFinalStatus::Failed => "failed",
            _ => "failed",
        };
        let mut active: host_package_update_history::ActiveModel = history_record.into();
        active.status = Set(status_str.to_string());
        active.output = Set(if result.output.is_empty() {
            None
        } else {
            Some(result.output.clone())
        });
        active.output_bytes = Set(result.output.len() as i64);
        active.completed_at = Set(Some(now));
        if let Some(ref error) = result.error {
            // Store error in output if no other output.
            if result.output.is_empty() {
                active.output = Set(Some(error.clone()));
                active.output_bytes = Set(error.len() as i64);
            }
        }
        if let Err(e) = active.update(state.db()).await {
            tracing::warn!(
                error = %e,
                update_history_id = %result.update_history_id,
                "failed to update host_package_update_history"
            );
        }

        // On success, update host_package.installed_version.
        if result.status == UpdateFinalStatus::Completed {
            match host_package::Entity::find_by_id(result.host_package_id)
                .one(state.db())
                .await
            {
                Ok(Some(hp)) => {
                    let mut hp_active: host_package::ActiveModel = hp.into();
                    if let Some(ref new_version) = result.installed_version {
                        hp_active.installed_version = Set(Some(new_version.clone()));
                        hp_active.installed_version_detected_at = Set(Some(now));
                    }
                    hp_active.last_updated_at = Set(Some(now));
                    hp_active.updated_at = Set(now);
                    if let Err(e) = hp_active.update(state.db()).await {
                        tracing::warn!(
                            error = %e,
                            host_package_id = %result.host_package_id,
                            "failed to update host_package installed_version"
                        );
                    }
                }
                Ok(None) => {
                    tracing::debug!(
                        host_package_id = %result.host_package_id,
                        "host_package not found for batch update result"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        host_package_id = %result.host_package_id,
                        "failed to look up host_package"
                    );
                }
            }
        }
    }

    // Push updated host-package states to MQTT so that `in_progress = false`
    // and the new `installed_version` are reflected immediately after the batch
    // completes — mirroring what `handle_update_result` does for software items.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// Role-based plugin helpers
// ---------------------------------------------------------------------------

/// Build a [`PluginAssignment`] from a role plugin row and its plugin config.
///
/// Returns `None` if the plugin type string cannot be deserialized.
fn build_plugin_assignment(
    assignment: &host_software_item_plugin::Model,
    config: &plugin_config::Model,
) -> Option<PluginAssignment> {
    let plugin_type: uptrakit_internal_wire::PluginType =
        serde_json::from_value(serde_json::Value::String(config.plugin_type.clone())).ok()?;

    let merged_config =
        uptrakit_update_hooks::merge_config(&config.config, assignment.config_override.as_ref());

    Some(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}
