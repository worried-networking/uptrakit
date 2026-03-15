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
    BatchUpdateResultPayload, ControllerMessage, ExecuteUpdatePayload, OutgoingSeq,
    OutputStreamType, PluginAssignment, UpdateFinalStatus, UpdateOutputPayload,
    UpdateResultPayload, UpdateStartedPayload,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, service, service_host,
    software_item, update_history, update_output_line,
};

use super::messages::ProcessorResponse;
use super::{HandlerError, HandlerResult, MAX_UPDATE_OUTPUT_BYTES};
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::routes::service_ws::protocol::serialize_controller_msg;
use uptrakit_web_api_types::events::AdminEvent;

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

    // 1b. Fail any in-progress records from a previous agent session so they
    //     don't stay stuck forever. Any newly-queued follow-up dispatches run
    //     before the pending query below so promoted items are included.
    fail_in_progress_on_reconnect(state, service_id, &host_ids).await;

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
        .filter_map(|a| a.plugin_config_id)
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
        let exec_config = exec_assignment
            .plugin_config_id
            .and_then(|pc_id| configs_map.get(&pc_id));

        let execute_update_plugin =
            match build_plugin_assignment_nullable(exec_assignment, exec_config) {
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
        let detect_version_plugin = assignments_map.get(&detect_key).and_then(|a| {
            let c = a.plugin_config_id.and_then(|pc_id| configs_map.get(&pc_id));
            build_plugin_assignment_nullable(a, c)
        });

        // Resolve hooks from the execute_update plugin config + per-role override.
        let resolved_hooks = uptrakit_update_hooks::resolve_hooks(
            exec_config
                .map(|c| &c.config)
                .unwrap_or(&serde_json::Value::Object(Default::default())),
            exec_assignment.config.as_ref(),
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
            .and_then(|a| a.plugin_config_id)
            .and_then(|pc_id| configs_map.get(&pc_id))
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
            to_version: update_record.to_version.clone().unwrap_or_default(),
            detect_version_plugin,
            execute_update_plugin,
            pre_update_hooks: resolved_hooks.pre_update_hooks,
            post_update_hooks: resolved_hooks.post_update_hooks,
            release_info,
            timeout: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT,
            interactive: false,
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
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
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
        Err(_) => return ProcessorResponse::cont(),
    };
    let record_batch_id = record.batch_id;
    let record_host_id = record.host_id;
    let record_software_item_id = record.software_item_id;
    let record_tenant_id = record.tenant_id;
    let mut active: update_history::ActiveModel = record.into();
    active.status = Set(update_history::UpdateStatus::InProgress);
    active.started_at = Set(Some(time::OffsetDateTime::now_utc()));
    if payload.from_version.is_some() {
        active.from_version = Set(payload.from_version.clone());
    }
    active.output = Set(String::new());
    active.output_bytes = Set(0);
    active.interactive = Set(payload.interactive);
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

    // Broadcast AdminEvent::UpdateStarted so the history-list SSE subscribers
    // can update the "Input Required" badge in real-time without reloading.
    state
        .event_broadcaster
        .send(
            record_tenant_id,
            AdminEvent::UpdateStarted {
                update_history_id: payload.update_history_id,
                host_id: record_host_id,
                software_item_id: record_software_item_id,
                interactive: payload.interactive,
            },
        )
        .await;

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

    ProcessorResponse::cont()
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
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::trace!(
        update_id = %payload.update_history_id,
        stream = ?payload.stream,
        "update output"
    );
    if validate_update_ownership(
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

    let output_line = payload.output.clone();
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
        return ProcessorResponse::cont();
    };

    if updated.rows_affected == 0 {
        // Cap exceeded — mark the first truncation atomically and, if this is
        // the first time, emit a visible system notice into the output stream.
        let mark_result = update_history::Entity::update_many()
            .col_expr(update_history::Column::OutputTruncated, Expr::value(true))
            .filter(update_history::Column::Id.eq(payload.update_history_id))
            .filter(update_history::Column::OutputTruncated.eq(false))
            .exec(state.db())
            .await;

        match mark_result {
            Ok(r) if r.rows_affected == 1 => {
                // First truncation — insert and broadcast a system notice line.
                tracing::warn!(
                    update_id = %payload.update_history_id,
                    cap_bytes = MAX_UPDATE_OUTPUT_BYTES,
                    "update output exceeded cap — truncation notice emitted"
                );
                let notice_text = "\n[Output truncated: this update produced more than 50 MB of \
                    output. Only the first 50 MB is stored.]\n"
                    .to_string();
                let notice_id = uuid::Uuid::now_v7();
                let notice_ts = time::OffsetDateTime::now_utc();
                let notice_line = update_output_line::ActiveModel {
                    id: Set(notice_id),
                    update_history_id: Set(payload.update_history_id),
                    stream: Set(OutputStreamType::System),
                    output: Set(notice_text.clone()),
                    created_at: Set(notice_ts),
                };
                if let Err(e) = update_output_line::Entity::insert(notice_line)
                    .exec(state.db())
                    .await
                {
                    tracing::warn!(error = %e, "failed to insert truncation notice line");
                }
                state
                    .update_output_broadcaster
                    .send_line(
                        payload.update_history_id,
                        notice_id,
                        notice_text,
                        OutputStreamType::System,
                        notice_ts,
                    )
                    .await;
            }
            Ok(_) => {
                // Already truncated — quiet drop.
                tracing::trace!(
                    update_id = %payload.update_history_id,
                    "update output exceeded cap, dropping"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    update_id = %payload.update_history_id,
                    "failed to mark output_truncated"
                );
            }
        }

        return ProcessorResponse::cont();
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

/// Map [`UpdateFinalStatus`] to the DB enum.
fn final_status_to_db(status: &UpdateFinalStatus) -> update_history::UpdateStatus {
    match status {
        UpdateFinalStatus::Completed => update_history::UpdateStatus::Completed,
        _ => update_history::UpdateStatus::Failed,
    }
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
        (agent_output[..MAX_UPDATE_OUTPUT_BYTES].to_string(), true)
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

/// Handle an `UpdateResult` message: validate ownership, set final status,
/// store output, update installed version on success, push software states.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id, status = ?payload.status))]
pub(super) async fn handle_update_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: UpdateResultPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
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
        Err(_) => return ProcessorResponse::cont(),
    };

    // Persist final status and output.
    let mut active: update_history::ActiveModel = record.clone().into();
    active.status = Set(final_status_to_db(&payload.status));
    active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));

    let (final_output, agent_truncated) =
        select_best_output(state, payload.update_history_id, payload.output.clone()).await;

    active.output = Set(final_output.clone());
    active.output_bytes = Set(final_output.len() as i64);
    if agent_truncated {
        active.output_truncated = Set(true);
    }
    if payload.from_version.is_some() {
        active.from_version = Set(payload.from_version.clone());
    }
    if let Err(e) = active.update(state.db()).await {
        tracing::warn!(error = %e, "failed to update update_history result");
    }

    // Notify SSE subscribers and clean up streaming output lines.
    state
        .update_output_broadcaster
        .send_completed(
            payload.update_history_id,
            final_status_str(&payload.status).to_string(),
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

/// Mark any `InProgress` updates for these hosts as `Failed`, close their SSE
/// streams, broadcast `UpdateCompleted` events, and dispatch follow-up updates.
///
/// Called at the start of `deliver_pending_updates` so that orphaned
/// in-progress records from a previous agent session are resolved before we
/// attempt to re-deliver pending items.
async fn fail_in_progress_on_reconnect(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    host_ids: &[uuid::Uuid],
) {
    let records = match crate::queries::update_batches::mark_in_progress_as_failed(
        state.db(),
        host_ids,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "failed to mark in-progress updates as failed on reconnect"
            );
            return;
        }
    };

    if records.is_empty() {
        return;
    }

    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    let reason = "Update interrupted: agent restarted".to_string();

    for record in &records {
        notify_failed_reconnect_update(state, service_id, tenant_id, record, &reason).await;
    }

    state
        .notification_service
        .push_software_states_for_tenant(state.db(), tenant_id)
        .await;
}

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
) {
    tracing::warn!(
        update_id = %record.id,
        host_id = %record.host_id,
        "in-progress update marked failed due to agent restart"
    );

    state
        .update_output_broadcaster
        .send_completed(record.id, "failed".to_string(), Some(reason.to_string()))
        .await;

    state
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

    if let Some(batch_id) = record.batch_id {
        dispatch_next_batch_update(state, service_id, batch_id, record.host_id).await;
    } else {
        dispatch_next_queued_update(state, service_id, record.host_id).await;
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
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        _ => return,
    };

    if let Err(e) = crate::queries::update_batches::dispatch_next_queued_for_host(
        state.db(),
        &state.notification_service,
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
// handle_batch_update_result
// ---------------------------------------------------------------------------

/// Process a single item result within a batch: validate ownership, persist
/// status/output, and update the installed version on success.
async fn process_single_batch_result(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    result: &uptrakit_internal_wire::BatchUpdateItemResult,
    linked_host_ids: &HashSet<uuid::Uuid>,
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

    // Persist status and output.
    let mut active: update_history::ActiveModel = history_record.into();
    active.status = Set(final_status_to_db(&result.status));
    active.output = Set(if result.output.is_empty() {
        String::new()
    } else {
        result.output.clone()
    });
    active.output_bytes = Set(result.output.len() as i64);
    active.completed_at = Set(Some(time::OffsetDateTime::now_utc()));
    if let Some(ref error) = result.error
        && result.output.is_empty()
    {
        active.output = Set(error.clone());
        active.output_bytes = Set(error.len() as i64);
    }
    if let Err(e) = active.update(state.db()).await {
        tracing::warn!(
            error = %e,
            update_history_id = %result.update_history_id,
            "failed to update update_history"
        );
    }

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
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    tracing::info!(
        batch_id = %payload.batch_id,
        results = payload.results.len(),
        "batch update result"
    );

    for result in &payload.results {
        process_single_batch_result(state, service_id, result, &linked_host_ids).await;
    }

    // Push updated software states to MQTT so that `in_progress = false`
    // and the new `installed_version` are reflected immediately after the batch
    // completes.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
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
fn build_plugin_assignment_nullable(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> Option<PluginAssignment> {
    let plugin_type_str = config
        .map(|c| c.plugin_type.clone())
        .unwrap_or_else(|| assignment.plugin_type.clone());
    let plugin_type: uptrakit_internal_wire::PluginType =
        serde_json::from_value(serde_json::Value::String(plugin_type_str)).ok()?;

    let merged_config = uptrakit_update_hooks::resolve_effective_config(
        None,
        config.map(|c| &c.config),
        assignment.config.as_ref(),
    );

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
) -> ProcessorResponse {
    let linked_host_ids = linked_host_ids.lock().clone();
    // Validate that this service owns the update
    if let Err(e) = validate_update_ownership(
        &state.db,
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

    state
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

        state
            .notification_dispatcher
            .dispatch(crate::notifications::events::NotificationEvent {
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
            });
    }

    tracing::debug!(
        hint = ?payload.hint,
        "broadcast StdinAttention for update"
    );
    ProcessorResponse::cont()
}
