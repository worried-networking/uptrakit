use std::fmt;

use uuid::Uuid;

use uptrakit_controller_core::auth::Permission;
use uptrakit_controller_core::update::UpdateDispatchError;

/// Per-request auth context injected into MCP request extensions by `McpAuthLayer`.
///
/// `#[non_exhaustive]`: OAuth 2.1 will add fields (scope claims, sub, etc.).
/// External code must use `McpRequestContext::new(…)`.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub struct McpRequestContext {
    pub user_id: Uuid,
    pub token_id: Uuid,
    pub tenant_id: Uuid,
    pub permissions: Vec<Permission>,
}

impl McpRequestContext {
    /// Creates a new [`McpRequestContext`].
    #[must_use]
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
/// `#[non_exhaustive]`: OAuth 2.1 will introduce new rejection cases.
#[non_exhaustive]
#[derive(Debug)]
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

/// Error variants for the MCP update-trigger tool.
///
/// NOT a wire type — converted to MCP tool error responses internally.
/// `#[non_exhaustive]`: future triggers may add variants (rate-limit, quota).
#[non_exhaustive]
#[derive(Debug)]
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

impl fmt::Display for McpTriggerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PermissionDenied => write!(f, "permission denied"),
            Self::HostNotFound => write!(f, "host not found"),
            Self::SoftwareItemNotFound => write!(f, "software item not found"),
            Self::NotConfigured => write!(f, "host not configured for updates"),
            Self::AgentUnavailable => write!(f, "agent unavailable"),
            Self::AlreadyInProgress => write!(f, "update already in progress"),
            Self::Internal => write!(f, "internal error"),
        }
    }
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
