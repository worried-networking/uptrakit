//! OAuth 2.1 wire types shared between `uptrakit-web-api` (authorization
//! server) and `uptrakit-mcp` (resource server).

pub mod canonical_url;
pub mod device;
pub mod error;
pub mod grant_type;
pub mod metadata;
pub mod requests;
pub mod responses;
pub mod scope;
pub mod tokens;

pub use canonical_url::{
    CanonicalResourceUrl, CanonicalUrlConfig, CanonicalUrlConfigError, CanonicalUrlError,
    MAX_ACCEPTED_AUDIENCE_HOSTS,
};
pub use device::{
    DeviceAuthDenyRequest, DeviceAuthDenyResponse, DeviceAuthLookupQuery, DeviceAuthLookupResponse,
    DeviceAuthorizationRequest, DeviceAuthorizationResponse, OAuthAuthorizationServerMetadata,
    OAuthErrorCode, OAuthErrorResponse, OAuthTokenRequest, OAuthTokenResponse,
    ParseOAuthErrorCodeError, USER_CODE_ALPHABET,
};
pub use error::OAuthError;
pub use grant_type::{CodeChallengeMethod, OAuthGrantType, ResponseType, TokenEndpointAuthMethod};
pub use metadata::{AuthorizationServerMetadata, ProtectedResourceMetadata};
pub use requests::{AuthorizeRequest, ConsentDecision, TokenRequest};
pub use responses::{DcrRegistrationRequest, DcrRegistrationResponse, TokenResponse};
pub use scope::McpScope;
pub use tokens::{AuthorizationCode, McpAccessTokenClaims, OpaqueRefreshToken, TokenParseError};

/// MCP Authorization spec revision this implementation targets. Emitted by the PRM
/// endpoint as `x-uptrakit-mcp-auth-spec-revision` per spec §23.1 so downstream tooling
/// can correlate behavior with the spec revision.
pub const MCP_AUTH_SPEC_REVISION: &str = "2025-11-25";
