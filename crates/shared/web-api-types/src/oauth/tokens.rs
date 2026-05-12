//! Token-shape newtypes + access-token claims envelope.
//!
//! The newtypes [`OpaqueRefreshToken`] and [`AuthorizationCode`] guard the
//! prefix invariants required by the MCP OAuth spec §10.2 (refresh tokens
//! begin with `upr_`; authorization codes begin with `upc_`). Parsing is the
//! single entry point for taking a wire string into the typed form; parsed
//! values either get hashed for storage or returned to the client.
//!
//! [`McpAccessTokenClaims`] is the JWT claims envelope defined by MCP OAuth
//! spec §9.1. The `typ: "at+jwt"` declaration required by RFC 9068 lives in
//! the JWT header (not in these claims) and is set by the signing layer.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned when parsing a wire token string into a typed newtype.
#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenParseError {
    /// The supplied string did not begin with the expected token prefix.
    #[error("token must begin with {expected:?}")]
    WrongPrefix {
        /// The required prefix (e.g. `"upr_"` or `"upc_"`).
        expected: &'static str,
    },
}

/// Opaque refresh token carried with the `upr_` prefix.
///
/// Refresh tokens are opaque to clients per MCP OAuth spec §10.2; the prefix
/// is a deployment convention so a leaked token can be identified by shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpaqueRefreshToken(String);

impl OpaqueRefreshToken {
    /// Parse a `upr_`-prefixed refresh token string.
    ///
    /// # Errors
    ///
    /// Returns [`TokenParseError::WrongPrefix`] if the string does not begin
    /// with `upr_`.
    #[must_use = "parsed refresh token must be either hashed for storage or returned to client"]
    pub fn parse(s: &str) -> Result<Self, TokenParseError> {
        if !s.starts_with("upr_") {
            return Err(TokenParseError::WrongPrefix { expected: "upr_" });
        }
        Ok(Self(s.to_string()))
    }

    /// Returns the underlying token string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Authorization code carried with the `upc_` prefix.
///
/// Authorization codes are single-use opaque values exchanged for tokens at
/// the token endpoint per MCP OAuth spec §9. The prefix lets infrastructure
/// distinguish a code from an access or refresh token at a glance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizationCode(String);

impl AuthorizationCode {
    /// Parse a `upc_`-prefixed authorization code string.
    ///
    /// # Errors
    ///
    /// Returns [`TokenParseError::WrongPrefix`] if the string does not begin
    /// with `upc_`.
    #[must_use = "parsed authorization code must be hashed for storage or redeemed at the token endpoint"]
    pub fn parse(s: &str) -> Result<Self, TokenParseError> {
        if !s.starts_with("upc_") {
            return Err(TokenParseError::WrongPrefix { expected: "upc_" });
        }
        Ok(Self(s.to_string()))
    }

    /// Returns the underlying authorization code string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// MCP OAuth access token claims envelope per spec §9.1.
///
/// `typ: "at+jwt"` per RFC 9068 lives in the JWT header, not these claims.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAccessTokenClaims {
    /// Issuer — canonical URL of the authorization server.
    pub iss: String,
    /// Subject — user UUID encoded as a string.
    pub sub: String,
    /// Audience — canonical resource URL of the MCP server.
    pub aud: String,
    /// OAuth client identifier of the authorized client.
    pub client_id: String,
    /// Space-separated list of granted scope values.
    pub scope: String,
    /// Unique JWT identifier — used for revocation/audit correlation.
    pub jti: String,
    /// Issued-at time (seconds since Unix epoch).
    pub iat: i64,
    /// Not-before time (seconds since Unix epoch).
    pub nbf: i64,
    /// Expiration time (seconds since Unix epoch).
    pub exp: i64,
    /// Tenant UUID owning the subject.
    pub tenant_id: String,
}

impl McpAccessTokenClaims {
    /// Construct a claims envelope.
    ///
    /// Required because `#[non_exhaustive]` prevents struct-literal construction
    /// outside this crate.
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "all JWT spec claims are required"
    )]
    pub fn new(
        iss: String,
        sub: String,
        aud: String,
        client_id: String,
        scope: String,
        jti: String,
        iat: i64,
        nbf: i64,
        exp: i64,
        tenant_id: String,
    ) -> Self {
        Self {
            iss,
            sub,
            aud,
            client_id,
            scope,
            jti,
            iat,
            nbf,
            exp,
            tenant_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opaque_refresh_token_must_use_upr_prefix() {
        OpaqueRefreshToken::parse("upr_abc").unwrap();
        assert!(matches!(
            OpaqueRefreshToken::parse("upk_abc"),
            Err(TokenParseError::WrongPrefix { .. })
        ));
    }

    #[test]
    fn authorization_code_must_use_upc_prefix() {
        AuthorizationCode::parse("upc_abc").unwrap();
        assert!(matches!(
            AuthorizationCode::parse("upr_abc"),
            Err(TokenParseError::WrongPrefix { .. })
        ));
    }

    #[test]
    fn access_claims_round_trip_json() {
        let claims = McpAccessTokenClaims {
            iss: "https://example.com".into(),
            sub: "00000000-0000-0000-0000-000000000001".into(),
            aud: "https://example.com/mcp".into(),
            client_id: "abc".into(),
            scope: "mcp:read mcp:write".into(),
            jti: "00000000-0000-0000-0000-000000000002".into(),
            iat: 1_715_520_000,
            nbf: 1_715_520_000,
            exp: 1_715_520_900,
            tenant_id: "00000000-0000-0000-0000-000000000003".into(),
        };
        let json = serde_json::to_string(&claims).unwrap();
        let back: McpAccessTokenClaims = serde_json::from_str(&json).unwrap();
        assert_eq!(claims, back);
    }
}
