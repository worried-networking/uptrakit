use std::sync::Arc;

use rmcp::{
    ErrorData, Json, ServerHandler,
    handler::server::tool::Extension,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::AppState;
use crate::mcp::auth::McpRequestContext;

pub mod history;
pub mod update;
pub mod user;

use history::{GetUpdateHistoryDetailInput, ListUpdateHistoryInput};
use history::{ListUpdateHistoryResult, UpdateHistoryDetailResult};
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

    /// List update history records for the authenticated tenant, newest first.
    ///
    /// Requires the `ViewSoftware` permission. The `output` field is excluded
    /// from list results — call `get_update_history_detail` to retrieve the
    /// rendered terminal output for a specific record.
    #[tool(
        name = "list_update_history",
        description = "List update history records for the authenticated tenant, newest \
                       first. Requires ViewSoftware permission. Terminal output is not \
                       included; use get_update_history_detail for that."
    )]
    pub async fn list_update_history(
        &self,
        Extension(ctx): Extension<McpRequestContext>,
        Parameters(input): Parameters<ListUpdateHistoryInput>,
    ) -> Result<Json<ListUpdateHistoryResult>, ErrorData> {
        self.list_update_history_impl(ctx, input).await
    }

    /// Retrieve a single update history record with rendered terminal output.
    ///
    /// Requires the `ViewSoftware` permission. The raw vt100 byte stream is
    /// rendered to plain text (ANSI escape sequences stripped) before returning.
    #[tool(
        name = "get_update_history_detail",
        description = "Retrieve a single update history record, including rendered \
                       terminal output (ANSI escapes stripped). Requires ViewSoftware \
                       permission."
    )]
    pub async fn get_update_history_detail(
        &self,
        Extension(ctx): Extension<McpRequestContext>,
        Parameters(input): Parameters<GetUpdateHistoryDetailInput>,
    ) -> Result<Json<UpdateHistoryDetailResult>, ErrorData> {
        self.get_update_history_detail_impl(ctx, input).await
    }
}

#[tool_handler]
impl ServerHandler for McpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::default())
            .with_server_info(Implementation::new("uptrakit", env!("CARGO_PKG_VERSION")))
    }
}
