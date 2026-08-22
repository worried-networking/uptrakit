//! Service for managing `oauth_authorization_requests` table rows.
//!
//! Per spec §12.2: single-use consume with `BEGIN IMMEDIATE`; 10-minute TTL.

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};
use std::sync::Arc;
use thiserror::Error;
use time::{Duration, OffsetDateTime};
use uptrakit_shared_db::begin_immediate;
use uptrakit_shared_db::entity::oauth_authorization_request;
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

const TTL_SECONDS: i64 = 600;

/// Errors produced by [`OAuthAuthorizationRequestService`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum AuthorizationRequestError {
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),

    #[error("authorization request not found, expired, or already consumed")]
    NotFound,
}

pub(crate) type Result<T> = std::result::Result<T, Report<AuthorizationRequestError>>;

impl_report_conversion! {
    sea_orm::DbErr => AuthorizationRequestError::Database,
}

/// Parameters for creating a new authorization request row.
pub struct CreateAuthorizationRequest {
    pub client_id: String,
    pub user_id: Uuid,
    pub redirect_uri: String,
    pub scope: String,
    pub state: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub resource: String,
}

/// Service that manages `oauth_authorization_requests` table rows.
pub struct OAuthAuthorizationRequestService {
    db: sea_orm::DatabaseConnection,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
}

impl OAuthAuthorizationRequestService {
    pub fn new(
        db: sea_orm::DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Self {
        Self { db, clock }
    }

    /// Insert a new authorization request row.
    ///
    /// Sets `expires_at = now + 600 s` and returns the generated `request_id`.
    pub async fn create(&self, params: CreateAuthorizationRequest) -> Result<Uuid> {
        let now = (self.clock)();
        let expires_at = now + Duration::seconds(TTL_SECONDS);
        let request_id = Uuid::now_v7();

        let model = oauth_authorization_request::ActiveModel {
            request_id: Set(request_id),
            client_id: Set(params.client_id),
            user_id: Set(params.user_id),
            redirect_uri: Set(params.redirect_uri),
            scope: Set(params.scope),
            state: Set(params.state),
            code_challenge: Set(params.code_challenge),
            code_challenge_method: Set(params.code_challenge_method),
            resource: Set(params.resource),
            created_at: Set(now),
            expires_at: Set(expires_at),
            consumed_at: Set(None),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(request_id)
    }

    /// Atomically consume an authorization request.
    ///
    /// Inside a `BEGIN IMMEDIATE` transaction: SELECTs the row, checks that
    /// `consumed_at IS NULL` and `expires_at > now`, then sets
    /// `consumed_at = now`.
    ///
    /// Returns `Some(model)` on success, `None` if the request is expired,
    /// already consumed, or does not exist.
    pub async fn consume(
        &self,
        request_id: Uuid,
    ) -> Result<Option<oauth_authorization_request::Model>> {
        let now = (self.clock)();

        let txn = begin_immediate(&self.db).await.context_to()?;

        let row = oauth_authorization_request::Entity::find()
            .filter(oauth_authorization_request::Column::RequestId.eq(request_id))
            .one(&txn)
            .await
            .context_to()?;

        let row = match row {
            Some(r) => r,
            None => {
                txn.commit().await.context_to()?;
                return Ok(None);
            }
        };

        // Reject if already consumed or expired.
        if row.consumed_at.is_some() || now >= row.expires_at {
            txn.commit().await.context_to()?;
            return Ok(None);
        }

        // Mark as consumed.
        let mut active: oauth_authorization_request::ActiveModel = row.clone().into();
        active.consumed_at = Set(Some(now));
        active.update(&txn).await.context_to()?;

        txn.commit().await.context_to()?;

        Ok(Some(row))
    }
}

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test assertions — panics on setup failure are acceptable in tests"
    )]

    use super::*;
    use crate::test_harness::setup_migrated_db;
    use parking_lot::Mutex;
    use sea_orm::{ActiveModelTrait, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{oauth_client, user};

    /// Insert a minimal `oauth_clients` row (required by FK on `oauth_authorization_requests`).
    async fn insert_oauth_client(db: &sea_orm::DatabaseConnection) -> String {
        let now = OffsetDateTime::now_utc();
        let client_id = format!("test-client-{}", Uuid::now_v7());

        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Test Client".to_string()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set("https://example.com/callback".to_string()),
            default_scope: Set("openid".to_string()),
            grant_types: Set("authorization_code".to_string()),
            response_types: Set("code".to_string()),
            token_endpoint_auth_method: Set("none".to_string()),
            client_secret_hash: Set(None),
            registration_access_token_hash: Set(None),
            created_via: Set("test".to_string()),
            created_at: Set(now),
            last_used_at: Set(None),
            revoked_at: Set(None),
            metadata_cached_at: Set(None),
            metadata_etag: Set(None),
            metadata_content_hash: Set(None),
            metadata_raw: Set(None),
            metadata_parse_error: Set(None),
            metadata_parse_error_at: Set(None),
            trusted_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_client");

        client_id
    }

    /// Insert a minimal `users` row (required by FK on `oauth_authorization_requests`).
    async fn insert_user(db: &sea_orm::DatabaseConnection) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        user::ActiveModel {
            id: Set(id),
            email: Set("testuser@example.com".parse().expect("valid test email")),
            first_name: Set("Test".to_string()),
            last_name: Set("User".to_string()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user");

        id
    }

    /// Build a clock function driven by `Arc<Mutex<OffsetDateTime>>`.
    fn make_clock(
        cell: Arc<Mutex<OffsetDateTime>>,
    ) -> Arc<dyn Fn() -> OffsetDateTime + Send + Sync> {
        Arc::new(move || *cell.lock())
    }

    fn make_params(client_id: String, user_id: Uuid) -> CreateAuthorizationRequest {
        CreateAuthorizationRequest {
            client_id,
            user_id,
            redirect_uri: "https://example.com/callback".to_string(),
            scope: "openid".to_string(),
            state: "test-state".to_string(),
            code_challenge: "dGVzdC1jaGFsbGVuZ2U".to_string(),
            code_challenge_method: "S256".to_string(),
            resource: "https://resource.example.com".to_string(),
        }
    }

    #[tokio::test]
    async fn insert_then_consume_succeeds_before_ttl() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationRequestService::new(db, make_clock(Arc::clone(&clock_cell)));

        let request_id = svc
            .create(make_params(client_id, user_id))
            .await
            .expect("create should succeed");

        let result = svc
            .consume(request_id)
            .await
            .expect("consume should not DB-error");
        assert!(result.is_some(), "consume before TTL must return Some(row)");
        assert_eq!(result.unwrap().request_id, request_id);
    }

    #[tokio::test]
    async fn consume_after_expiry_returns_none() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationRequestService::new(db, make_clock(Arc::clone(&clock_cell)));

        let request_id = svc
            .create(make_params(client_id, user_id))
            .await
            .expect("create should succeed");

        // Advance clock past the 600 s TTL.
        *clock_cell.lock() += Duration::seconds(601);

        let result = svc
            .consume(request_id)
            .await
            .expect("consume should not DB-error");
        assert!(result.is_none(), "consume after expiry must return None");
    }

    #[tokio::test]
    async fn double_consume_returns_none_second_time() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationRequestService::new(db, make_clock(Arc::clone(&clock_cell)));

        let request_id = svc
            .create(make_params(client_id, user_id))
            .await
            .expect("create should succeed");

        // First consume — must succeed.
        let first = svc.consume(request_id).await.expect("first consume DB-ok");
        assert!(first.is_some(), "first consume must return Some(row)");

        // Second consume — must return None (already consumed).
        let second = svc.consume(request_id).await.expect("second consume DB-ok");
        assert!(second.is_none(), "second consume must return None");
    }
}
