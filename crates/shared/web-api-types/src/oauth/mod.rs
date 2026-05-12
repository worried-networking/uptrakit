//! OAuth 2.1 wire types shared between `uptrakit-web-api` (authorization
//! server) and `uptrakit-mcp` (resource server).

pub mod canonical_url;
pub mod error;
pub mod grant_type;
pub mod requests;
pub mod scope;
pub mod tokens;

pub use canonical_url::{
    CanonicalResourceUrl, CanonicalUrlConfig, CanonicalUrlConfigError, CanonicalUrlError,
    MAX_ACCEPTED_AUDIENCE_HOSTS,
};
pub use error::OAuthError;
pub use grant_type::{CodeChallengeMethod, OAuthGrantType, ResponseType, TokenEndpointAuthMethod};
pub use requests::{AuthorizeRequest, ConsentDecision, TokenRequest};
pub use scope::McpScope;
pub use tokens::{AuthorizationCode, McpAccessTokenClaims, OpaqueRefreshToken, TokenParseError};
