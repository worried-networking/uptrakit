use super::token::{generate_secure_token, generate_uuid, hash_token};
use super::{AuthError, Result};
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, Database};
    use uptrakit_shared_db::MaskedEmail;
    use uptrakit_shared_db::entity::user;

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");

        let now = OffsetDateTime::now_utc();
        let test_user = user::ActiveModel {
            id: Set(generate_uuid()),
            email: Set(MaskedEmail::new("test@example.com".to_string())),
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
