//! Shared audit infrastructure for software-item route handlers.
//!
//! All items here are `pub(super)` — consumed by handlers in the facade (`mod.rs`)
//! and later by sibling submodules (`merge`, `version_check`, `batch`) via
//! `super::audit::`.

use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
use uuid::Uuid;

pub(super) const SOFTWARE_ITEM_CREATE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_CREATE;
pub(super) const SOFTWARE_ITEM_UPDATE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE;
pub(super) const SOFTWARE_ITEM_DELETE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_DELETE;
pub(super) const SOFTWARE_ITEM_APPROVE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_APPROVE;
pub(super) const SOFTWARE_ITEM_ASSIGN_HOSTS_AUDIT_ACTION:
    uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_ASSIGN_HOSTS;
pub(super) const SOFTWARE_ITEM_UNASSIGN_HOST_AUDIT_ACTION:
    uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UNASSIGN_HOST;
pub(super) const SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT_AUDIT_ACTION:
    uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_UPDATE_HOST_ASSIGNMENT;
pub(super) const SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT_AUDIT_ACTION:
    uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_DELETE_PLUGIN_ASSIGNMENT;
pub(super) const SOFTWARE_ITEM_MERGE_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_MERGE;
pub(super) const SOFTWARE_ITEM_BATCH_AUDIT_ACTION: uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_BATCH;
pub(super) const SOFTWARE_VERSION_CHECK_TRIGGERED_AUDIT_ACTION:
    uptrakit_audit_log::RegisteredAuditAction =
    uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_TRIGGERED;

pub(super) struct AuditContext<'a> {
    pub(super) audit_emitter: &'a uptrakit_audit_log::AuditEmitter,
    pub(super) tenant_id: Uuid,
    pub(super) user: &'a AuthenticatedUser,
    pub(super) api_token_id: Option<AuthenticatedApiTokenId>,
}

pub(super) fn emit_software_item_mutation_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_id: String,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);
    let entry = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(ctx.tenant_id)
        .actor(actor_type, actor_id)
        .target("software_item", target_id, target_display)
        .outcome(outcome)
        .details(details)
        .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_event(entry);
    }
}

pub(super) fn emit_software_version_check_audit(
    ctx: &AuditContext<'_>,
    item_id: Uuid,
    item_name: Option<&str>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);
    let entry =
        uptrakit_audit_log::AuditEntry::builder(SOFTWARE_VERSION_CHECK_TRIGGERED_AUDIT_ACTION)
            .tenant_scope(ctx.tenant_id)
            .actor(actor_type, actor_id)
            .target(
                "software_item",
                item_id.to_string(),
                item_name.map(str::to_string),
            )
            .outcome(outcome)
            .details(details)
            .build();

    if let Ok(entry) = entry {
        ctx.audit_emitter.emit_event(entry);
    }
}
