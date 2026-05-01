// crates/ui/web-api/src/mcp_compat.rs

use std::sync::Arc;

use uuid::Uuid;

use uptrakit_web_api_types::software_items::TriggerUpdateStatus;

use crate::AppState;
use crate::auth::AuthMethod;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::{
    AuthFailure, AuthenticatedApiTokenId, AuthenticatedUser, authenticate_api_token,
    emit_api_token_auth_audit,
};
use crate::queries::update_triggers::TriggerUpdateParams;
use crate::queries::update_types::ActorType;
use uptrakit_web_api_queries::queries::update_dispatch::TriggerUpdateError;

/// Per-request auth context injected into MCP request extensions by `McpAuthLayer`.
///
/// Tool handlers extract this from request extensions:
/// `parts.extensions.get::<McpRequestContext>()`.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (scope claims, sub, etc.).
/// External code must use `McpRequestContext::new(...)`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
}

impl McpRequestContext {
    pub fn new(
        user_id: Uuid,
        token_id: Uuid,
        tenant_id: Uuid,
        permissions: Vec<Permission>,
    ) -> Self {
        Self {
            user_id,
            token_id,
            tenant_id,
            permissions,
        }
    }

    /// Returns `true` if the user holds `perm`.
    pub fn has_permission(&self, perm: &Permission) -> bool {
        self.permissions.contains(perm)
    }
}

/// Error variants for MCP authentication.
///
/// `#[non_exhaustive]`: OAuth 2.1 will introduce new rejection cases (e.g. scope mismatch).
#[derive(Debug)]
#[non_exhaustive]
pub enum McpAuthError {
    /// No `Authorization` header or empty bearer token.
    MissingCredentials,
    /// Token is present but not an `upk_`-prefixed API token (e.g. a JWT).
    JwtNotAccepted,
    /// API token is invalid, expired, or revoked.
    Unauthorized,
    /// User is deactivated or lacks the `AccessMcp` permission.
    Forbidden,
    /// Internal error during validation.
    Internal,
}

/// Validate a bearer token for an MCP request.
///
/// Accepts `None` (missing `Authorization` header) or `Some(token_str)`. Handles the
/// full auth path: missing token, JWT rejection, DB lookup, `AccessMcp` permission check,
/// and audit emission.
///
/// # TODO
///
/// Replace with OAuth 2.1 Resource Server / Authorization Server validation when that
/// feature lands. At that point `McpAuthLayer` drops this import and owns its own
/// validation logic.
pub async fn validate_api_token_for_mcp(
    state: &AppState,
    token: Option<&str>,
) -> Result<McpRequestContext, McpAuthError> {
    let token = match token {
        Some(t) if !t.is_empty() => t,
        _ => {
            emit_api_token_auth_audit(
                state,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                "missing_authorization_header",
            );
            return Err(McpAuthError::MissingCredentials);
        }
    };

    if !token.starts_with("upk_") {
        emit_api_token_auth_audit(
            state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            "jwt_not_accepted_for_mcp",
        );
        return Err(McpAuthError::JwtNotAccepted);
    }

    let (auth_user, token_id) = match authenticate_api_token(state, token).await {
        Ok(pair) => pair,
        Err(failure) => {
            if let Some(reason) = failure.api_token_reason_code() {
                emit_api_token_auth_audit(
                    state,
                    None,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    reason,
                );
            }
            return Err(match failure {
                AuthFailure::UserDeactivated => McpAuthError::Forbidden,
                AuthFailure::InternalError => McpAuthError::Internal,
                _ => McpAuthError::Unauthorized,
            });
        }
    };

    if !auth_user.has_permission(Permission::AccessMcp) {
        emit_api_token_auth_audit(
            state,
            None,
            uptrakit_audit_log::AuditOutcome::Denied,
            "missing_access_mcp_permission",
        );
        return Err(McpAuthError::Forbidden);
    }

    emit_api_token_auth_audit(
        state,
        None,
        uptrakit_audit_log::AuditOutcome::Success,
        "authenticated",
    );

    Ok(McpRequestContext::new(
        auth_user.user_id,
        token_id,
        state.default_tenant_id,
        auth_user.permissions,
    ))
}

/// Error variants for the MCP update-trigger bridge.
///
/// Maps the full `TriggerUpdateError` surface from `uptrakit-web-api-queries` so that
/// MCP tool handlers can produce meaningful protocol-level errors rather than collapsing
/// everything to a generic internal error.
///
/// `#[non_exhaustive]`: future triggers (rate-limit, quota) may add variants.
#[derive(Debug)]
#[non_exhaustive]
pub enum McpTriggerError {
    PermissionDenied,
    HostNotFound,
    SoftwareItemNotFound,
    /// Host exists but lacks assignment, plugin config, or a known plugin type.
    NotConfigured,
    /// Host has no linked agent or agent is not in Approved status.
    AgentUnavailable,
    AlreadyInProgress,
    Internal,
}

/// Trigger a software update for an MCP tool call.
///
/// Wraps `actions::software_items::trigger_update`, `update_orchestrator::spawn_protection_and_dispatch`,
/// and `routes::software_items::emit_software_update_audit` — all of which are `pub(crate)` in `web-api`.
///
/// Returns `(update_history_id, TriggerUpdateStatus)` using only types from shared workspace crates,
/// so `uptrakit-mcp` can call this without a circular dependency.
pub async fn mcp_trigger_update(
    state: Arc<AppState>,
    ctx: &McpRequestContext,
    host_id: Uuid,
    software_item_id: Uuid,
    to_version: String,
) -> Result<(Uuid, TriggerUpdateStatus), rootcause::Report<McpTriggerError>> {
    let actor_id_str = ctx.token_id.to_string();
    let tenant_db = crate::tenant_db::TenantDb(uptrakit_shared_db::TenantDb::new(
        state.db().clone(),
        ctx.tenant_id,
    ));
    let mut_ctx = state.mutation_context();

    let audit_user = AuthenticatedUser {
        user_id: ctx.user_id,
        auth_method: AuthMethod::ApiToken,
        permissions: ctx.permissions.clone(),
        jti: None,
    };
    let audit_token = AuthenticatedApiTokenId(ctx.token_id);

    let trigger_result = crate::actions::software_items::trigger_update(
        &tenant_db,
        &mut_ctx,
        TriggerUpdateParams {
            tenant_id: ctx.tenant_id,
            item_id: software_item_id,
            host_id,
            to_version: to_version.clone(),
            actor_type: ActorType::ApiToken.as_str(),
            actor_id: &actor_id_str,
            release_info: None,
            interactive: false,
        },
    )
    .await
    .map_err(|err| {
        let (outcome, reason_code) = err.current_context().trigger_audit_classification();
        crate::routes::software_items::emit_software_update_audit(
            &state,
            ctx.tenant_id,
            &audit_user,
            Some(audit_token),
            software_item_id,
            outcome,
            serde_json::json!({
                "host_id": host_id,
                "to_version": to_version,
                "interactive": false,
                "reason_code": reason_code,
            }),
        );
        let mcp_err = match err.current_context() {
            TriggerUpdateError::HostNotFound => McpTriggerError::HostNotFound,
            TriggerUpdateError::SoftwareItemNotFound => McpTriggerError::SoftwareItemNotFound,
            TriggerUpdateError::UpdateAlreadyActive => McpTriggerError::AlreadyInProgress,
            TriggerUpdateError::HostNotAssigned
            | TriggerUpdateError::NoExecuteUpdatePlugin
            | TriggerUpdateError::PluginConfigNotFound
            | TriggerUpdateError::UnknownPluginType(_) => McpTriggerError::NotConfigured,
            TriggerUpdateError::NoAgent | TriggerUpdateError::AgentNotApproved => {
                McpTriggerError::AgentUnavailable
            }
            _ => McpTriggerError::Internal,
        };
        rootcause::report!(mcp_err)
    })?;

    if let Some(work) = trigger_result.pending_protection_work {
        crate::update_orchestrator::spawn_protection_and_dispatch(Arc::clone(&state), *work);
    }

    let status = match trigger_result.initial_status {
        uptrakit_shared_db::entity::update_history::UpdateStatus::Pending => {
            TriggerUpdateStatus::Pending
        }
        uptrakit_shared_db::entity::update_history::UpdateStatus::Failed => {
            TriggerUpdateStatus::Failed
        }
        _ => TriggerUpdateStatus::Queued,
    };

    let audit_outcome = if matches!(status, TriggerUpdateStatus::Failed) {
        uptrakit_audit_log::AuditOutcome::Failed
    } else {
        uptrakit_audit_log::AuditOutcome::Success
    };

    crate::routes::software_items::emit_software_update_audit(
        &state,
        ctx.tenant_id,
        &audit_user,
        Some(audit_token),
        software_item_id,
        audit_outcome,
        serde_json::json!({
            "host_id": host_id,
            "to_version": to_version,
            "interactive": false,
            "update_history_id": trigger_result.update_history_id,
            "dispatch_status": status.to_string(),
        }),
    );

    Ok((trigger_result.update_history_id, status))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        assert_send_sync::<McpRequestContext>();
    }
}
