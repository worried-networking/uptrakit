use super::token::{generate_secure_token, generate_uuid, hash_token};
use super::{AuthError, Result};
use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use time::{Duration, OffsetDateTime};
use uptrakit_shared_db::entity::{prelude::*, session};

/// Session configuration constants
const SESSION_EXPIRY_DAYS: i64 = 7;
const SESSION_SLIDING_WINDOW_MINUTES: i64 = 30;

pub struct SessionService {
    db: DatabaseConnection,
}

impl SessionService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a new session for a user
    ///
    /// Returns the plaintext session token (only time it's available)
    pub async fn create_session(
        &self,
        user_id: uuid::Uuid,
        auth_method: String,
        user_agent: Option<String>,
        ip_address: Option<String>,
    ) -> Result<String> {
        let token = generate_secure_token()?;
        let token_hash = hash_token(&token);

        let now = OffsetDateTime::now_utc();
        let expires_at = now + Duration::days(SESSION_EXPIRY_DAYS);

        let session = session::ActiveModel {
            id: Set(generate_uuid()),
            user_id: Set(user_id),
            token_hash: Set(token_hash),
            auth_method: Set(auth_method),
            created_at: Set(now),
            expires_at: Set(expires_at),
            last_activity_at: Set(now),
            user_agent: Set(user_agent),
            ip_address: Set(ip_address),
        };

        session.insert(&self.db).await.context_to()?;

        Ok(token)
    }

    /// Verify a session token and return the user_id if valid
    ///
    /// Also updates last_activity_at if within sliding window
    pub async fn verify_session(&self, token: &str) -> Result<uuid::Uuid> {
        let token_hash = hash_token(token);
        let now = OffsetDateTime::now_utc();

        // Query session by token_hash
        let session = Session::find()
            .filter(session::Column::TokenHash.eq(token_hash.clone()))
            .one(&self.db)
            .await
            .context_to()?
            .ok_or_else(|| report!(AuthError::SessionExpired))?;

        // Check if expired
        if now >= session.expires_at {
            return Err(report!(AuthError::SessionExpired));
        }

        // Save user_id before potentially moving session
        let user_id = session.user_id;

        // Update last_activity_at if within sliding window
        let time_since_activity = now - session.last_activity_at;
        if time_since_activity >= Duration::minutes(SESSION_SLIDING_WINDOW_MINUTES) {
            let mut session: session::ActiveModel = session.into();
            session.last_activity_at = Set(now);
            session.update(&self.db).await.context_to()?;
        }

        Ok(user_id)
    }

    /// Delete a session (logout)
    pub async fn delete_session(&self, token: &str) -> Result<()> {
        let token_hash = hash_token(token);

        Session::delete_many()
            .filter(session::Column::TokenHash.eq(token_hash))
            .exec(&self.db)
            .await
            .context_to()?;

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
    use sea_orm::{ConnectOptions, ConnectionTrait, Database};
    use uptrakit_shared_db::entity::user;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn setup_test_db() -> DatabaseConnection {
        let db = test_db().await;

        // Create tables (using raw SQL for tests)
        db.execute_unprepared(
            "CREATE TABLE users (
                id TEXT PRIMARY KEY,
                email TEXT UNIQUE NOT NULL,
                first_name TEXT NOT NULL,
                last_name TEXT NOT NULL,
                password_hash TEXT,
                is_active INTEGER NOT NULL DEFAULT 1,
                deactivated_at INTEGER,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            )",
        )
        .await
        .unwrap();

        db.execute_unprepared(
            "CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                token_hash TEXT UNIQUE NOT NULL,
                auth_method TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                last_activity_at INTEGER NOT NULL,
                user_agent TEXT,
                ip_address TEXT,
                FOREIGN KEY (user_id) REFERENCES users(id)
            )",
        )
        .await
        .unwrap();

        // Insert test user using entity
        let now = OffsetDateTime::now_utc();
        let test_user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set("test@example.com".to_string()),
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
    async fn test_create_session() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        // Get test user
        let user = User::find().one(&db).await.unwrap().unwrap();

        let token = service
            .create_session(
                user.id,
                "password".to_string(),
                Some("test-agent".to_string()),
                Some("127.0.0.1".to_string()),
            )
            .await
            .unwrap();

        assert!(!token.is_empty());
        assert_eq!(token.len(), 43); // 32 bytes base64url = 43 chars
    }

    #[tokio::test]
    async fn test_verify_session_valid() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();
        let user_id = user.id;

        let token = service
            .create_session(user_id, "password".to_string(), None, None)
            .await
            .unwrap();

        let verified_user_id = service.verify_session(&token).await.unwrap();
        assert_eq!(verified_user_id, user_id);
    }

    #[tokio::test]
    async fn test_verify_session_invalid_token() {
        let db = setup_test_db().await;
        let service = SessionService::new(db);

        let result = service.verify_session("invalid-token").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_session() {
        let db = setup_test_db().await;
        let service = SessionService::new(db.clone());

        let user = User::find().one(&db).await.unwrap().unwrap();

        let token = service
            .create_session(user.id, "password".to_string(), None, None)
            .await
            .unwrap();

        // Verify it exists
        assert!(service.verify_session(&token).await.is_ok());

        // Delete it
        service.delete_session(&token).await.unwrap();

        // Verify it's gone
        let result = service.verify_session(&token).await;
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
            token_hash: Set(token_hash),
            auth_method: Set("password".to_string()),
            created_at: Set(now),
            expires_at: Set(expired_at),
            last_activity_at: Set(now),
            user_agent: Set(None),
            ip_address: Set(None),
        };
        expired_session.insert(&db).await.unwrap();

        // Clean up expired sessions
        let deleted = service.cleanup_expired_sessions().await.unwrap();
        assert_eq!(deleted, 1);
    }
}
