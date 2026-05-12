//! OAuth 2.1 wire types shared between `uptrakit-web-api` (authorization
//! server) and `uptrakit-mcp` (resource server).

pub mod scope;

pub use scope::McpScope;
