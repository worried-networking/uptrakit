//! OAuth 2.1 wire types shared between `uptrakit-web-api` (authorization
//! server) and `uptrakit-mcp` (resource server).

pub mod canonical_url;
pub mod grant_type;
pub mod scope;

pub use canonical_url::{
    CanonicalResourceUrl, CanonicalUrlConfig, CanonicalUrlConfigError, CanonicalUrlError,
    MAX_ACCEPTED_AUDIENCE_HOSTS,
};
pub use grant_type::{CodeChallengeMethod, OAuthGrantType, ResponseType, TokenEndpointAuthMethod};
pub use scope::McpScope;
