use std::net::SocketAddr;

use uptrakit_controller_core::settings::Settings;

/// MCP-specific projection of [`Settings`].
///
/// Extracts only the fields the MCP transport layer needs so that
/// `build_mcp_router` can operate without holding a reference to the full
/// `AppState` or `McpState`.
pub struct McpSettings {
    pub sans: Vec<String>,
    pub https_addr: SocketAddr,
}

impl From<&Settings> for McpSettings {
    fn from(s: &Settings) -> Self {
        Self {
            sans: s.sans(),
            https_addr: s.https_addr(),
        }
    }
}
