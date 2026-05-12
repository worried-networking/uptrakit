//! OAuth AS-internal typed enums (RFC 6749 / RFC 8414 vocabulary).
//!
//! These enums describe discriminators that are produced and consumed by the
//! authorization server. Unlike [`crate::oauth::McpScope`] they are not
//! intended to round-trip through an older peer, so they use derived
//! `Serialize` / `Deserialize` and have no `Other(String)` catch-all. Each
//! enum is `Copy` per the typed-enum-for-internal-discriminator rule.

use serde::{Deserialize, Serialize};

/// OAuth 2.1 grant types accepted by the authorization server's token
/// endpoint (RFC 6749 §4 / RFC 6749 §6).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthGrantType {
    /// Exchange an authorization code for tokens (`authorization_code`).
    AuthorizationCode,
    /// Refresh an existing access token (`refresh_token`).
    RefreshToken,
}

impl OAuthGrantType {
    /// Returns the canonical OAuth string representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            OAuthGrantType::AuthorizationCode => "authorization_code",
            OAuthGrantType::RefreshToken => "refresh_token",
        }
    }
}

/// OAuth 2.1 authorization endpoint response types (RFC 6749 §3.1.1).
///
/// MCP authorization servers only issue `code` per OAuth 2.1.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResponseType {
    /// Authorization Code flow response type (`code`).
    Code,
}

/// PKCE code challenge methods (RFC 7636 §4.2).
///
/// MCP authorization servers only accept `S256` per OAuth 2.1.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CodeChallengeMethod {
    /// SHA-256 PKCE method. Serialized as the literal `"S256"`.
    S256,
}

/// Client authentication methods accepted at the token endpoint
/// (RFC 8414 §2 `token_endpoint_auth_methods_supported`).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenEndpointAuthMethod {
    /// Public client; no client secret presented (`none`).
    None,
    /// Confidential client using HTTP Basic auth (`client_secret_basic`).
    ClientSecretBasic,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grant_type_serializes_as_oauth_strings() {
        assert_eq!(
            serde_json::to_string(&OAuthGrantType::AuthorizationCode).unwrap(),
            r#""authorization_code""#
        );
        assert_eq!(
            serde_json::to_string(&OAuthGrantType::RefreshToken).unwrap(),
            r#""refresh_token""#
        );
    }

    #[test]
    fn code_challenge_method_only_s256() {
        let s = serde_json::to_string(&CodeChallengeMethod::S256).unwrap();
        assert_eq!(s, r#""S256""#);
    }
}
