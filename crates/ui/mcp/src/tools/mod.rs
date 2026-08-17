use rmcp::{
    ErrorData, Json, ServerHandler,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};

use crate::context::McpRequestContext;
use crate::state::McpState;

pub mod history;
pub mod update;
pub mod user;

use history::{GetUpdateHistoryDetailInput, ListUpdateHistoryInput};
use history::{ListUpdateHistoryResult, UpdateHistoryDetailResult};
use update::{TriggerUpdateInput, TriggerUpdateResult};
use user::GetCurrentUserResult;

/// Build an `rmcp::ErrorData` with the `internal_error` code.
pub(crate) fn mcp_error(msg: impl Into<String>) -> ErrorData {
    ErrorData::internal_error(msg.into(), None)
}

/// Minimal MCP handler wired to [`McpState`].
#[derive(Clone)]
pub struct McpHandler {
    pub(crate) state: McpState,
}

impl McpHandler {
    /// Create a new [`McpHandler`] from the given [`McpState`].
    #[must_use]
    pub fn new(state: McpState) -> Self {
        Self { state }
    }
}

#[tool_router]
impl McpHandler {
    /// Return the identity (user_id, email, name) of the API token owner. No
    /// catalog action is required beyond the connection-level `mcp:use` grant;
    /// any authenticated MCP principal may call this tool.
    #[tool(
        name = "get_current_user",
        description = "Return the identity (user_id, email, name) of the API token \
                       owner. No action beyond the mcp:use connection grant is required."
    )]
    pub async fn get_current_user(
        &self,
        ctx: McpRequestContext,
    ) -> Result<Json<GetCurrentUserResult>, ErrorData> {
        self.get_current_user_impl(ctx).await
    }

    /// List update history records for the authenticated tenant, newest first.
    ///
    /// Requires the `software:read` action. The `output` field is excluded
    /// from list results — call `get_update_history_detail` to retrieve the
    /// rendered terminal output for a specific record.
    #[tool(
        name = "list_update_history",
        description = "List update history records for the authenticated tenant, newest \
                       first. Requires the software:read action. Terminal output is not \
                       included; use get_update_history_detail for that."
    )]
    pub async fn list_update_history(
        &self,
        ctx: McpRequestContext,
        Parameters(input): Parameters<ListUpdateHistoryInput>,
    ) -> Result<Json<ListUpdateHistoryResult>, ErrorData> {
        self.list_update_history_impl(ctx, input).await
    }

    /// Retrieve a single update history record with rendered terminal output.
    ///
    /// Requires the `software:read` action. The raw vt100 byte stream is
    /// rendered to plain text (ANSI escape sequences stripped) before returning.
    #[tool(
        name = "get_update_history_detail",
        description = "Retrieve a single update history record, including rendered \
                       terminal output (ANSI escapes stripped). Requires the \
                       software:read action."
    )]
    pub async fn get_update_history_detail(
        &self,
        ctx: McpRequestContext,
        Parameters(input): Parameters<GetUpdateHistoryDetailInput>,
    ) -> Result<Json<UpdateHistoryDetailResult>, ErrorData> {
        self.get_update_history_detail_impl(ctx, input).await
    }

    /// Trigger a software update for a specific host.
    ///
    /// Requires the `updates:trigger` action. The `interactive` flag is
    /// always `false` — AI agents cannot interact with a PTY.
    #[tool(
        name = "trigger_update",
        description = "Trigger a software update for a specific host. \
                       Requires the updates:trigger action. interactive is always \
                       false — AI agents cannot interact with a PTY."
    )]
    pub async fn trigger_update(
        &self,
        ctx: McpRequestContext,
        Parameters(input): Parameters<TriggerUpdateInput>,
    ) -> Result<Json<TriggerUpdateResult>, ErrorData> {
        self.trigger_update_impl(ctx, input).await
    }
}

#[tool_handler]
impl ServerHandler for McpHandler {
    // Protocol version: deliberately left at rmcp's default
    // (`ProtocolVersion::LATEST`, which is `2025-11-25` in rmcp 3.1). That value
    // is only the *fallback* used when a client asks for a version the server
    // does not know — `negotiate_protocol_version` honours any client-requested
    // version present in `supported_protocol_versions()`, which defaults to
    // `ProtocolVersion::KNOWN_VERSIONS` and already includes `V_2026_07_28`.
    // So a 2026-07-28 client negotiates 2026-07-28 without an override here,
    // while older clients keep working. Pinning `with_protocol_version` would
    // only narrow that, and opting the *fallback* up to 2026-07-28 would make
    // SEP-2243 standard headers mandatory for clients that never asked for
    // them. Revisit if the fallback needs to move.
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("uptrakit", env!("CARGO_PKG_VERSION")))
    }
}
