use std::sync::Arc;

use rmcp::{
    ServerHandler,
    model::{Implementation, ServerCapabilities, ServerInfo},
};

use crate::AppState;

pub mod history;
pub mod update;
pub mod user;

/// Minimal MCP handler wired to `AppState`.
///
/// Tool implementations are added in subsequent tasks.
#[derive(Clone)]
pub struct McpHandler {
    // Will be accessed by tool methods added in subsequent tasks.
    #[allow(dead_code)]
    pub(crate) state: Arc<AppState>,
}

impl McpHandler {
    pub fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

impl ServerHandler for McpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::default())
            .with_server_info(Implementation::new("uptrakit", env!("CARGO_PKG_VERSION")))
    }
}
