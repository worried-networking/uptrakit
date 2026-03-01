use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use super::permissions::Permission;
use super::token::generate_uuid;
use super::{AuthError, Result};

pub const ACCESS_TOKEN_EXPIRY_SECS: i64 = 900; // 15 minutes

#[derive(Debug, Serialize, Deserialize)]
pub struct AccessTokenClaims {
    pub sub: String,
    pub jti: String,
    pub permissions: Vec<Permission>,
    pub auth_method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oidc_provider_id: Option<String>,
    pub iat: i64,
    pub exp: i64,
    /// Issuer — always `"uptrakit"`. `#[serde(default)]` allows tokens issued
    /// before this field was introduced to be deserialized; the `Validation`
    /// step rejects them because `iss` is in `required_spec_claims`.
    #[serde(default)]
    pub iss: String,
    /// Audience — always `["uptrakit"]`. `#[serde(default)]` allows legacy
    /// tokens without an `aud` claim to reach the `Validation` step, which
    /// rejects them because `aud` is in `required_spec_claims`.
    #[serde(default)]
    pub aud: Vec<String>,
}

pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtManager {
    /// Create a `JwtManager` from a raw secret.
    pub fn from_secret(secret: &[u8]) -> Self {
        Self {
            encoding_key: EncodingKey::from_secret(secret),
            decoding_key: DecodingKey::from_secret(secret),
        }
    }

    /// Create a signed JWT access token.
    pub fn create_access_token(
        &self,
        user_id: uuid::Uuid,
        permissions: &[Permission],
        auth_method: &str,
        oidc_provider_id: Option<uuid::Uuid>,
    ) -> Result<String> {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        let claims = AccessTokenClaims {
            sub: user_id.to_string(),
            jti: generate_uuid().to_string(),
            permissions: permissions.to_vec(),
            auth_method: auth_method.to_string(),
            oidc_provider_id: oidc_provider_id.map(|id| id.to_string()),
            iat: now,
            exp: now + ACCESS_TOKEN_EXPIRY_SECS,
            iss: "uptrakit".to_string(),
            aud: vec!["uptrakit".to_string()],
        };

        jsonwebtoken::encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| report!(AuthError::JwtEncode(e.to_string())))
    }

    /// Decode and validate a JWT access token.
    ///
    /// Requires `iss = "uptrakit"` and `aud = ["uptrakit"]` in addition to the
    /// standard `exp` check. Tokens minted before these claims were introduced
    /// (or tokens from a different deployment) are rejected with
    /// [`AuthError::JwtDecode`].
    pub fn decode_access_token(&self, token: &str) -> Result<AccessTokenClaims> {
        let mut validation = Validation::default();
        validation.set_audience(&["uptrakit"]);
        validation.set_issuer(&["uptrakit"]);
        // Require aud and iss to be present in the token, not just validated when present.
        // Without this, jsonwebtoken v10 skips the audience/issuer check for tokens
        // that omit these claims entirely (e.g. tokens minted before these fields were added).
        validation.required_spec_claims.insert("aud".to_string());
        validation.required_spec_claims.insert("iss".to_string());

        let token_data = jsonwebtoken::decode::<AccessTokenClaims>(
            token,
            &self.decoding_key,
            &validation,
        )
        .map_err(|e| report!(AuthError::JwtDecode(e.to_string())))?;

        Ok(token_data.claims)
    }

    /// Return the access token expiry in seconds (for API responses).
    pub fn expires_in(&self) -> i64 {
        ACCESS_TOKEN_EXPIRY_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager() -> JwtManager {
        JwtManager::from_secret(b"test-secret-key-for-jwt-testing-only-do-not-use")
    }

    #[test]
    fn test_create_and_decode_access_token() {
        let manager = test_manager();
        let user_id = uuid::Uuid::now_v7();
        let permissions = vec![Permission::ViewSettings, Permission::ManageAgents];

        let token = manager
            .create_access_token(user_id, &permissions, "password", None)
            .unwrap();

        let claims = manager.decode_access_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(
            claims.permissions,
            vec![Permission::ViewSettings, Permission::ManageAgents]
        );
        assert_eq!(claims.auth_method, "password");
        assert!(claims.oidc_provider_id.is_none());
    }

    #[test]
    fn test_create_and_decode_with_oidc() {
        let manager = test_manager();
        let user_id = uuid::Uuid::now_v7();
        let provider_id = uuid::Uuid::now_v7();
        let permissions = vec![Permission::ViewAgents];

        let token = manager
            .create_access_token(user_id, &permissions, "oidc", Some(provider_id))
            .unwrap();

        let claims = manager.decode_access_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.auth_method, "oidc");
        assert_eq!(claims.oidc_provider_id, Some(provider_id.to_string()));
    }

    #[test]
    fn test_decode_invalid_token() {
        let manager = test_manager();
        let result = manager.decode_access_token("invalid.token.here");
        assert!(result.is_err());
    }

    #[test]
    fn test_decode_wrong_secret() {
        let manager1 = JwtManager::from_secret(b"secret-one");
        let manager2 = JwtManager::from_secret(b"secret-two");
        let user_id = uuid::Uuid::now_v7();

        let token = manager1
            .create_access_token(user_id, &[], "password", None)
            .unwrap();

        let result = manager2.decode_access_token(&token);
        assert!(result.is_err());
    }

    /// Tokens minted without `aud`/`iss` (e.g. before this validation was
    /// introduced) must be rejected even when signed with the correct key.
    #[test]
    fn test_decode_legacy_token_without_aud_rejected() {
        use jsonwebtoken::{EncodingKey, Header};

        let secret = b"test-secret-key-for-jwt-testing-only-do-not-use";
        let manager = JwtManager::from_secret(secret);

        // Simulate a legacy token struct that does not include `aud` or `iss`.
        #[derive(serde::Serialize)]
        struct LegacyClaims<'a> {
            sub: &'a str,
            jti: &'a str,
            permissions: Vec<Permission>,
            auth_method: &'a str,
            iat: i64,
            exp: i64,
        }

        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let legacy = LegacyClaims {
            sub: "00000000-0000-0000-0000-000000000001",
            jti: "00000000-0000-0000-0000-000000000002",
            permissions: vec![],
            auth_method: "password",
            iat: now,
            exp: now + 900,
        };

        let token = jsonwebtoken::encode(
            &Header::default(),
            &legacy,
            &EncodingKey::from_secret(secret),
        )
        .expect("encode legacy token");

        let result = manager.decode_access_token(&token);
        assert!(
            result.is_err(),
            "token without aud/iss must be rejected by decode_access_token"
        );
    }

    /// Two `JwtManager` instances built from the same secret must accept each
    /// other's tokens — `from_secret` is deterministic (no internal state).
    #[test]
    fn test_from_secret_is_deterministic() {
        let secret = b"deterministic-secret-for-jwt-testing-only";
        let manager1 = JwtManager::from_secret(secret);
        let manager2 = JwtManager::from_secret(secret);

        let user_id = uuid::Uuid::now_v7();
        let token = manager1
            .create_access_token(user_id, &[], "password", None)
            .unwrap();

        let claims = manager2.decode_access_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
    }
}
