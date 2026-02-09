use std::path::Path;

use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use rootcause::prelude::*;
use serde::{Deserialize, Serialize};

use super::permissions::Permission;
use super::token::generate_uuid;
use super::{AuthError, Result};

pub const ACCESS_TOKEN_EXPIRY_SECS: i64 = 900; // 15 minutes
const KEY_FILE_NAME: &str = "jwt_signing.key";
const KEY_LENGTH: usize = 64;

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
}

pub struct JwtManager {
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtManager {
    /// Load or generate a JWT signing key from the data directory.
    ///
    /// Creates `{data_dir}/jwt_signing.key` with 64 random bytes if it doesn't exist.
    /// Sets file permissions to 0o600.
    pub fn load_or_generate(data_dir: &Path) -> Result<Self> {
        let key_path = data_dir.join(KEY_FILE_NAME);

        let secret = if key_path.exists() {
            std::fs::read(&key_path).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to read JWT signing key: {e}"
                )))
            })?
        } else {
            let mut rng = rand::rng();
            let mut bytes = vec![0u8; KEY_LENGTH];
            rng.fill(&mut bytes[..]);

            std::fs::write(&key_path, &bytes).map_err(|e| {
                report!(AuthError::Internal(format!(
                    "failed to write JWT signing key: {e}"
                )))
            })?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| {
                        report!(AuthError::Internal(format!(
                            "failed to set JWT key permissions: {e}"
                        )))
                    })?;
            }

            tracing::info!("generated new JWT signing key at {}", key_path.display());
            bytes
        };

        Ok(Self::from_secret(&secret))
    }

    /// Create a JwtManager from a raw secret (for tests).
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
        };

        jsonwebtoken::encode(&Header::default(), &claims, &self.encoding_key)
            .map_err(|e| report!(AuthError::JwtEncode(e.to_string())))
    }

    /// Decode and validate a JWT access token.
    pub fn decode_access_token(&self, token: &str) -> Result<AccessTokenClaims> {
        let token_data = jsonwebtoken::decode::<AccessTokenClaims>(
            token,
            &self.decoding_key,
            &Validation::default(),
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

    #[test]
    fn test_load_or_generate_creates_key() {
        let dir = std::env::temp_dir().join(format!("jwt_test_{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();

        let manager = JwtManager::load_or_generate(&dir).unwrap();
        let user_id = uuid::Uuid::now_v7();
        let token = manager
            .create_access_token(user_id, &[], "password", None)
            .unwrap();

        // Load again and verify token still works
        let manager2 = JwtManager::load_or_generate(&dir).unwrap();
        let claims = manager2.decode_access_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
