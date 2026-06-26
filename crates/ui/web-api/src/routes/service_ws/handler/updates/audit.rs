//! Update-lifecycle audit emission (semantic Audit V2 `emit_event` path).

use std::sync::Arc;

use uptrakit_shared_db::entity::update_history;
use uptrakit_wire::UpdateFinalStatus;

use super::final_status_str;
use super::lookups::{resolve_host_name, resolve_software_item_name};
use crate::AppState;

struct UpdateLifecycleAuditCtx<'a> {
    state: &'a Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
}

async fn emit_service_update_lifecycle_audit(
    ctx: &UpdateLifecycleAuditCtx<'_>,
    target_type: &'static str,
    target_id: uuid::Uuid,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    match uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
        ctx.action_type,
    )
    .tenant_scope(ctx.tenant_id)
    .actor_service(ctx.service_id)
    .target(target_type, target_id.to_string(), target_display)
    .outcome(outcome)
    .details(details)
    .build()
    {
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
        Err(error) => tracing::warn!(
            error = %error,
            service_id = %ctx.service_id,
            action_type = %ctx.action_type,
            "failed to build update lifecycle audit entry"
        ),
    }
}

/// Resolve the `(software_name, host_name, "<sw> on <host>")` triple used as
/// the audit `target_display` for update-lifecycle events.
async fn resolve_target_display(
    state: &Arc<AppState>,
    record: &update_history::Model,
) -> (String, String, String) {
    let software_name = resolve_software_item_name(state, record.software_item_id).await;
    let host_name = resolve_host_name(state, record.host_id).await;
    let display = format!("{software_name} on {host_name}");
    (software_name, host_name, display)
}

pub(super) async fn emit_update_finalized_audit(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    record: &update_history::Model,
    status: &UpdateFinalStatus,
    outcome: uptrakit_audit_log::AuditOutcome,
    output_truncated: bool,
    reason_code: Option<&'static str>,
) {
    let (_software_name, _host_name, target_display) = resolve_target_display(state, record).await;
    let mut details = serde_json::json!({
        "batch_id": record.batch_id,
        "dispatch_mode": if record.batch_id.is_some() { "batch" } else { "queued" },
        "host_id": record.host_id,
        "interactive": record.interactive,
        "output_truncated": output_truncated,
        "software_item_id": record.software_item_id,
        "status": final_status_str(status),
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    emit_service_update_lifecycle_audit(
        &UpdateLifecycleAuditCtx {
            state,
            service_id,
            tenant_id: record.tenant_id,
            action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_FINALIZED,
        },
        "update_history",
        record.id,
        Some(target_display),
        outcome,
        details,
    )
    .await;
}

pub(super) async fn emit_batch_update_finalized_audit(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    tenant_id: uuid::Uuid,
    batch_id: uuid::Uuid,
    summary: &super::BatchUpdateAuditSummary,
) {
    let mut details = serde_json::json!({
        "completed_count": summary.completed_count,
        "dispatch_mode": "batch",
        "failed_count": summary.failed_count,
        "finalize_error_count": summary.finalize_error_count,
        "result_count": summary.result_count,
        "stale_count": summary.stale_count,
    });
    if let Some(reason_code) = summary.reason_code() {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    emit_service_update_lifecycle_audit(
        &UpdateLifecycleAuditCtx {
            state,
            service_id,
            tenant_id,
            action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_BATCH_UPDATE_FINALIZED,
        },
        "batch_update",
        batch_id,
        None,
        summary.outcome(),
        details,
    )
    .await;
}

pub(super) async fn emit_stdin_attention_audit(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    record: &update_history::Model,
    hint: Option<&str>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let (_software_name, _host_name, target_display) = resolve_target_display(state, record).await;
    let mut details = serde_json::json!({
        "dispatch_mode": if record.batch_id.is_some() { "batch" } else { "queued" },
        "hint_length": hint.map(|value| value.len()),
        "hint_present": hint.is_some(),
        "host_id": record.host_id,
        "interactive": record.interactive,
        "software_item_id": record.software_item_id,
    });
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    emit_service_update_lifecycle_audit(
        &UpdateLifecycleAuditCtx {
            state,
            service_id,
            tenant_id: record.tenant_id,
            action_type: uptrakit_audit_log::AuditActionType::SOFTWARE_UPDATE_STDIN_ATTENTION,
        },
        "update_history",
        record.id,
        Some(target_display),
        outcome,
        details,
    )
    .await;
}
