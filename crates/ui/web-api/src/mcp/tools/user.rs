use rmcp::{ErrorData, Json};
use schemars::JsonSchema;
use sea_orm::EntityTrait;
use serde::Serialize;
use uptrakit_shared_db::entity::prelude::User;

use crate::mcp::auth::McpRequestContext;
use crate::mcp::tools::{McpHandler, mcp_error};

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
    /// Permissions held by the API token used for this request.
    pub permissions: Vec<String>,
}

impl McpHandler {
    /// Core logic for `get_current_user`; called by the `#[tool]` wrapper in `mod.rs`.
    pub(crate) async fn get_current_user_impl(
        &self,
        ctx: McpRequestContext,
    ) -> Result<Json<GetCurrentUserResult>, ErrorData> {
        let user = User::find_by_id(ctx.user_id)
            .one(self.state.db())
            .await
            .map_err(|e| mcp_error(format!("database error: {e}")))?
            .ok_or_else(|| mcp_error("authenticated user not found in database"))?;

        let permissions = ctx
            .permissions
            .iter()
            .map(|p| p.as_str().to_owned())
            .collect();

        Ok(Json(GetCurrentUserResult {
            user_id: user.id.to_string(),
            email: user.email.expose_email().to_owned(),
            first_name: user.first_name,
            last_name: user.last_name,
            permissions,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::permissions::Permission;
    use uuid::Uuid;

    #[test]
    fn get_current_user_result_serialises() {
        let result = GetCurrentUserResult {
            user_id: Uuid::nil().to_string(),
            email: "test@example.com".to_owned(),
            first_name: "Alice".to_owned(),
            last_name: "Smith".to_owned(),
            permissions: vec!["access_mcp".to_owned(), "view_services".to_owned()],
        };
        let json = serde_json::to_string(&result).expect("serialisation must succeed");
        assert!(
            json.contains("access_mcp"),
            "permissions should appear in JSON"
        );
        assert!(
            json.contains("test@example.com"),
            "email should appear in JSON"
        );
    }

    #[test]
    fn permissions_convert_to_strings() {
        let perms = [Permission::AccessMcp, Permission::ViewServices];
        let strings: Vec<String> = perms.iter().map(|p| p.as_str().to_owned()).collect();
        assert_eq!(strings, ["access_mcp", "view_services"]);
    }
}
