use super::token::{generate_secure_token, generate_uuid, hash_token};
use super::{AuthError, Result};
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use time::{Duration, OffsetDateTime};
use uptrakit_shared_db::entity::{prelude::*, session};
use uptrakit_shared_types::SessionTokenType;

use super::AuthMethod;

/// Refresh token configuration constants
const REFRESH_TOKEN_EXPIRY_DAYS: i64 = 7;

/// Verified refresh token data returned by `verify_refresh_token`.
pub struct VerifiedSession {
    pub user_id: uuid::Uuid,
    pub auth_method: AuthMethod,
}

pub struct SessionService {
    db: DatabaseConnection,
}

impl SessionService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a new refresh token for a user.
    ///
    /// Returns the plaintext refresh token (only time it's available).
    pub async fn create_refresh_token(
        &self,
        user_id: uuid::Uuid,
        auth_method: AuthMethod,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<String> {
        let token = generate_secure_token()?;
        let token_hash = hash_token(&token);

        let now = OffsetDateTime::now_utc();
        let expires_at = now + Duration::days(REFRESH_TOKEN_EXPIRY_DAYS);

        let session = session::ActiveModel {
            id: Set(generate_uuid()),
            user_id: Set(user_id),
            refresh_token_hash: Set(token_hash),
            auth_method: Set(auth_method.kind().to_string()),
            oidc_provider_id: Set(auth_method.oidc_provider_id()),
            token_type: Set(SessionTokenType::RefreshToken),
            created_at: Set(now),
            expires_at: Set(expires_at),
            revoked_at: Set(None),
            user_agent: Set(user_agent),
            ip_address: Set(ip_address),
        };

        session.insert(&self.db).await.context_to()?;

        Ok(token)
    }

    /// Verify a refresh token and return the verified session info.
    pub async fn verify_refresh_token(&self, token: &str) -> Result<VerifiedSession> {
        let token_hash = hash_token(token);
        let now = OffsetDateTime::now_utc();

        // Query session by refresh_token_hash
        let session = Session::find()
            .filter(session::Column::RefreshTokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .context_to()?
            .ok_or_else(|| report!(AuthError::InvalidRefreshToken))?;

        // Check if revoked
        if session.revoked_at.is_some() {
            bail!(AuthError::RefreshTokenRevoked);
        }

        // Check if expired
        if now >= session.expires_at {
            bail!(AuthError::RefreshTokenExpired);
        }

        let user_id = session.user_id;
        let auth_method = AuthMethod::from_session(&session.auth_method, session.oidc_provider_id)
            .ok_or_else(|| {
                tracing::warn!(
                    user_id = %user_id,
                    auth_method = %session.auth_method,
                    "session has corrupted auth method data; rejecting"
                );
                report!(AuthError::InvalidSession)
            })?;

        Ok(VerifiedSession {
            user_id,
            auth_method,
        })
    }

    /// Atomically rotate a refresh token: revoke the old session and create a new one.
    ///
    /// Returns the verified session info and the new plaintext refresh token.
    /// The old token is immediately revoked so it cannot be reused.
    pub async fn rotate_refresh_token(&self, token: &str) -> Result<(VerifiedSession, String)> {
        let token_hash = hash_token(token);
        let now = OffsetDateTime::now_utc();

        // Wrap find → revoke → insert in a transaction to prevent token-reuse
        // races in multi-controller HA deployments.
        let txn = self.db.begin().await.context_to()?;

        // Find the session by refresh token hash
        let session_model = Session::find()
            .filter(session::Column::RefreshTokenHash.eq(token_hash))
            .one(&txn)
            .await
            .context_to()?
            .ok_or_else(|| report!(AuthError::InvalidRefreshToken))?;

        // Check if already revoked
        if session_model.revoked_at.is_some() {
            bail!(AuthError::RefreshTokenRevoked);
        }

        // Check if expired
        if now >= session_model.expires_at {
            bail!(AuthError::RefreshTokenExpired);
        }

        // Revoke old session
        let old_user_id = session_model.user_id;
        let old_auth_method_str = session_model.auth_method.clone();
        let old_oidc_provider_id = session_model.oidc_provider_id;
        let old_user_agent = session_model.user_agent.clone();
        let old_ip_address = session_model.ip_address.clone();

        let mut old_active: session::ActiveModel = session_model.into();
        old_active.revoked_at = Set(Some(now));
        old_active.update(&txn).await.context_to()?;

        // Create new session with a fresh refresh token
        let auth_method = AuthMethod::from_session(&old_auth_method_str, old_oidc_provider_id)
            .ok_or_else(|| {
                tracing::warn!(
                    user_id = %old_user_id,
                    auth_method = %old_auth_method_str,
                    "session has corrupted auth method data; rejecting rotation"
                );
                report!(AuthError::InvalidSession)
            })?;

        let new_token = generate_secure_token()?;
        let new_hash = hash_token(&new_token);
        let expires_at = now + Duration::days(REFRESH_TOKEN_EXPIRY_DAYS);

        let new_session = session::ActiveModel {
            id: Set(generate_uuid()),
            user_id: Set(old_user_id),
            refresh_token_hash: Set(new_hash),
            auth_method: Set(old_auth_method_str),
            oidc_provider_id: Set(old_oidc_provider_id),
            token_type: Set(SessionTokenType::RefreshToken),
            created_at: Set(now),
            expires_at: Set(expires_at),
            revoked_at: Set(None),
            user_agent: Set(old_user_agent),
            ip_address: Set(old_ip_address),
        };
        new_session.insert(&txn).await.context_to()?;

        txn.commit().await.context_to()?;

        let verified = VerifiedSession {
            user_id: old_user_id,
            auth_method,
        };

        Ok((verified, new_token))
    }

    /// Revoke a refresh token (logout). Sets revoked_at instead of deleting.
    pub async fn revoke_refresh_token(&self, token: &str) -> Result<()> {
        let token_hash = hash_token(token);
        let now = OffsetDateTime::now_utc();

        let session = Session::find()
            .filter(session::Column::RefreshTokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .context_to()?;

        if let Some(session) = session {
            let mut session: session::ActiveModel = session.into();
            session.revoked_at = Set(Some(now));
            session.update(&self.db).await.context_to()?;
        }

        Ok(())
    }

    /// Delete all sessions for a user
    pub async fn delete_user_sessions(&self, user_id: uuid::Uuid) -> Result<()> {
        Session::delete_many()
            .filter(session::Column::UserId.eq(user_id))
            .exec(&self.db)
            .await
            .context_to()?;

        Ok(())
    }

    /// Clean up expired sessions (should be called periodically)
    pub async fn cleanup_expired_sessions(&self) -> Result<u64> {
        let now = OffsetDateTime::now_utc();

        let result = Session::delete_many()
            .filter(session::Column::ExpiresAt.lt(now))
            .exec(&self.db)
            .await
            .context_to()?;

        Ok(result.rows_affected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};
    use uptrakit_shared_db::entity::user;
    use uptrakit_shared_types::MaskedEmail;

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");

        // Insert test user
        let now = OffsetDateTime::now_utc();
        let test_user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("test@example.com")),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        };
        test_user.insert(&db).await.unwrap();

        db
    }

    #[tokio::test]
    async fn test_create_refresh_token() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        // Get test user
        let user = User::find().one(&db).await.unwrap().unwrap();

        let token = service
            .create_refresh_token(
                user.id,
                AuthMethod::Password,
                Some("test-agent".to_string()),
                Some("127.0.0.1".to_string()),
            )
            .await
            .unwrap();

        assert!(!token.is_empty());
        assert_eq!(token.len(), 43); // 32 bytes base64url = 43 chars
    }

    #[tokio::test]
    async fn test_verify_refresh_token_valid() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();
        let user_id = user.id;

        let token = service
            .create_refresh_token(user_id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        let verified = service.verify_refresh_token(&token).await.unwrap();
        assert_eq!(verified.user_id, user_id);
        assert_eq!(verified.auth_method, AuthMethod::Password);
    }

    #[tokio::test]
    async fn test_verify_refresh_token_invalid() {
        let db = setup_test_db().await;
        let service = SessionService::new(db);

        let result = service.verify_refresh_token("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_refresh_token() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let token = service
            .create_refresh_token(user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        // Verify it exists
        assert!(service.verify_refresh_token(&token).await.is_ok());

        // Revoke it
        service.revoke_refresh_token(&token).await.unwrap();

        // Verify it's revoked
        let result = service.verify_refresh_token(&token).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cleanup_expired_sessions() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        // Create an expired session manually
        let token = generate_secure_token().unwrap();
        let token_hash = hash_token(&token);
        let now = OffsetDateTime::now_utc();
        let expired_at = now - Duration::days(1);

        let expired_session = session::ActiveModel {
            id: Set(generate_uuid()),
            user_id: Set(user.id),
            refresh_token_hash: Set(token_hash),
            auth_method: Set("password".to_string()),
            oidc_provider_id: Set(None),
            token_type: Set(SessionTokenType::RefreshToken),
            created_at: Set(now),
            expires_at: Set(expired_at),
            revoked_at: Set(None),
            user_agent: Set(None),
            ip_address: Set(None),
        };
        expired_session.insert(&db).await.unwrap();

        // Clean up expired sessions
        let deleted = service.cleanup_expired_sessions().await.unwrap();
        assert_eq!(deleted, 1);
    }

    #[tokio::test]
    async fn test_rotate_refresh_token_returns_new_token() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let old_token = service
            .create_refresh_token(user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        let (verified, new_token) = service.rotate_refresh_token(&old_token).await.unwrap();
        assert_eq!(verified.user_id, user.id);
        assert_eq!(verified.auth_method, AuthMethod::Password);
        assert_ne!(old_token, new_token, "rotated token must differ from old");
        assert!(!new_token.is_empty());
    }

    #[tokio::test]
    async fn test_rotate_refresh_token_old_is_revoked() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let old_token = service
            .create_refresh_token(user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        let (_verified, _new_token) = service.rotate_refresh_token(&old_token).await.unwrap();

        // Old token must be rejected
        let result = service.verify_refresh_token(&old_token).await;
        assert!(result.is_err(), "old token must be revoked after rotation");
    }

    #[tokio::test]
    async fn test_rotate_refresh_token_new_is_valid() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let old_token = service
            .create_refresh_token(user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        let (_verified, new_token) = service.rotate_refresh_token(&old_token).await.unwrap();

        // New token must be valid
        let verified = service.verify_refresh_token(&new_token).await.unwrap();
        assert_eq!(verified.user_id, user.id);
    }

    #[tokio::test]
    async fn test_rotate_revoked_token_fails() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let token = service
            .create_refresh_token(user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        // Revoke it first
        service.revoke_refresh_token(&token).await.unwrap();

        // Rotating a revoked token must fail
        let result = service.rotate_refresh_token(&token).await;
        assert!(result.is_err(), "rotating a revoked token must fail");
    }

    #[tokio::test]
    async fn test_rotate_same_token_twice_fails() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let token = service
            .create_refresh_token(user.id, AuthMethod::Password, None, None)
            .await
            .unwrap();

        // First rotation succeeds
        let (_verified, _new_token) = service.rotate_refresh_token(&token).await.unwrap();

        // Second rotation of the same token must fail (replay detection)
        let result = service.rotate_refresh_token(&token).await;
        assert!(result.is_err(), "rotating already-rotated token must fail");
    }

    /// The schema enforces `CHECK(auth_method != 'oidc' OR oidc_provider_id IS NOT NULL)`.
    /// Inserting a session with `auth_method = 'oidc'` and a NULL `oidc_provider_id` must be
    /// rejected at the database level, guaranteeing that the application-level verification
    /// code path for corrupted OIDC sessions is never reachable in practice.
    #[tokio::test]
    async fn test_corrupted_oidc_session_rejected_at_db_level() {
        let db = setup_test_db().await;

        let user = User::find().one(&db).await.unwrap().unwrap();
        let token = generate_secure_token().unwrap();
        let token_hash = hash_token(&token);
        let now = OffsetDateTime::now_utc();
        let expires_at = now + Duration::days(REFRESH_TOKEN_EXPIRY_DAYS);

        let corrupted_session = session::ActiveModel {
            id: Set(generate_uuid()),
            user_id: Set(user.id),
            refresh_token_hash: Set(token_hash),
            auth_method: Set("oidc".to_string()),
            oidc_provider_id: Set(None), // violates CHECK constraint
            token_type: Set(SessionTokenType::RefreshToken),
            created_at: Set(now),
            expires_at: Set(expires_at),
            revoked_at: Set(None),
            user_agent: Set(None),
            ip_address: Set(None),
        };

        let result = corrupted_session.insert(&db).await;
        assert!(
            result.is_err(),
            "DB CHECK constraint must prevent inserting a corrupted OIDC session"
        );
    }
}
