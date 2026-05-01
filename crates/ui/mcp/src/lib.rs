use std::sync::Arc;

use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use uptrakit_web_api::AppState;

use crate::auth::McpAuthLayer;
use crate::tools::McpHandler;

pub mod auth;
pub mod terminal;
pub mod tools;

/// Mount the MCP Streamable HTTP transport at `/mcp`.
///
/// The returned router has no axum state type parameter (`Router<()>`); the
/// `McpHandler` captures `Arc<AppState>` directly so no `.with_state()` call
/// is needed.
pub fn build_mcp_router(state: Arc<AppState>) -> Router {
    let config = build_config(&state);
    let raw_service = StreamableHttpService::new(
        {
            let state = Arc::clone(&state);
            move || Ok(McpHandler::new(Arc::clone(&state)))
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let auth_layer = McpAuthLayer::new(Arc::clone(&state));
    let service = tower::ServiceBuilder::new()
        .layer(auth_layer)
        .service(raw_service);

    Router::new().nest_service("/mcp", service)
}

fn build_config(state: &AppState) -> StreamableHttpServerConfig {
    let sans = state.settings.sans();
    let allowed_hosts = build_allowed_hosts(&sans);
    StreamableHttpServerConfig::default().with_allowed_hosts(allowed_hosts)
}

fn build_allowed_hosts(sans: &[String]) -> Vec<String> {
    let mut hosts = Vec::with_capacity(sans.len() * 4);
    for san in sans {
        hosts.push(san.clone());
        hosts.push(format!("{san}:9443"));
        hosts.push(format!("{san}:443"));
        hosts.push(format!("{san}:80"));
    }
    hosts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_allowed_hosts_includes_port_variants() {
        let sans = vec!["controller.example.com".to_string()];
        let hosts = build_allowed_hosts(&sans);
        assert!(hosts.contains(&"controller.example.com".to_string()));
        assert!(hosts.contains(&"controller.example.com:9443".to_string()));
        assert!(hosts.contains(&"controller.example.com:443".to_string()));
        assert!(hosts.contains(&"controller.example.com:80".to_string()));
    }

    #[test]
    fn build_allowed_hosts_empty_sans() {
        let hosts = build_allowed_hosts(&[]);
        assert!(hosts.is_empty());
    }
}
