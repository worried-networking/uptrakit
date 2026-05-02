use super::token::{generate_secure_token, generate_uuid, hash_token};
use super::{AuthError, Result};
use async_trait::async_trait;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, QueryOrder, Set,
};
use serde::Serialize;
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{api_token, prelude::*};

/// Public information about an API token (no secrets).
#[derive(Debug, Serialize)]
pub struct ApiTokenInfo {
    pub id: uuid::Uuid,
    pub name: String,
    pub created_at: OffsetDateTime,
    pub last_used_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
}

/// Result of creating a new API token.
pub struct CreatedApiToken {
    pub id: uuid::Uuid,
    pub plaintext_token: String,
    pub created_at: OffsetDateTime,
}

pub struct ApiTokenService {
    db: DatabaseConnection,
}

impl ApiTokenService {
    pub fn new(db: DatabaseConnection) -> Self {
        Self { db }
    }

    /// Create a new API token for a user.
    ///
    /// Returns the plaintext token (only time it's available).
    /// The token is prefixed with `upk_` for identification.
    pub async fn create_token(&self, user_id: uuid::Uuid, name: &str) -> Result<CreatedApiToken> {
        let raw_token = generate_secure_token()?;
        let plaintext = format!("upk_{raw_token}");
        let token_hash = hash_token(&plaintext);
        let now = OffsetDateTime::now_utc();
        let id = generate_uuid();

        let model = api_token::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            name: Set(name.to_string()),
            token_hash: Set(token_hash),
            created_at: Set(now),
            last_used_at: Set(None),
            revoked_at: Set(None),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(CreatedApiToken {
            id,
            plaintext_token: plaintext,
            created_at: now,
        })
    }

    /// List all tokens for a user (no secrets).
    pub async fn list_tokens(&self, user_id: uuid::Uuid) -> Result<Vec<ApiTokenInfo>> {
        let tokens = ApiToken::find()
            .filter(api_token::Column::UserId.eq(user_id))
            .order_by_desc(api_token::Column::CreatedAt)
            .all(&self.db)
            .await
            .context_to()?;

        Ok(tokens
            .into_iter()
            .map(|t| ApiTokenInfo {
                id: t.id,
                name: t.name,
                created_at: t.created_at,
                last_used_at: t.last_used_at,
                revoked_at: t.revoked_at,
            })
            .collect())
    }

    /// Revoke a token. Only the owning user can revoke.
    pub async fn revoke_token(&self, token_id: uuid::Uuid, user_id: uuid::Uuid) -> Result<()> {
        let token = ApiToken::find_by_id(token_id)
            .filter(api_token::Column::UserId.eq(user_id))
            .one(&self.db)
            .await
            .context_to()?
            .ok_or_else(|| report!(AuthError::ApiTokenNotFound))?;

        let mut model: api_token::ActiveModel = token.into();
        model.revoked_at = Set(Some(OffsetDateTime::now_utc()));
        model.update(&self.db).await.context_to()?;

        Ok(())
    }

    /// Verify a plaintext API token. Returns (user_id, token_id) on success.
    ///
    /// Also updates `last_used_at`.
    pub async fn verify_token(&self, plaintext: &str) -> Result<(uuid::Uuid, uuid::Uuid)> {
        let token_hash = hash_token(plaintext);

        let token = ApiToken::find()
            .filter(api_token::Column::TokenHash.eq(token_hash))
            .one(&self.db)
            .await
            .context_to()?
            .ok_or_else(|| report!(AuthError::ApiTokenNotFound))?;

        if token.revoked_at.is_some() {
            bail!(AuthError::ApiTokenRevoked);
        }

        // Update last_used_at
        let token_id = token.id;
        let user_id = token.user_id;
        let mut model: api_token::ActiveModel = token.into();
        model.last_used_at = Set(Some(OffsetDateTime::now_utc()));
        model.update(&self.db).await.context_to()?;

        Ok((user_id, token_id))
    }
}

/// Service trait for API token operations, enabling controller embedding without
/// a dependency on `uptrakit-web-api` or Axum.
///
/// Implemented by [`ApiTokenService`]. The `#[async_trait]` desugaring ensures
/// `Arc<dyn ApiTokenOps>` is dyn-safe.
#[async_trait]
pub trait ApiTokenOps: Send + Sync {
    async fn create_token(&self, user_id: uuid::Uuid, name: &str) -> Result<CreatedApiToken>;
    async fn list_tokens(&self, user_id: uuid::Uuid) -> Result<Vec<ApiTokenInfo>>;
    async fn revoke_token(&self, token_id: uuid::Uuid, user_id: uuid::Uuid) -> Result<()>;
    async fn verify_token(&self, plaintext: &str) -> Result<(uuid::Uuid, uuid::Uuid)>;
}

#[async_trait]
impl ApiTokenOps for ApiTokenService {
    async fn create_token(&self, user_id: uuid::Uuid, name: &str) -> Result<CreatedApiToken> {
        ApiTokenService::create_token(self, user_id, name).await
    }

    async fn list_tokens(&self, user_id: uuid::Uuid) -> Result<Vec<ApiTokenInfo>> {
        ApiTokenService::list_tokens(self, user_id).await
    }

    async fn revoke_token(&self, token_id: uuid::Uuid, user_id: uuid::Uuid) -> Result<()> {
        ApiTokenService::revoke_token(self, token_id, user_id).await
    }

    async fn verify_token(&self, plaintext: &str) -> Result<(uuid::Uuid, uuid::Uuid)> {
        ApiTokenService::verify_token(self, plaintext).await
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok/is_err()) is idiomatic in tests"
    )]

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
    async fn test_create_token() {
        let db = setup_test_db().await;
        let service = ApiTokenService::new(db.clone());
        let user = User::find().one(&db).await.unwrap().unwrap();

        let created = service.create_token(user.id, "test-token").await.unwrap();

        assert!(created.plaintext_token.starts_with("upk_"));
        assert_eq!(created.plaintext_token.len(), 47); // "upk_" + 43 chars
    }

    #[tokio::test]
    async fn test_list_tokens() {
        let db = setup_test_db().await;
        let service = ApiTokenService::new(db.clone());
        let user = User::find().one(&db).await.unwrap().unwrap();

        service.create_token(user.id, "token-1").await.unwrap();
        service.create_token(user.id, "token-2").await.unwrap();

        let tokens = service.list_tokens(user.id).await.unwrap();
        assert_eq!(tokens.len(), 2);
    }

    #[tokio::test]
    async fn test_verify_token() {
        let db = setup_test_db().await;
        let service = ApiTokenService::new(db.clone());
        let user = User::find().one(&db).await.unwrap().unwrap();

        let created = service.create_token(user.id, "test").await.unwrap();

        let (uid, tid) = service
            .verify_token(&created.plaintext_token)
            .await
            .unwrap();
        assert_eq!(uid, user.id);
        assert_eq!(tid, created.id);
    }

    #[tokio::test]
    async fn test_revoke_and_verify_fails() {
        let db = setup_test_db().await;
        let service = ApiTokenService::new(db.clone());
        let user = User::find().one(&db).await.unwrap().unwrap();

        let created = service.create_token(user.id, "test").await.unwrap();
        service.revoke_token(created.id, user.id).await.unwrap();

        let result = service.verify_token(&created.plaintext_token).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_invalid_token() {
        let db = setup_test_db().await;
        let service = ApiTokenService::new(db.clone());

        let result = service.verify_token("upk_invalid").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_revoke_other_users_token_fails() {
        let db = setup_test_db().await;
        let service = ApiTokenService::new(db.clone());
        let user = User::find().one(&db).await.unwrap().unwrap();

        let created = service.create_token(user.id, "test").await.unwrap();

        let other_user_id = generate_uuid();
        let result = service.revoke_token(created.id, other_user_id).await;
        assert!(result.is_err());
    }
}

#[cfg(test)]
mod controller_di_tests {
    #![expect(
        clippy::unimplemented,
        clippy::assertions_on_result_states,
        reason = "test stubs — unimplemented! in unused mock methods and assert!(result.is_ok()) are idiomatic in test stubs"
    )]

    use super::*;
    use async_trait::async_trait;
    use std::sync::Arc;

    struct MockApiTokenOps;

    #[async_trait]
    impl ApiTokenOps for MockApiTokenOps {
        async fn create_token(&self, _user_id: uuid::Uuid, _name: &str) -> Result<CreatedApiToken> {
            unimplemented!("mock")
        }

        async fn list_tokens(&self, _user_id: uuid::Uuid) -> Result<Vec<ApiTokenInfo>> {
            Ok(vec![])
        }

        async fn revoke_token(&self, _token_id: uuid::Uuid, _user_id: uuid::Uuid) -> Result<()> {
            Ok(())
        }

        async fn verify_token(&self, _plaintext: &str) -> Result<(uuid::Uuid, uuid::Uuid)> {
            Ok((uuid::Uuid::new_v4(), uuid::Uuid::new_v4()))
        }
    }

    struct TestController {
        token_ops: Arc<dyn ApiTokenOps>,
    }

    #[tokio::test]
    async fn mock_api_token_ops_injection() {
        let mock: Arc<dyn ApiTokenOps> = Arc::new(MockApiTokenOps);
        let controller = TestController { token_ops: mock };
        let result = controller.token_ops.verify_token("test").await;
        assert!(result.is_ok());
    }
}
