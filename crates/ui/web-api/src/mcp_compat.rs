// crates/ui/web-api/src/mcp_compat.rs

use uuid::Uuid;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::{
    AuthFailure, authenticate_api_token, emit_api_token_auth_audit,
};

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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        assert_send_sync::<McpRequestContext>();
    }
}
