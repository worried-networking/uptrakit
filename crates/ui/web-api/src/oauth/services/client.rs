//! Service for managing `oauth_clients` table rows.
//!
//! Covers Dynamic Client Registration (RFC 7591), operator-initiated manual
//! registration, lookup, revocation with cascade, and trust promotion.
//!
//! Per spec §11.2 / §11.4 + §10.5 cascade rules.

use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    Set, TransactionTrait,
};
use std::sync::Arc;
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_audit_log::{AuditActionType, AuditEmitter, AuditEntry, AuditOutcome, Event};
use uptrakit_shared_db::entity::{oauth_client, oauth_consent, oauth_refresh_token};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_auth::auth::token::hash_token;
use uptrakit_web_api_types::oauth::responses::{DcrRegistrationRequest, DcrRegistrationResponse};
use uuid::Uuid;

use crate::oauth::http_responses::{oauth_403, oauth_500};

/// Build the RFC 6749 response for a DCR (`POST /oauth/register`) failure.
///
/// Sanctioned RFC 6749 exit for the DCR endpoint — keeps the
/// `match e.current_context()` pattern out of `crates/ui/web-api/src/routes/` per
/// the `check_legacy_error_matches.sh` gate. See `docs/development/error-handling.md`
/// Pattern 18.
pub(crate) fn registration_error_to_response(e: &Report<OAuthClientError>) -> Response {
    match e.current_context() {
        OAuthClientError::RegistrationCapExceeded => {
            oauth_403("registration_not_allowed", "per-IP lifetime cap exceeded")
        }
        OAuthClientError::Database(_) | OAuthClientError::NotFound => {
            tracing::error!(error = %e, "DCR registration failed");
            oauth_500()
        }
    }
}

/// Errors produced by [`OAuthClientService`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OAuthClientError {
    #[error("client not found")]
    NotFound,
    #[error("DCR per-IP lifetime cap exceeded")]
    RegistrationCapExceeded,
    #[error("database error")]
    Database(sea_orm::DbErr),
}

pub(crate) type Result<T> = std::result::Result<T, Report<OAuthClientError>>;

impl_report_conversion! {
    sea_orm::DbErr => OAuthClientError::Database,
}

/// Maximum number of DCR-registered clients allowed.
const DCR_CLIENT_CAP: u64 = 20;

/// Service that manages `oauth_clients` table rows.
pub struct OAuthClientService {
    db: DatabaseConnection,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    audit_emitter: Arc<AuditEmitter>,
}

impl OAuthClientService {
    /// Construct a new client service.
    pub fn new(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
        audit_emitter: Arc<AuditEmitter>,
    ) -> Self {
        Self {
            db,
            clock,
            audit_emitter,
        }
    }

    /// Register a new client via Dynamic Client Registration (RFC 7591).
    ///
    /// Enforces a lifetime cap of 20 total DCR registrations. On cap-exceeded
    /// an `OAUTH_CLIENT_REGISTRATION_RATE_LIMITED` audit entry is emitted and
    /// [`OAuthClientError::RegistrationCapExceeded`] is returned.
    pub async fn register_dcr(
        &self,
        req: DcrRegistrationRequest,
        _source_ip: std::net::IpAddr,
        _controller_secret: &[u8],
    ) -> Result<DcrRegistrationResponse> {
        // Cap check: count all DCR clients ever registered.
        let count = oauth_client::Entity::find()
            .filter(oauth_client::Column::CreatedVia.eq("dcr"))
            .count(&self.db)
            .await
            .context_to()?;

        if count >= DCR_CLIENT_CAP {
            self.emit_rate_limited_audit();
            bail!(OAuthClientError::RegistrationCapExceeded);
        }

        self.do_register(req, "dcr").await
    }

    /// Register a new client by an operator (no cap check).
    pub async fn register_manual(
        &self,
        req: DcrRegistrationRequest,
    ) -> Result<DcrRegistrationResponse> {
        self.do_register(req, "manual").await
    }

    /// Look up a client by `client_id`.
    pub async fn lookup(&self, client_id: &str) -> Result<Option<oauth_client::Model>> {
        let row = oauth_client::Entity::find_by_id(client_id.to_string())
            .one(&self.db)
            .await
            .context_to()?;
        Ok(row)
    }

    /// Revoke a client and cascade-revoke all its active consents and refresh tokens.
    ///
    /// Multi-statement atomic per §10.5: wrapped in a single transaction.
    pub async fn revoke(&self, client_id: &str) -> Result<()> {
        let now = (self.clock)();

        let txn = self.db.begin().await.context_to()?;

        let result = oauth_client::Entity::update_many()
            .col_expr(
                oauth_client::Column::RevokedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(oauth_client::Column::Id.eq(client_id))
            .filter(oauth_client::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            bail!(OAuthClientError::NotFound);
        }

        oauth_consent::Entity::update_many()
            .col_expr(
                oauth_consent::Column::RevokedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(oauth_consent::Column::ClientId.eq(client_id))
            .filter(oauth_consent::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        oauth_refresh_token::Entity::update_many()
            .col_expr(
                oauth_refresh_token::Column::RevokedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(oauth_refresh_token::Column::ClientId.eq(client_id))
            .filter(oauth_refresh_token::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;
        Ok(())
    }

    /// Promote a client to trusted status by setting `trusted_at = now`.
    ///
    /// Returns [`OAuthClientError::NotFound`] when no rows were updated.
    pub async fn promote_trusted(&self, client_id: &str) -> Result<()> {
        let now = (self.clock)();

        let result = oauth_client::Entity::update_many()
            .col_expr(
                oauth_client::Column::TrustedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(oauth_client::Column::Id.eq(client_id))
            .exec(&self.db)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            bail!(OAuthClientError::NotFound);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Shared INSERT logic for both DCR and manual registration.
    async fn do_register(
        &self,
        req: DcrRegistrationRequest,
        created_via: &str,
    ) -> Result<DcrRegistrationResponse> {
        let now = (self.clock)();

        let client_id = Uuid::new_v4().to_string();

        // Generate the registration access token (32 random bytes, base64url-no-pad).
        let mut bytes = [0u8; 32];
        rand::rng().fill(&mut bytes);
        let registration_access_token = URL_SAFE_NO_PAD.encode(bytes);
        let registration_access_token_hash = hash_token(&registration_access_token);

        // Serialize array fields to JSON text for the DB columns.
        let redirect_uris_json =
            serde_json::to_string(&req.redirect_uris).unwrap_or_else(|_| "[]".to_string());
        let grant_types_json =
            serde_json::to_string(&req.grant_types).unwrap_or_else(|_| "[]".to_string());
        let response_types_json =
            serde_json::to_string(&req.response_types).unwrap_or_else(|_| "[]".to_string());
        let default_scope = req.scope.clone().unwrap_or_default();

        let model = oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set(req.client_name.clone()),
            client_uri: Set(req.client_uri.clone()),
            logo_uri: Set(req.logo_uri.clone()),
            redirect_uris: Set(redirect_uris_json),
            default_scope: Set(default_scope.clone()),
            grant_types: Set(grant_types_json),
            response_types: Set(response_types_json),
            token_endpoint_auth_method: Set(req.token_endpoint_auth_method.clone()),
            client_secret_hash: Set(None),
            registration_access_token_hash: Set(Some(registration_access_token_hash)),
            created_via: Set(created_via.to_string()),
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
        };

        model.insert(&self.db).await.context_to()?;

        let response = DcrRegistrationResponse::new(
            client_id.clone(),
            now.unix_timestamp(),
            Some(registration_access_token),
            format!("/oauth/register/{client_id}"),
            req.client_name,
            req.client_uri,
            req.logo_uri,
            req.redirect_uris,
            req.grant_types,
            req.response_types,
            req.token_endpoint_auth_method,
            default_scope,
        );

        Ok(response)
    }

    /// Emit an audit event for DCR cap-exceeded.
    fn emit_rate_limited_audit(&self) {
        let entry = match AuditEntry::<Event>::builder(
            AuditActionType::OAUTH_CLIENT_REGISTRATION_RATE_LIMITED,
        )
        .actor_system()
        .outcome(AuditOutcome::Denied)
        .details(serde_json::json!({
            "reason": "dcr_lifetime_cap_exceeded",
        }))
        .build()
        {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, "dropping invalid rate-limited audit entry");
                return;
            }
        };
        self.audit_emitter.emit_event(entry);
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
    use std::net::IpAddr;
    use uptrakit_audit_log::{AuditEmitter, AuditLogDispatcher, NoopBackend};
    use uptrakit_shared_db::entity::{oauth_client, oauth_consent, oauth_refresh_token, user};
    use uptrakit_shared_types::MaskedEmail;

    const FAKE_SECRET: &[u8] = b"test-secret-32-bytes-minimum-aaa";
    const FAKE_IP: IpAddr = IpAddr::V4(std::net::Ipv4Addr::new(127, 0, 0, 1));

    fn make_emitter() -> Arc<AuditEmitter> {
        let dispatcher = AuditLogDispatcher::new(Arc::new(NoopBackend));
        Arc::new(AuditEmitter::new(dispatcher))
    }

    fn make_clock(
        cell: Arc<Mutex<OffsetDateTime>>,
    ) -> Arc<dyn Fn() -> OffsetDateTime + Send + Sync> {
        Arc::new(move || *cell.lock())
    }

    fn make_service(
        db: sea_orm::DatabaseConnection,
        clock_cell: Arc<Mutex<OffsetDateTime>>,
    ) -> OAuthClientService {
        OAuthClientService::new(db, make_clock(clock_cell), make_emitter())
    }

    /// Insert a minimal user row and return the generated user_id.
    async fn insert_user(db: &sea_orm::DatabaseConnection) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let user_id = Uuid::now_v7();
        user::ActiveModel {
            id: Set(user_id),
            email: Set(MaskedEmail::new(format!("test-{user_id}@example.com"))),
            first_name: Set("Test".into()),
            last_name: Set("User".into()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user");
        user_id
    }

    fn minimal_req() -> DcrRegistrationRequest {
        DcrRegistrationRequest::new(
            "Test Client",
            None,
            None,
            vec!["https://example.com/callback".into()],
            vec!["authorization_code".into()],
            vec!["code".into()],
            "none",
            Some("openid mcp:read".into()),
        )
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 1 — register_dcr returns client_id and registration_access_token
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_dcr_returns_client_id_and_registration_token() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db, clock_cell);

        let resp = svc
            .register_dcr(minimal_req(), FAKE_IP, FAKE_SECRET)
            .await
            .expect("register_dcr should succeed");

        assert!(!resp.client_id.is_empty());
        assert!(
            resp.registration_access_token
                .as_deref()
                .is_some_and(|t| !t.is_empty()),
            "registration_access_token must be present and non-empty"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 2 — stored token hash differs from the raw token
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_dcr_stored_token_hash_differs_from_raw() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db.clone(), clock_cell);

        let resp = svc
            .register_dcr(minimal_req(), FAKE_IP, FAKE_SECRET)
            .await
            .expect("register_dcr should succeed");

        let row = oauth_client::Entity::find_by_id(resp.client_id.clone())
            .one(&db)
            .await
            .expect("db query should succeed")
            .expect("row should exist");

        let stored_hash = row
            .registration_access_token_hash
            .expect("hash should be stored");
        let raw_token = resp
            .registration_access_token
            .expect("registration_access_token must be present");
        assert_ne!(
            stored_hash, raw_token,
            "stored hash must not equal the raw token"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 3 — register_dcr sets created_via = "dcr"
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_dcr_sets_created_via_dcr() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db.clone(), clock_cell);

        let resp = svc
            .register_dcr(minimal_req(), FAKE_IP, FAKE_SECRET)
            .await
            .expect("register_dcr should succeed");

        let row = oauth_client::Entity::find_by_id(resp.client_id)
            .one(&db)
            .await
            .expect("db query should succeed")
            .expect("row should exist");

        assert_eq!(row.created_via, "dcr");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 4 — register_manual sets created_via = "manual"
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_manual_sets_created_via_manual() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db.clone(), clock_cell);

        let resp = svc
            .register_manual(minimal_req())
            .await
            .expect("register_manual should succeed");

        let row = oauth_client::Entity::find_by_id(resp.client_id)
            .one(&db)
            .await
            .expect("db query should succeed")
            .expect("row should exist");

        assert_eq!(row.created_via, "manual");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 5 — lookup returns None for an unknown client_id
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn lookup_returns_none_for_unknown_client() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db, clock_cell);

        let result = svc
            .lookup("does-not-exist")
            .await
            .expect("lookup should not error");

        assert!(result.is_none());
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 6 — lookup returns model for a known client_id
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn lookup_returns_model_for_known_client() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db, clock_cell);

        let resp = svc
            .register_manual(minimal_req())
            .await
            .expect("register_manual should succeed");

        let row = svc
            .lookup(&resp.client_id)
            .await
            .expect("lookup should succeed")
            .expect("row should be found");

        assert_eq!(row.id, resp.client_id);
        assert_eq!(row.client_name, "Test Client");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 7 — revoke cascades to consents and refresh tokens
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn revoke_cascades_to_consents_and_refresh_tokens() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db.clone(), clock_cell);
        let now = OffsetDateTime::now_utc();

        // Register a client.
        let resp = svc
            .register_manual(minimal_req())
            .await
            .expect("register_manual should succeed");
        let client_id = &resp.client_id;

        // Insert a real user so the FK constraint on oauth_consents is satisfied.
        let user_id = insert_user(&db).await;

        // Insert a consent row for this client.
        let consent_id = Uuid::now_v7();
        oauth_consent::ActiveModel {
            id: Set(consent_id),
            user_id: Set(user_id),
            client_id: Set(client_id.clone()),
            scopes: Set("openid mcp:read".into()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert oauth_consent");

        // Insert a refresh token row for this client.
        let token_id = Uuid::now_v7();
        let family_id = Uuid::now_v7();
        oauth_refresh_token::ActiveModel {
            id: Set(token_id),
            family_id: Set(family_id),
            parent_id: Set(None),
            token_hash: Set(hash_token("raw-token-value")),
            client_id: Set(client_id.clone()),
            user_id: Set(user_id),
            consent_id: Set(consent_id),
            scope: Set("openid mcp:read".into()),
            resource: Set("https://mcp.example.com".into()),
            issued_at: Set(now),
            expires_at: Set(now + time::Duration::days(30)),
            family_expires_at: Set(now + time::Duration::days(90)),
            rotated_at: Set(None),
            revoked_at: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert oauth_refresh_token");

        // Now revoke the client.
        svc.revoke(client_id).await.expect("revoke should succeed");

        // Verify client is revoked.
        let client_row = oauth_client::Entity::find_by_id(client_id.clone())
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            client_row.revoked_at.is_some(),
            "client revoked_at should be set"
        );

        // Verify consent is revoked.
        let consent_row = oauth_consent::Entity::find_by_id(consent_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            consent_row.revoked_at.is_some(),
            "consent revoked_at should be set"
        );

        // Verify refresh token is revoked.
        let token_row = oauth_refresh_token::Entity::find_by_id(token_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            token_row.revoked_at.is_some(),
            "refresh token revoked_at should be set"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 8 — promote_trusted sets trusted_at
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn promote_trusted_sets_trusted_at() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db.clone(), Arc::clone(&clock_cell));

        let resp = svc
            .register_manual(minimal_req())
            .await
            .expect("register_manual should succeed");

        // Confirm not yet trusted.
        let before = oauth_client::Entity::find_by_id(resp.client_id.clone())
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(
            before.trusted_at.is_none(),
            "should not be trusted initially"
        );

        // Promote.
        svc.promote_trusted(&resp.client_id)
            .await
            .expect("promote_trusted should succeed");

        // Verify trusted_at is now set.
        let after = oauth_client::Entity::find_by_id(resp.client_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert!(after.trusted_at.is_some(), "trusted_at should be set");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 9 — DCR cap blocks 21st registration and emits audit
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_dcr_cap_blocks_21st_registration_and_emits_audit() {
        let db = setup_migrated_db().await;
        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = make_service(db, clock_cell);

        // Register exactly 20 DCR clients.
        for _ in 0..20 {
            svc.register_dcr(minimal_req(), FAKE_IP, FAKE_SECRET)
                .await
                .expect("registration should succeed while under cap");
        }

        // The 21st registration must be rejected.
        let err = svc
            .register_dcr(minimal_req(), FAKE_IP, FAKE_SECRET)
            .await
            .expect_err("21st registration should be rejected");

        assert!(
            matches!(
                err.current_context(),
                OAuthClientError::RegistrationCapExceeded
            ),
            "expected RegistrationCapExceeded, got {:?}",
            err.current_context()
        );
    }
}
