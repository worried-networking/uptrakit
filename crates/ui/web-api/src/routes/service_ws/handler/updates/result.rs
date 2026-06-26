//! `UpdateResult` message handling: finalize, output selection, side-effects.
#![expect(
    clippy::string_slice,
    reason = "slice index is at a validated char boundary"
)]

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use time::OffsetDateTime;

use uptrakit_shared_db::entity::{
    host, host_software_item, service, software_item, update_history, update_output_line,
};
use uptrakit_web_api_types::events::AdminEvent;
use uptrakit_wire::{UpdateFinalStatus, UpdateResultPayload};

use super::audit::emit_update_finalized_audit;
use super::finalize::finalize_post_update_best_effort;
use super::lookups::{resolve_host_name, resolve_software_item_name};
use super::{
    MAX_UPDATE_OUTPUT_BYTES, ProcessorResponse, dispatch_next_batch_update,
    dispatch_next_queued_update, emit_batch_progress_event, validate_host_link_visibility,
};
use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};

/// Map [`UpdateFinalStatus`] to a status string used by SSE events.
pub(super) fn final_status_str(status: &UpdateFinalStatus) -> &'static str {
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
pub(super) async fn select_best_output(
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

/// Set `host_software_item.installed_version` (+ detected_at, last_updated_at)
/// for rows matching `filter`. Shared by the standalone and batch result paths.
pub(super) async fn set_installed_version(
    state: &Arc<AppState>,
    filter: Condition,
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
        .filter(filter)
        .exec(state.db())
        .await
    {
        tracing::warn!(error = %e, "failed to update host_software_item installed_version");
    }
}

/// Update installed version for the standalone (host_id + software_item_id) path.
async fn update_installed_version_on_success(
    state: &Arc<AppState>,
    host_id: uuid::Uuid,
    software_item_id: uuid::Uuid,
    to_version: &str,
) {
    set_installed_version(
        state,
        Condition::all()
            .add(host_software_item::Column::HostId.eq(host_id))
            .add(host_software_item::Column::SoftwareItemId.eq(software_item_id)),
        to_version,
    )
    .await;
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

        {
            let mut event = NotificationEvent::new(svc.tenant_id, details);
            event.host_id = Some(record.host_id);
            event.host_name = host_name;
            event.software_item_id = Some(record.software_item_id);
            event.software_item_name = sw_name;
            state.notification.notification_dispatcher.dispatch(event);
        }
    }
}

/// Handle an `UpdateResult` message: validate ownership, set final status,
/// store output, update installed version on success, push software states.
#[tracing::instrument(skip_all, fields(%service_id, update_id = %payload.update_history_id, status = ?payload.status))]
pub(in super::super) async fn handle_update_result(
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

    // ── Resumable interception — BEFORE finalize_update_result_if_owned ──
    //
    // When the agent reports `Completed` with `resumable = Some(true)`, the
    // update is not yet terminal: it has produced an artifact that requires a
    // restart to take effect.  Transition `InProgress → AwaitingRestart` via
    // CAS (guarded by `execution_owner_service_id = service_id`) while the row
    // is still `InProgress`.  If the CAS wins, return early — SSE/MQTT/dispatch
    // will fire when the update reaches terminal status post-restart.  If the
    // CAS loses (row is not InProgress with this owner), fall through to normal
    // finalization so we don't silently discard a completion result we own.
    if matches!(final_status, UpdateFinalStatus::Completed) && payload.resumable == Some(true) {
        let rows = crate::queries::update_batches::transition_to_awaiting_restart(
            state.db(),
            payload.update_history_id,
            service_id,
        )
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                update_history_id = %payload.update_history_id,
                "transition_to_awaiting_restart failed"
            );
            0
        });
        if rows > 0 {
            emit_update_finalized_audit(
                state,
                service_id,
                &record,
                &final_status,
                uptrakit_audit_log::AuditOutcome::Partial,
                agent_truncated,
                Some("awaiting_restart"),
            )
            .await;
            return ProcessorResponse::cont();
        }
        tracing::warn!(
            update_history_id = %payload.update_history_id,
            "transition_to_awaiting_restart: CAS lost (rows_affected=0), falling through to finalization"
        );
    }

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
            emit_update_finalized_audit(
                state,
                service_id,
                &record,
                &final_status,
                uptrakit_audit_log::AuditOutcome::Failed,
                false,
                Some("finalization_error"),
            )
            .await;
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
                #[cfg(feature = "plugin-ops")]
                state.controller_update_hook(),
                #[cfg(feature = "plugin-ops")]
                Some(state.plugin.plugin_ops.as_ref()),
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
                    emit_update_finalized_audit(
                        state,
                        service_id,
                        &record,
                        &final_status,
                        uptrakit_audit_log::AuditOutcome::Denied,
                        false,
                        Some("not_owned"),
                    )
                    .await;
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
                    emit_update_finalized_audit(
                        state,
                        service_id,
                        &record,
                        &final_status,
                        uptrakit_audit_log::AuditOutcome::Failed,
                        false,
                        Some("pending_unowned_finalization_error"),
                    )
                    .await;
                    return ProcessorResponse::cont();
                }
            }
        } else {
            tracing::debug!(
                update_id = %payload.update_history_id,
                "ignoring stale UpdateResult from non-owner"
            );
            emit_update_finalized_audit(
                state,
                service_id,
                &record,
                &final_status,
                uptrakit_audit_log::AuditOutcome::Denied,
                false,
                Some("not_owned"),
            )
            .await;
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
        finalize_post_update_best_effort(state, &finalized_record, None).await;
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

    emit_update_finalized_audit(
        state,
        service_id,
        &record,
        &payload.status,
        if matches!(payload.status, UpdateFinalStatus::Completed) {
            uptrakit_audit_log::AuditOutcome::Success
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        },
        agent_truncated,
        if matches!(payload.status, UpdateFinalStatus::Completed) {
            None
        } else {
            Some("agent_reported_failure")
        },
    )
    .await;

    ProcessorResponse::cont()
}
