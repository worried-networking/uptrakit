//! OAuth 2.1 wire types shared between `uptrakit-web-api` (authorization
//! server) and `uptrakit-mcp` (resource server).

pub mod grant_type;
pub mod scope;

pub use grant_type::{CodeChallengeMethod, OAuthGrantType, ResponseType, TokenEndpointAuthMethod};
pub use scope::McpScope;
