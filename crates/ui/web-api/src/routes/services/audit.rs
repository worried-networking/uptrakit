//! Service audit-emit helpers (fire-and-forget `emit_event`) + audit context.

use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use uuid::Uuid;

pub(super) struct AuditContext<'a> {
    pub(super) audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    pub(super) tenant_id: Uuid,
    pub(super) user: &'a AuthenticatedUser,
    pub(super) api_token_id: Option<AuthenticatedApiTokenId>,
}

pub(super) fn emit_service_lifecycle_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    service_id: Uuid,
    service_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .target("service", service_id.to_string(), service_display)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_event(entry);
    }
}

pub(super) fn emit_service_batch_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_event(entry);
    }
}

pub(super) fn batch_action_to_audit_action(
    action: &str,
) -> Option<uptrakit_audit_log::RegisteredAuditAction> {
    match action {
        "approve" => Some(uptrakit_audit_log::AuditActionType::SERVICE_APPROVE),
        "reject" => Some(uptrakit_audit_log::AuditActionType::SERVICE_REJECT),
        "deactivate" => Some(uptrakit_audit_log::AuditActionType::SERVICE_DEACTIVATE),
        _ => None,
    }
}
