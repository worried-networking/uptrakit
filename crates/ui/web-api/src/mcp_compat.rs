// crates/ui/web-api/src/mcp_compat.rs

use uuid::Uuid;

use crate::auth::permissions::Permission;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + Clone>() {}

    #[test]
    fn mcp_request_context_is_clone_send_sync() {
        assert_send_sync::<McpRequestContext>();
    }
}
