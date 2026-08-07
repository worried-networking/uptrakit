use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use sea_orm::EntityTrait;
use serde::Serialize;
use uptrakit_shared_db::entity::prelude::User;
use uptrakit_web_api_types::oauth::McpScope;

use crate::context::McpRequestContext;
use crate::oauth::tool_auth::{ToolAuth, require_tool_auth};
use crate::tools::{McpHandler, mcp_error};

// No catalog actions required beyond the connection-level `mcp:use` gate
// (`McpAuthLayer`, `auth.rs`): any authenticated MCP principal may inspect
// its own identity.
pub(crate) const GET_CURRENT_USER_AUTH: ToolAuth = ToolAuth {
    required_scopes: &[McpScope::Read],
    required_actions: &[],
};

/// Result returned by the `get_current_user` MCP tool.
#[derive(Debug, Serialize, JsonSchema)]
pub struct GetCurrentUserResult {
    /// UUID of the authenticated user.
    pub user_id: String,
    /// Email address of the authenticated user.
    pub email: String,
    /// First name.
    pub first_name: String,
    /// Last name.
    pub last_name: String,
    /// Concrete actions the caller may perform — catalog-expanded plus
    /// live-registered dynamic `plugin.*`/`surface.*` actions, derived via
    /// `AccessEngine::allowed_actions` (single derivation shared with `me`).
    pub actions: Vec<String>,
}

impl McpHandler {
    /// Core logic for `get_current_user`; called by the `#[tool]` wrapper in `mod.rs`.
    pub(crate) async fn get_current_user_impl(
        &self,
        ctx: McpRequestContext,
    ) -> Result<Json<GetCurrentUserResult>, ErrorData> {
        require_tool_auth(&self.state, &ctx, &GET_CURRENT_USER_AUTH)?;

        let user = User::find_by_id(ctx.user_id)
            .one(self.state.db.db())
            .await
            .map_err(|e| mcp_error(format!("database error: {e}")))?
            .ok_or_else(|| mcp_error("authenticated user not found in database"))?;

        let actions: Vec<String> = self
            .state
            .access_engine
            .allowed_actions(&ctx.access)
            .iter()
            .map(ToString::to_string)
            .collect();

        Ok(Json(GetCurrentUserResult {
            user_id: user.id.to_string(),
            email: user.email.expose_email().to_owned(),
            first_name: user.first_name,
            last_name: user.last_name,
            actions,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn get_current_user_result_serialises() {
        let result = GetCurrentUserResult {
            user_id: Uuid::nil().to_string(),
            email: "test@example.com".to_owned(),
            first_name: "Alice".to_owned(),
            last_name: "Smith".to_owned(),
            actions: vec!["mcp:use".to_owned()],
        };
        let json = serde_json::to_string(&result).expect("serialisation must succeed");
        assert!(
            json.contains("test@example.com"),
            "email should appear in JSON"
        );
    }
}
