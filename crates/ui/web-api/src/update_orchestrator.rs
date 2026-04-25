//! Background orchestration of pre-update protection and agent dispatch.

use std::sync::Arc;

use time::OffsetDateTime;
use tokio::sync::mpsc;
use uptrakit_shared_types::OutputStreamType;
use uuid::Uuid;

use crate::AppState;
use crate::queries::update_dispatch::{
    DispatchUpdateParams, PreUpdateProtectionOutcome, dispatch_update_to_agent,
    fail_before_agent_dispatch, insert_protection_output_line, prepare_pre_update_protection,
    set_inprogress_for_orchestrator,
};
use crate::queries::update_triggers::PendingProtectionWork;
use uptrakit_web_api_types::events::AdminEvent;

/// Spawn a background task that runs pre-update protection then dispatches to
/// the agent. The caller passes the bundle returned by `trigger_update_for_host`
/// for `Pending` records.
pub fn spawn_protection_and_dispatch(state: Arc<AppState>, work: PendingProtectionWork) {
    tokio::spawn(run_protection_and_dispatch(state, work));
}

#[tracing::instrument(skip_all, fields(update_id = %work.update_history_id))]
async fn run_protection_and_dispatch(state: Arc<AppState>, work: PendingProtectionWork) {
    let update_history_id = work.update_history_id;
    let tenant_id = work.target.item.tenant_id;
    let host_id = work.target.host.id;
    let software_item_id = work.target.item.id;

    // 1. Check agent connectivity — if the agent is offline there is nothing to
    //    dispatch yet. Leave the record as Pending for reconnect recovery.
    if !state
        .service_connections
        .is_connected(&work.target.agent.id)
        .await
    {
        tracing::debug!(
            update_id = %update_history_id,
            agent_id = %work.target.agent.id,
            "agent offline at orchestration time — leaving record Pending for reconnect recovery"
        );
        return;
    }

    // 2. CAS Pending → InProgress. If the row was already claimed by another
    //    controller or is no longer Pending, bail out silently.
    let rows = match set_inprogress_for_orchestrator(state.db(), update_history_id).await {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(
                update_id = %update_history_id,
                error = %e,
                "set_inprogress_for_orchestrator query failed"
            );
            return;
        }
    };

    if rows == 0 {
        tracing::debug!(
            update_id = %update_history_id,
            "CAS Pending→InProgress matched 0 rows — record already claimed or gone"
        );
        return;
    }

    // 3. Create the broadcast channel for real-time output streaming.
    state
        .broadcast
        .update_output_broadcaster
        .create_channel(update_history_id)
        .await;

    // 4. Push MQTT software states so the UI transitions to in_progress.
    state
        .notification
        .notification_service
        .push_software_states_for_tenant(state.db(), tenant_id)
        .await;

    // 5. Emit AdminEvent::UpdateProtectionStarted for SSE subscribers.
    state
        .notification
        .event_broadcaster
        .send(
            tenant_id,
            AdminEvent::UpdateProtectionStarted {
                update_history_id,
                host_id,
                software_item_id,
            },
        )
        .await;

    // 6. Create mpsc channel for streaming protection output lines.
    let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();

    // 7. Spawn forwarder task.
    let db_for_fwd = state.db().clone();
    let broadcaster_for_fwd = state.broadcast.update_output_broadcaster.clone();
    tokio::spawn(forward_protection_output(
        db_for_fwd,
        broadcaster_for_fwd,
        update_history_id,
        rx,
    ));

    // 8. Run pre-update protection.
    let protection = state.controller_update_protection();
    let db = state.db().clone();
    let outcome = match prepare_pre_update_protection(
        &db,
        protection,
        &work.target,
        update_history_id,
        Some(tx),
    )
    .await
    {
        Ok(outcome) => outcome,
        Err(e) => {
            tracing::warn!(
                update_id = %update_history_id,
                error = %e,
                "prepare_pre_update_protection returned an error"
            );
            state
                .notification
                .notification_service
                .push_software_states_for_tenant(state.db(), tenant_id)
                .await;
            return;
        }
    };

    // 9. Handle outcome.
    match outcome {
        PreUpdateProtectionOutcome::Failed => {
            // Protection failed — record already marked Failed by the query.
            state
                .notification
                .notification_service
                .push_software_states_for_tenant(state.db(), tenant_id)
                .await;
        }
        PreUpdateProtectionOutcome::Proceed => {
            // 10. Dispatch to agent.
            let notifier = &state.notification.notification_service;
            let dispatch_result = dispatch_update_to_agent(
                notifier,
                &work.target,
                DispatchUpdateParams {
                    update_history_id,
                    to_version: work.to_version,
                    release_info: work.release_info,
                    interactive: work.interactive,
                },
            )
            .await;

            match dispatch_result {
                Ok(true) => {
                    // Agent was connected and the message was sent.
                }
                Ok(false) => {
                    // Agent disconnected between the connectivity check and
                    // dispatch. Leave the record in InProgress for the
                    // reconnect recovery path.
                    tracing::debug!(
                        update_id = %update_history_id,
                        agent_id = %work.target.agent.id,
                        "agent disconnected before dispatch — leaving InProgress for reconnect recovery"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        update_id = %update_history_id,
                        error = %e,
                        "dispatch_update_to_agent failed"
                    );
                    if let Err(fail_err) =
                        fail_before_agent_dispatch(&db, update_history_id, None).await
                    {
                        tracing::warn!(
                            update_id = %update_history_id,
                            error = %fail_err,
                            "fail_before_agent_dispatch also failed after dispatch error"
                        );
                    }
                    state
                        .notification
                        .notification_service
                        .push_software_states_for_tenant(state.db(), tenant_id)
                        .await;
                }
            }
        }
    }
}

async fn forward_protection_output(
    db: sea_orm::DatabaseConnection,
    broadcaster: crate::update_output_broadcaster::UpdateOutputBroadcaster,
    update_history_id: Uuid,
    mut rx: mpsc::UnboundedReceiver<Vec<u8>>,
) {
    while let Some(raw) = rx.recv().await {
        let text = String::from_utf8_lossy(&raw).into_owned();
        let line_id = Uuid::now_v7();
        let timestamp = OffsetDateTime::now_utc();

        if let Err(e) = insert_protection_output_line(
            &db,
            update_history_id,
            line_id,
            text.clone(),
            OutputStreamType::Stdout,
            timestamp,
        )
        .await
        {
            tracing::warn!(
                update_id = %update_history_id,
                error = %e,
                "failed to persist protection output line"
            );
        }

        broadcaster
            .send_line(
                update_history_id,
                line_id,
                text,
                OutputStreamType::Stdout,
                timestamp,
            )
            .await;
    }
}
