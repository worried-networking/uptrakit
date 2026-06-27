use std::sync::Arc;

use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::{
    emit_batch_progress_event, emit_batch_progress_from_db, handle_batch_completion,
    resolve_host_name, resolve_software_item_name,
};
use crate::AppState;

/// After an `AwaitingRestart` record transitions to `Completed` or `Failed`,
/// emit a per-item `BatchProgressEvent`, then promote the next queued update
/// for the same host (batch or standalone).  If the batch is now complete,
/// `handle_batch_completion` is called to emit the final summary and send
/// batch-completion notifications.
pub(super) async fn trigger_host_progression_after_awaiting_restart(
    state: &Arc<AppState>,
    hsi_id: uuid::Uuid,
) {
    use sea_orm::QueryOrder;
    use uptrakit_shared_db::entity::update_history;

    // Load the record just transitioned from AwaitingRestart.
    // Filter on awaiting_restart_since IS NOT NULL to avoid picking up
    // unrelated records that happened to end up Completed/Failed.
    let record = match update_history::Entity::find()
        .filter(update_history::Column::HostSoftwareItemId.eq(hsi_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Completed,
            update_history::UpdateStatus::Failed,
        ]))
        .filter(update_history::Column::AwaitingRestartSince.is_not_null())
        .order_by_desc(update_history::Column::CompletedAt)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                host_software_item_id = %hsi_id,
                "no Completed/Failed record found after AwaitingRestart transition"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                host_software_item_id = %hsi_id,
                "failed to load update_history for post-AwaitingRestart dispatch"
            );
            return;
        }
    };

    let dispatch = crate::queries::update_dispatch::DispatchContext {
        notifier: &state.notification.notification_service,
        protection: state.controller_update_protection(),
        #[cfg(feature = "plugin-ops")]
        hook: state.controller_update_hook(),
        #[cfg(feature = "plugin-ops")]
        notification_ops: Some(state.plugin.plugin_ops.as_ref()),
    };

    if let Some(batch_id) = record.batch_id {
        // Emit per-item progress event before dispatching next — mirrors
        // what handle_update_result does in updates.rs.
        let event = match record.status {
            update_history::UpdateStatus::Completed => {
                crate::batch_progress_broadcaster::BatchProgressEvent::UpdateCompleted {
                    update_history_id: record.id,
                    software_item_name: resolve_software_item_name(state, record.software_item_id)
                        .await,
                    host_name: resolve_host_name(state, record.host_id).await,
                }
            }
            _ => crate::batch_progress_broadcaster::BatchProgressEvent::UpdateFailed {
                update_history_id: record.id,
                software_item_name: resolve_software_item_name(state, record.software_item_id)
                    .await,
                host_name: resolve_host_name(state, record.host_id).await,
                // The error detail is not stored on the AwaitingRestart record itself.
                error: None,
            },
        };
        emit_batch_progress_event(state, batch_id, event).await;

        match crate::queries::update_batches::dispatch_next_in_batch(
            state.db(),
            dispatch,
            batch_id,
            record.host_id,
            record.tenant_id,
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
                    error = %e,
                    %batch_id,
                    host_id = %record.host_id,
                    "post-AwaitingRestart batch dispatch failed"
                );
            }
        }
    } else if let Err(e) = crate::queries::update_batches::dispatch_next_queued_for_host(
        state.db(),
        dispatch,
        record.host_id,
        record.tenant_id,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            host_id = %record.host_id,
            "post-AwaitingRestart standalone dispatch failed"
        );
    }
}
