//! Service for minting and consuming `oauth_authorization_codes` table rows.
//!
//! Per spec §16. 30-second TTL, single-use consume with `BEGIN IMMEDIATE`.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set, SqliteTransactionMode,
    TransactionOptions, TransactionTrait,
};
use std::sync::Arc;
use thiserror::Error;
use time::{Duration, OffsetDateTime};
use uptrakit_shared_db::entity::oauth_authorization_code;
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_auth::auth::token::hash_token;
use uptrakit_web_api_types::oauth::AuthorizationCode;
use uuid::Uuid;

use crate::oauth::pkce::PkceVerifier;

const TTL_SECONDS: i64 = 30;

/// Errors produced by [`OAuthAuthorizationCodeService`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OAuthCodeError {
    #[error("invalid_grant: {0}")]
    InvalidGrant(&'static str),
    #[error("invalid_target: resource mismatch")]
    InvalidTarget,
    #[error("database error")]
    Database(#[from] sea_orm::DbErr),
}

pub(crate) type Result<T> = std::result::Result<T, Report<OAuthCodeError>>;

impl_report_conversion! {
    sea_orm::DbErr => OAuthCodeError::Database,
}

/// Parameters for minting a new authorization code row.
pub struct MintAuthorizationCode {
    pub request_id: Uuid,
    pub client_id: String,
    pub user_id: Uuid,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub resource: String,
}

/// Service that mints and consumes `oauth_authorization_codes` table rows.
pub struct OAuthAuthorizationCodeService {
    db: sea_orm::DatabaseConnection,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
}

impl OAuthAuthorizationCodeService {
    pub fn new(
        db: sea_orm::DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Self {
        Self { db, clock }
    }

    /// Mint a new single-use authorization code.
    ///
    /// Generates 32 random bytes, base64url-no-padding encodes them, prepends
    /// `"upc_"`, stores the SHA-256 hash, and returns the raw [`AuthorizationCode`].
    ///
    /// TTL is 30 seconds (`expires_at = now + 30 s`).
    pub async fn mint(&self, params: MintAuthorizationCode) -> Result<AuthorizationCode> {
        let now = (self.clock)();
        let expires_at = now + Duration::seconds(TTL_SECONDS);
        let id = Uuid::now_v7();

        // Generate raw code: "upc_" + base64url(32 random bytes).
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        let raw = format!("upc_{}", URL_SAFE_NO_PAD.encode(bytes));

        // Validate prefix via the canonical newtype parser.
        // SAFETY: raw is constructed as `format!("upc_{…}")` — the prefix is always present.
        #[expect(
            clippy::expect_used,
            reason = "raw is constructed with a literal upc_ prefix; parse cannot fail"
        )]
        let code = AuthorizationCode::parse(&raw).expect("generated code always starts with upc_");

        let code_hash = hash_token(&raw);

        let model = oauth_authorization_code::ActiveModel {
            id: Set(id),
            code_hash: Set(code_hash),
            request_id: Set(params.request_id),
            client_id: Set(params.client_id),
            user_id: Set(params.user_id),
            redirect_uri: Set(params.redirect_uri),
            scope: Set(params.scope),
            code_challenge: Set(params.code_challenge),
            code_challenge_method: Set(params.code_challenge_method),
            resource: Set(params.resource),
            issued_at: Set(now),
            expires_at: Set(expires_at),
            consumed_at: Set(None),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(code)
    }

    /// Atomically verify and consume an authorization code.
    ///
    /// Inside a `BEGIN IMMEDIATE` transaction:
    /// 1. Hash `code` with SHA-256.
    /// 2. SELECT row by `code_hash`.
    /// 3. Validate `consumed_at IS NULL`, `expires_at > now`, `client_id`,
    ///    `redirect_uri`, `resource`, and PKCE challenge.
    /// 4. SET `consumed_at = now`.
    ///
    /// Returns the consumed row on success.
    pub async fn verify_and_consume(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        code_verifier: &str,
        resource: &str,
    ) -> Result<oauth_authorization_code::Model> {
        let now = (self.clock)();
        let code_hash = hash_token(code);

        let txn = self
            .db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .context_to()?;

        let row = oauth_authorization_code::Entity::find()
            .filter(oauth_authorization_code::Column::CodeHash.eq(&code_hash))
            .one(&txn)
            .await
            .context_to()?;

        let row = match row {
            Some(r) => r,
            None => bail!(OAuthCodeError::InvalidGrant("code_not_found")),
        };

        if row.consumed_at.is_some() {
            bail!(OAuthCodeError::InvalidGrant("code_already_used"));
        }

        if now >= row.expires_at {
            bail!(OAuthCodeError::InvalidGrant("code_expired"));
        }

        if row.client_id != client_id {
            bail!(OAuthCodeError::InvalidGrant("client_mismatch"));
        }

        if row.redirect_uri != redirect_uri {
            bail!(OAuthCodeError::InvalidGrant("redirect_uri_mismatch"));
        }

        if row.resource != resource {
            bail!(OAuthCodeError::InvalidTarget);
        }

        PkceVerifier::new(row.code_challenge.clone())
            .verify(code_verifier)
            .map_err(|_pkce_err| report!(OAuthCodeError::InvalidGrant("pkce_mismatch")))?;

        let mut active: oauth_authorization_code::ActiveModel = row.clone().into();
        active.consumed_at = Set(Some(now));
        active.update(&txn).await.context_to()?;

        txn.commit().await.context_to()?;

        Ok(row)
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
    use sha2::{Digest, Sha256};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{oauth_authorization_request, oauth_client, user};
    use uptrakit_shared_types::MaskedEmail;

    /// Insert a minimal `oauth_clients` row (required by FK).
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

    /// Insert a minimal `users` row (required by FK).
    async fn insert_user(db: &sea_orm::DatabaseConnection) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();

        user::ActiveModel {
            id: Set(id),
            email: Set(MaskedEmail::new("testuser@example.com")),
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

    /// Insert a minimal `oauth_authorization_requests` row (required by FK).
    async fn insert_oauth_authorization_request(
        db: &sea_orm::DatabaseConnection,
        client_id: &str,
        user_id: Uuid,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let request_id = Uuid::now_v7();

        oauth_authorization_request::ActiveModel {
            request_id: Set(request_id),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            redirect_uri: Set("https://example.com/callback".to_string()),
            scope: Set("openid".to_string()),
            state: Set("test-state".to_string()),
            code_challenge: Set("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".to_string()),
            code_challenge_method: Set("S256".to_string()),
            resource: Set("https://resource.example.com".to_string()),
            created_at: Set(now),
            expires_at: Set(now + Duration::seconds(600)),
            consumed_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_authorization_request");

        request_id
    }

    /// Build a clock function driven by `Arc<Mutex<OffsetDateTime>>`.
    fn make_clock(
        cell: Arc<Mutex<OffsetDateTime>>,
    ) -> Arc<dyn Fn() -> OffsetDateTime + Send + Sync> {
        Arc::new(move || *cell.lock())
    }

    /// Build a PKCE code challenge from a verifier using S256.
    fn make_pkce_challenge(verifier: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(verifier.as_bytes());
        let digest = hasher.finalize();
        URL_SAFE_NO_PAD.encode(digest)
    }

    fn make_mint_params(
        request_id: Uuid,
        client_id: String,
        user_id: Uuid,
        code_challenge: String,
    ) -> MintAuthorizationCode {
        MintAuthorizationCode {
            request_id,
            client_id,
            user_id,
            redirect_uri: "https://example.com/callback".to_string(),
            scope: "openid".to_string(),
            code_challenge,
            code_challenge_method: "S256".to_string(),
            resource: "https://resource.example.com".to_string(),
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 1 — mint produces a upc_-prefixed code whose hash differs from raw
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mint_produces_upc_prefixed_code() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(request_id, client_id, user_id, challenge))
            .await
            .expect("mint should succeed");

        let raw = code.as_str();
        assert!(raw.starts_with("upc_"), "code must begin with upc_");

        // Hash must differ from the raw code itself.
        let stored_hash = hash_token(raw);
        assert_ne!(raw, stored_hash, "hash must differ from raw code");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 2 — happy path within TTL
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_and_consume_succeeds_within_ttl() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(
                request_id,
                client_id.clone(),
                user_id,
                challenge,
            ))
            .await
            .expect("mint should succeed");

        let row = svc
            .verify_and_consume(
                code.as_str(),
                &client_id,
                "https://example.com/callback",
                verifier,
                "https://resource.example.com",
            )
            .await
            .expect("verify_and_consume should succeed within TTL");

        assert_eq!(row.client_id, client_id);
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 3 — fails after 30 s TTL has elapsed
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_and_consume_fails_after_expiry() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(
                request_id,
                client_id.clone(),
                user_id,
                challenge,
            ))
            .await
            .expect("mint should succeed");

        // Advance clock past the 30 s TTL.
        *clock_cell.lock() += Duration::seconds(31);

        let err = svc
            .verify_and_consume(
                code.as_str(),
                &client_id,
                "https://example.com/callback",
                verifier,
                "https://resource.example.com",
            )
            .await
            .expect_err("must fail after expiry");

        assert!(matches!(
            err.current_context(),
            OAuthCodeError::InvalidGrant("code_expired")
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 4 — second consume returns already_used
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_and_consume_fails_on_double_redeem() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(
                request_id,
                client_id.clone(),
                user_id,
                challenge,
            ))
            .await
            .expect("mint should succeed");

        // First consume — must succeed.
        svc.verify_and_consume(
            code.as_str(),
            &client_id,
            "https://example.com/callback",
            verifier,
            "https://resource.example.com",
        )
        .await
        .expect("first consume must succeed");

        // Second consume — must fail.
        let err = svc
            .verify_and_consume(
                code.as_str(),
                &client_id,
                "https://example.com/callback",
                verifier,
                "https://resource.example.com",
            )
            .await
            .expect_err("second consume must fail");

        assert!(matches!(
            err.current_context(),
            OAuthCodeError::InvalidGrant("code_already_used")
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 5 — wrong PKCE verifier
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_and_consume_fails_on_pkce_mismatch() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(
                request_id,
                client_id.clone(),
                user_id,
                challenge,
            ))
            .await
            .expect("mint should succeed");

        let err = svc
            .verify_and_consume(
                code.as_str(),
                &client_id,
                "https://example.com/callback",
                "wrong-verifier",
                "https://resource.example.com",
            )
            .await
            .expect_err("must fail on pkce mismatch");

        assert!(matches!(
            err.current_context(),
            OAuthCodeError::InvalidGrant("pkce_mismatch")
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 6 — redirect_uri mismatch
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_and_consume_fails_on_redirect_uri_mismatch() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(
                request_id,
                client_id.clone(),
                user_id,
                challenge,
            ))
            .await
            .expect("mint should succeed");

        let err = svc
            .verify_and_consume(
                code.as_str(),
                &client_id,
                "https://evil.example.com/callback",
                verifier,
                "https://resource.example.com",
            )
            .await
            .expect_err("must fail on redirect_uri mismatch");

        assert!(matches!(
            err.current_context(),
            OAuthCodeError::InvalidGrant("redirect_uri_mismatch")
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 7 — resource mismatch → InvalidTarget
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn verify_and_consume_fails_on_resource_mismatch() {
        let db = setup_migrated_db().await;
        let client_id = insert_oauth_client(&db).await;
        let user_id = insert_user(&db).await;
        let request_id = insert_oauth_authorization_request(&db, &client_id, user_id).await;

        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = make_pkce_challenge(verifier);

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthAuthorizationCodeService::new(db, make_clock(Arc::clone(&clock_cell)));

        let code = svc
            .mint(make_mint_params(
                request_id,
                client_id.clone(),
                user_id,
                challenge,
            ))
            .await
            .expect("mint should succeed");

        let err = svc
            .verify_and_consume(
                code.as_str(),
                &client_id,
                "https://example.com/callback",
                verifier,
                "https://other-resource.example.com",
            )
            .await
            .expect_err("must fail on resource mismatch");

        assert!(matches!(
            err.current_context(),
            OAuthCodeError::InvalidTarget
        ));
    }
}
