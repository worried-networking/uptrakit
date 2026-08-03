use rmcp::handler::server::common::{AsRequestContext, FromContextPart};
use thiserror::Error;
use uuid::Uuid;

use uptrakit_controller_core::access::AccessContext;
use uptrakit_controller_core::update::UpdateDispatchError;
use uptrakit_web_api_types::oauth::McpScope;

/// Authentication method used for an MCP request.
///
/// `#[non_exhaustive]`: future auth schemes (e.g. mTLS) may add variants.
/// External match sites must include a wildcard arm.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum McpAuthMethod {
    /// Legacy `upk_`-prefixed API token; the default path today.
    ApiToken,
    /// OAuth 2.1 access token issued by the MCP authorization server.
    OAuth {
        /// `client_id` of the OAuth client that obtained the token.
        client_id: String,
        /// JWT ID (`jti`) of the access token, used for revocation lookups.
        jti: Uuid,
        /// Scopes granted on the access token.
        scopes: Vec<McpScope>,
    },
}

/// Per-request auth context injected into MCP request extensions by `McpAuthLayer`.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (scope claims, sub, etc.).
/// External code must use `McpRequestContext::new(…)`.
#[non_exhaustive]
#[derive(Clone)]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub access: AccessContext,
    pub auth_method: McpAuthMethod,
}

impl McpRequestContext {
    /// Creates a new [`McpRequestContext`].
    #[must_use]
    pub fn new(
        user_id: Uuid,
        token_id: Uuid,
        tenant_id: Uuid,
        access: AccessContext,
        auth_method: McpAuthMethod,
    ) -> Self {
        Self {
            user_id,
            token_id,
            tenant_id,
            access,
            auth_method,
        }
    }
}

// `AccessContext` is not `Debug` (the grant snapshot is internal); the manual
// impl keeps `McpRequestContext` debuggable for tracing without exposing it.
impl std::fmt::Debug for McpRequestContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpRequestContext")
            .field("user_id", &self.user_id)
            .field("token_id", &self.token_id)
            .field("tenant_id", &self.tenant_id)
            .field("auth_method", &self.auth_method)
            .finish_non_exhaustive()
    }
}

// Allow tool handlers to receive `McpRequestContext` as a plain parameter.
// rmcp injects `http::request::Parts` into its own extension map; the auth
// layer stored `McpRequestContext` in the HTTP request extensions, so we
// retrieve it from `parts.extensions`.
impl<C: AsRequestContext> FromContextPart<C> for McpRequestContext {
    fn from_context_part(context: &mut C) -> Result<Self, rmcp::ErrorData> {
        context
            .as_request_context()
            .extensions
            .get::<axum::http::request::Parts>()
            .ok_or_else(|| rmcp::ErrorData::invalid_params("missing http request parts", None))?
            .extensions
            .get::<McpRequestContext>()
            .cloned()
            .ok_or_else(|| rmcp::ErrorData::invalid_params("missing mcp request context", None))
    }
}

/// Error variants for MCP authentication.
///
/// `#[non_exhaustive]`: OAuth 2.1 will introduce new rejection cases.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Error)]
pub enum McpAuthError {
    /// No `Authorization` header or empty bearer token.
    #[error("missing credentials")]
    MissingCredentials,
    /// Token is present but not an `upk_`-prefixed API token (e.g. a JWT).
    #[error("JWT tokens not accepted for MCP")]
    JwtNotAccepted,
    /// API token is invalid, expired, or revoked.
    #[error("unauthorized")]
    Unauthorized,
    /// User is deactivated or lacks `mcp:use` access.
    #[error("forbidden")]
    Forbidden,
    /// Internal error during validation.
    #[error("internal error")]
    Internal,
}

/// Error variants for the MCP update-trigger tool.
///
/// NOT a wire type — converted to MCP tool error responses internally.
/// `#[non_exhaustive]`: future triggers may add variants (rate-limit, quota).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Error)]
pub enum McpTriggerError {
    #[error("permission denied")]
    PermissionDenied,
    #[error("host not found")]
    HostNotFound,
    #[error("software item not found")]
    SoftwareItemNotFound,
    /// Host exists but lacks assignment, plugin config, or a known plugin type.
    #[error("host not configured for updates")]
    NotConfigured,
    /// Host has no linked agent or agent is not in Approved status.
    #[error("agent unavailable")]
    AgentUnavailable,
    #[error("update already in progress")]
    AlreadyInProgress,
    #[error("internal error")]
    Internal,
}

impl From<&UpdateDispatchError> for McpTriggerError {
    fn from(e: &UpdateDispatchError) -> Self {
        match e {
            UpdateDispatchError::HostNotFound => Self::HostNotFound,
            UpdateDispatchError::SoftwareItemNotFound => Self::SoftwareItemNotFound,
            UpdateDispatchError::UpdateAlreadyActive => Self::AlreadyInProgress,
            UpdateDispatchError::NotConfigured => Self::NotConfigured,
            UpdateDispatchError::AgentUnavailable => Self::AgentUnavailable,
            UpdateDispatchError::Internal => Self::Internal,
            _ => {
                tracing::warn!(
                    "unhandled UpdateDispatchError variant; mapping to McpTriggerError::Internal"
                );
                Self::Internal
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_std_error<E: std::error::Error>() {}

    #[test]
    fn mcp_auth_error_implements_std_error() {
        assert_std_error::<McpAuthError>();
    }

    #[test]
    fn mcp_trigger_error_implements_std_error() {
        assert_std_error::<McpTriggerError>();
    }
}
