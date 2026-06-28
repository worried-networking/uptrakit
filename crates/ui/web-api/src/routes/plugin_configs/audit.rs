use super::command_safety::DangerousPatternMatch;
use super::command_safety::{collect_dangerous_patterns, detect_command_fields};
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

pub(super) fn emit_plugin_config_semantic_audit(
    ctx: &AuditContext<'_>,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    target_type: Option<&'static str>,
    target_id: Option<String>,
    target_display: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(ctx.user, ctx.api_token_id);

    let target_type = target_type.map(std::string::ToString::to_string);
    if let Ok(entry) =
        uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(action_type)
            .tenant_scope(ctx.tenant_id)
            .actor(actor_type, actor_id)
            .target_opt(target_type, target_id, target_display)
            .outcome(outcome)
            .details(details)
            .build()
    {
        ctx.audit_emitter.emit_event(entry);
    }
}

pub(super) fn dangerous_pattern_matches_to_json(
    matches: &[DangerousPatternMatch],
) -> serde_json::Value {
    serde_json::Value::Array(
        matches
            .iter()
            .map(|dangerous| {
                serde_json::json!({
                    "field": dangerous.field.clone(),
                    "description": dangerous.description,
                })
            })
            .collect(),
    )
}

#[derive(Default)]
pub(super) struct CommandRiskSummary {
    pub(super) command_fields: Vec<&'static str>,
    pub(super) dangerous_matches: Vec<DangerousPatternMatch>,
}

impl CommandRiskSummary {
    pub(super) fn from_config(config: &serde_json::Value) -> Self {
        Self {
            command_fields: detect_command_fields(config),
            dangerous_matches: collect_dangerous_patterns(config),
        }
    }

    pub(super) fn details_fragment(&self) -> serde_json::Value {
        serde_json::json!({
            "contains_command_fields": !self.command_fields.is_empty(),
            "command_fields": self.command_fields,
            "dangerous_command_match_count": self.dangerous_matches.len(),
            "dangerous_matches": dangerous_pattern_matches_to_json(&self.dangerous_matches),
        })
    }
}
