use std::sync::Arc;

use rmcp::{
    ErrorData, Json, ServerHandler,
    handler::server::tool::Extension,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::AppState;
use crate::mcp::auth::McpRequestContext;

pub mod history;
pub mod update;
pub mod user;

use user::GetCurrentUserResult;

/// Build an `rmcp::ErrorData` with the `internal_error` code.
pub(crate) fn mcp_error(msg: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(msg.into(), None)
}

/// Minimal MCP handler wired to `AppState`.
#[derive(Clone)]
pub struct McpHandler {
    pub(crate) state: Arc<AppState>,
}

impl McpHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

#[tool_router]
impl McpHandler {
    /// Return the identity (user_id, email, name, permissions) of the API token
    /// owner. No `ManageUsers` permission is required; any valid `AccessMcp`
    /// token may call this tool.
    #[tool(
        name = "get_current_user",
        description = "Return the identity (user_id, email, name, permissions) of the \
                       API token owner. No ManageUsers permission required."
    )]
    pub async fn get_current_user(
        &self,
        Extension(ctx): Extension<McpRequestContext>,
    ) -> Result<Json<GetCurrentUserResult>, ErrorData> {
        self.get_current_user_impl(ctx).await
    }
}

#[tool_handler]
impl ServerHandler for McpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::default())
            .with_server_info(Implementation::new("uptrakit", env!("CARGO_PKG_VERSION")))
    }
}
