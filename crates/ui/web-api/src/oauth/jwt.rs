//! MCP OAuth JWT signer. HS256 pinned; kid header for future rotation.
//!
//! The verifier ([`McpOAuthJwtVerifier`]) and error type ([`JwtError`]) live in
//! `uptrakit-web-api-types` so that `uptrakit-mcp` can import them without a
//! circular dependency on `uptrakit-web-api`.

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use sha2::{Digest, Sha256};
use uptrakit_web_api_types::oauth::McpAccessTokenClaims;

pub use uptrakit_web_api_types::oauth::{JwtError, McpOAuthJwtVerifier};

/// Signs MCP OAuth access tokens using HS256.
///
/// The `kid` header is derived from the first 16 hex characters of SHA-256(secret)
/// so that key rotation can be tracked without exposing the raw secret.
///
/// `EncodingKey` is not `Clone`, so wrap in `Arc<McpOAuthJwtSigner>` for sharing.
pub struct McpOAuthJwtSigner {
    key: EncodingKey,
    kid: String,
}

impl McpOAuthJwtSigner {
    /// Create a new signer from a raw HMAC secret.
    pub fn new(secret: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(secret);
        let digest = hasher.finalize();
        // Take the first 16 hex chars. `chars().take()` avoids the `string_slice`
        // lint; hex output is pure ASCII so no multi-byte boundary risk, but the
        // lint fires on index expressions regardless of content.
        let kid: String = hex::encode(digest).chars().take(16).collect();
        Self {
            key: EncodingKey::from_secret(secret),
            kid,
        }
    }

    /// Mint an `at+jwt`-typed access token with HS256 signature and `kid` header.
    ///
    /// # Errors
    ///
    /// Returns [`JwtError::Jwt`] if encoding fails (e.g. claims cannot be serialized).
    pub fn mint(&self, claims: &McpAccessTokenClaims) -> Result<String, JwtError> {
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("at+jwt".to_string());
        header.kid = Some(self.kid.clone());
        Ok(encode(&header, claims, &self.key)?)
    }

    /// Returns the `kid` (key identifier) derived from the secret.
    #[must_use]
    pub fn kid(&self) -> &str {
        &self.kid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn round_trips_minted_claims() {
        let signer = McpOAuthJwtSigner::new(&fixed_secret());
        let claims = sample_claims();
        let token = signer.mint(&claims).unwrap();
        let verifier = McpOAuthJwtVerifier::new(
            &fixed_secret(),
            "https://example.com".into(),
            vec!["https://example.com/mcp".into()],
        );
        let decoded = verifier.verify(&token).unwrap();
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
        let signer = McpOAuthJwtSigner::new(&fixed_secret());
        let mut claims = sample_claims();
        claims.client_id = String::new();
        let token = signer.mint(&claims).unwrap();
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
