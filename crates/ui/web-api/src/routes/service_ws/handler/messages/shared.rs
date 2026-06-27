use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use uptrakit_shared_db::entity::service;
use uptrakit_wire::OutgoingSeq;

use super::LoopAction;
use crate::AppState;
use crate::routes::service_ws::protocol::{
    record_service_activity, record_system_service_activity, send_pong,
};

pub(super) fn emit_service_inventory_audit(
    state: &AppState,
    service_model: &service::Model,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
    target: Option<(&str, String, Option<String>)>,
    details: serde_json::Value,
) {
    let mut builder =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
            .tenant_scope(service_model.tenant_id)
            .actor_service(service_model.id)
            .actor_display_opt(service_model.service_app_name.clone())
            .outcome(outcome)
            .details(details);
    if let Some((target_type, target_id, target_display)) = target {
        builder = builder.target(target_type, target_id, target_display);
    }
    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_event(entry),
        Err(error) => {
            tracing::warn!(
                service_id = %service_model.id,
                action_type = %action_type,
                error = %error,
                "failed to build service inventory audit entry"
            );
        }
    }
}

/// Handle a `Ping` message: send pong and record activity.
#[tracing::instrument(skip_all, fields(%service_id))]
pub(in super::super) async fn handle_ping(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    service_ts: i64,
    is_system: bool,
) -> LoopAction {
    let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else {
        return LoopAction::Break;
    };
    tracing::trace!(service_ts, controller_ts, "ping/pong");
    let activity_result = if is_system {
        record_system_service_activity(state.db(), service_id, None).await
    } else {
        record_service_activity(state.db(), service_id, None).await
    };
    if let Err(e) = activity_result {
        tracing::warn!(
            error = %e,
            %service_id,
            "failed to record service activity"
        );
    }
    LoopAction::Continue
}
