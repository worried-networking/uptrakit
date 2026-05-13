//! MCP OAuth JWT verifier. HS256 pinned; shared between the authorization
//! server (`uptrakit-web-api`) and the resource server (`uptrakit-mcp`).

use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use std::collections::HashSet;
use thiserror::Error;

use crate::oauth::McpAccessTokenClaims;

/// Errors produced by JWT verification.
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum JwtError {
    /// Underlying `jsonwebtoken` error (invalid signature, expired, malformed, etc.).
    #[error("jsonwebtoken: {0}")]
    Jwt(#[from] jsonwebtoken::errors::Error),
    /// The token header declared an algorithm other than HS256.
    #[error("algorithm pinning violation")]
    AlgorithmPinningViolation,
    /// The `aud` claim did not match any accepted audience.
    #[error("audience mismatch")]
    InvalidAudience,
    /// The `iss` claim did not match the expected issuer.
    #[error("issuer mismatch")]
    InvalidIssuer,
    /// A required application-level claim was present but empty.
    #[error("missing required claim: {0}")]
    MissingRequiredClaim(&'static str),
}

/// Verifies MCP OAuth access tokens issued by `McpOAuthJwtSigner`.
///
/// Enforces HS256-only algorithm, validates standard spec claims (`iss`, `sub`,
/// `aud`, `exp`, `iat`, `nbf`, `jti`) via `jsonwebtoken`, and then checks that
/// the application-level claims `client_id`, `tenant_id`, and `jti` are non-empty.
///
/// `DecodingKey` is not `Clone`, so wrap in `Arc<McpOAuthJwtVerifier>` for sharing.
pub struct McpOAuthJwtVerifier {
    key: DecodingKey,
    expected_issuer: String,
    accepted_audiences: HashSet<String>,
}

impl McpOAuthJwtVerifier {
    /// Create a new verifier.
    ///
    /// - `secret`: the same HMAC secret used by the matching `McpOAuthJwtSigner`.
    /// - `expected_issuer`: the `iss` claim value that tokens must carry.
    /// - `accepted_audiences`: the set of `aud` values that are considered valid.
    pub fn new(secret: &[u8], expected_issuer: String, accepted_audiences: Vec<String>) -> Self {
        let set = accepted_audiences.into_iter().collect();
        Self {
            key: DecodingKey::from_secret(secret),
            expected_issuer,
            accepted_audiences: set,
        }
    }

    /// Verify a compact JWT string.
    ///
    /// Checks, in order:
    /// 1. HS256 algorithm pin (via `Validation::algorithms`).
    /// 2. All standard spec claims (`iss`, `sub`, `aud`, `exp`, `iat`, `nbf`, `jti`)
    ///    are present and structurally valid.
    /// 3. `iss` matches `expected_issuer`.
    /// 4. `aud` is a member of `accepted_audiences`.
    /// 5. Application claims `client_id`, `tenant_id`, and `jti` are non-empty strings.
    ///
    /// # Errors
    ///
    /// - [`JwtError::Jwt`] — signature invalid, token expired, missing spec claim, etc.
    /// - [`JwtError::MissingRequiredClaim`] — `client_id`, `tenant_id`, or `jti` is empty.
    pub fn verify(&self, token: &str) -> Result<McpAccessTokenClaims, JwtError> {
        let mut validation = Validation::new(Algorithm::HS256);
        // Pin to exactly one algorithm — rejects `alg: none` and any other alg.
        validation.algorithms = vec![Algorithm::HS256];

        // Require all standard spec claims defined by RFC 9068 / MCP OAuth §9.1.
        let mut req = HashSet::new();
        for c in ["iss", "sub", "aud", "exp", "iat", "nbf", "jti"] {
            req.insert(c.to_string());
        }
        validation.required_spec_claims = req;

        validation.set_issuer(&[self.expected_issuer.as_str()]);
        let audiences: Vec<&str> = self.accepted_audiences.iter().map(String::as_str).collect();
        validation.set_audience(&audiences);

        let data = decode::<McpAccessTokenClaims>(token, &self.key, &validation)?;

        // Application-level required non-empty claims.
        if data.claims.client_id.is_empty() {
            return Err(JwtError::MissingRequiredClaim("client_id"));
        }
        if data.claims.tenant_id.is_empty() {
            return Err(JwtError::MissingRequiredClaim("tenant_id"));
        }
        if data.claims.jti.is_empty() {
            return Err(JwtError::MissingRequiredClaim("jti"));
        }

        Ok(data.claims)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use sha2::{Digest, Sha256};

    fn fixed_secret() -> Vec<u8> {
        b"unit-test-secret-32-bytes-minimum".to_vec()
    }

    fn sample_claims() -> McpAccessTokenClaims {
        McpAccessTokenClaims::new(
            "https://example.com".into(),
            "00000000-0000-0000-0000-000000000001".into(),
            "https://example.com/mcp".into(),
            "abc".into(),
            "mcp:read".into(),
            "00000000-0000-0000-0000-000000000002".into(),
            1,
            1,
            9_999_999_999,
            "00000000-0000-0000-0000-000000000003".into(),
        )
    }

    /// Minimal inline signer used only in tests — the real signer lives in
    /// `uptrakit-web-api` which cannot be imported from this shared crate.
    fn mint_token(secret: &[u8], claims: &McpAccessTokenClaims) -> String {
        let mut hasher = Sha256::new();
        hasher.update(secret);
        let digest = hasher.finalize();
        let kid: String = format!("{digest:x}").chars().take(16).collect();
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("at+jwt".to_string());
        header.kid = Some(kid);
        encode(&header, claims, &EncodingKey::from_secret(secret))
            .expect("test token encoding must succeed")
    }

    #[test]
    fn round_trips_minted_claims() {
        let claims = sample_claims();
        let token = mint_token(&fixed_secret(), &claims);
        let verifier = McpOAuthJwtVerifier::new(
            &fixed_secret(),
            "https://example.com".into(),
            vec!["https://example.com/mcp".into()],
        );
        let decoded = verifier.verify(&token).expect("valid token must verify");
        assert_eq!(decoded.sub, claims.sub);
    }

    #[test]
    fn rejects_non_hs256_alg() {
        // alg=none token (manually crafted base64url, no signature)
        let none_token = "eyJhbGciOiJub25lIiwidHlwIjoiYXQrand0In0.eyJzdWIiOiJ4IiwiaXNzIjoiaHR0cHM6Ly9leGFtcGxlLmNvbSIsImF1ZCI6Imh0dHBzOi8vZXhhbXBsZS5jb20vbWNwIiwiZXhwIjo5OTk5OTk5OTk5LCJpYXQiOjEsIm5iZiI6MX0.";
        let verifier = McpOAuthJwtVerifier::new(
            &fixed_secret(),
            "https://example.com".into(),
            vec!["https://example.com/mcp".into()],
        );
        verifier.verify(none_token).unwrap_err();
    }

    #[test]
    fn rejects_empty_client_id() {
        let mut claims = sample_claims();
        claims.client_id = String::new();
        let token = mint_token(&fixed_secret(), &claims);
        let verifier = McpOAuthJwtVerifier::new(
            &fixed_secret(),
            "https://example.com".into(),
            vec!["https://example.com/mcp".into()],
        );
        let result = verifier.verify(&token);
        assert!(matches!(
            result,
            Err(JwtError::MissingRequiredClaim("client_id"))
        ));
    }
}
