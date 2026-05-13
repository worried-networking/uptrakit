pub mod prm;
pub mod tool_auth;

pub use tool_auth::{McpScopeError, ToolAuth, require_scopes};
