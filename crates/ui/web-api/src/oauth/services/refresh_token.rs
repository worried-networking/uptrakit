//! Service for minting, rotating, and revoking `oauth_refresh_tokens` rows.
//!
//! Per spec §10.3: opaque refresh tokens with sliding TTL (default 30 days)
//! plus an absolute family TTL (default 90 days). On every rotation a new
//! token is minted in the same `family_id` lineage with `parent_id` pointing
//! at the previously-used row; the old row is marked `rotated_at`. Reusing
//! an already-rotated or revoked token (replay) cascade-revokes the entire
//! family.

use axum::response::Response;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait, JoinType,
    QueryFilter, QuerySelect, RelationTrait, Set, SqliteTransactionMode, TransactionOptions,
    TransactionTrait,
};
use std::sync::Arc;
use thiserror::Error;
use time::{Duration, OffsetDateTime};
use uptrakit_audit_log::{
    AuditActionType, AuditEmitter, AuditEntry, AuditOutcome, Event, RegisteredAuditAction,
};
use uptrakit_shared_db::entity::{oauth_consent, oauth_refresh_token, user_role};
use uptrakit_shared_macros::impl_report_conversion;
use uptrakit_web_api_auth::auth::token::hash_token;
use uptrakit_web_api_types::oauth::{McpAccessTokenClaims, OpaqueRefreshToken};
use uuid::Uuid;

use crate::oauth::http_responses::{oauth_400, oauth_500};

/// Build the RFC 6749 response for a refresh-token rotation failure.
///
/// Sanctioned RFC 6749 exit for the `/oauth/token` endpoint's refresh_token
/// grant — keeps the `match e.current_context()` pattern out of
/// `crates/ui/web-api/src/routes/` per the `check_legacy_error_matches.sh` gate.
/// See `docs/development/error-handling.md` Pattern 18.
pub(crate) fn refresh_error_to_response(e: &Report<OAuthRefreshError>) -> Response {
    match e.current_context() {
        OAuthRefreshError::InvalidGrant(reason) => oauth_400("invalid_grant", reason),
        OAuthRefreshError::InvalidTarget => oauth_400("invalid_target", "resource mismatch"),
        OAuthRefreshError::InvalidScope => {
            oauth_400("invalid_scope", "requested scope exceeds granted scope")
        }
        OAuthRefreshError::ConsentRevoked => oauth_400("invalid_grant", "consent has been revoked"),
        OAuthRefreshError::Jwt(_) => {
            tracing::error!(error = %e, "JWT error during refresh_token rotation");
            oauth_500()
        }
        OAuthRefreshError::Database(_) => {
            tracing::error!(error = %e, "DB error during refresh_token rotation");
            oauth_500()
        }
        #[expect(
            unreachable_patterns,
            reason = "OAuthRefreshError is #[non_exhaustive]; wildcard required for forward-compatibility"
        )]
        _ => {
            tracing::error!(error = %e, "unexpected error during refresh_token rotation");
            oauth_500()
        }
    }
}

const DEFAULT_ACCESS_TOKEN_TTL_SECS: i64 = 900;
const DEFAULT_REFRESH_TOKEN_TTL_SECS: i64 = 2_592_000; // 30 days
const DEFAULT_REFRESH_FAMILY_MAX_TTL_SECS: i64 = 7_776_000; // 90 days

/// Errors produced by [`OAuthRefreshTokenService`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OAuthRefreshError {
    /// Refresh-token validation failed (RFC 6749 `invalid_grant`).
    #[error("invalid_grant: {0}")]
    InvalidGrant(&'static str),
    /// Requested resource did not match the bound resource on the refresh row.
    #[error("invalid_target")]
    InvalidTarget,
    /// Requested scope is not a subset of the bound scope.
    #[error("invalid_scope")]
    InvalidScope,
    /// The consent row backing this refresh token has been revoked.
    #[error("consent_revoked")]
    ConsentRevoked,
    /// JWT signing failed when minting the new access token.
    #[error("jwt error: {0}")]
    Jwt(crate::oauth::jwt::JwtError),
    /// Underlying database error.
    #[error("database error")]
    Database(sea_orm::DbErr),
}

pub(crate) type Result<T> = std::result::Result<T, Report<OAuthRefreshError>>;

impl_report_conversion! {
    sea_orm::DbErr => OAuthRefreshError::Database,
}

impl_report_conversion! {
    crate::oauth::jwt::JwtError => OAuthRefreshError::Jwt,
}

/// Outcome of a successful initial mint.
#[derive(Debug)]
pub struct MintOutcome {
    pub refresh_token: OpaqueRefreshToken,
    pub expires_in: i64,
    pub family_expires_at: OffsetDateTime,
}

/// Outcome of a successful rotation.
#[derive(Debug)]
pub struct RotationOutcome {
    pub access_token: String,
    pub refresh_token: OpaqueRefreshToken,
    pub expires_in: i64,
    pub refresh_expires_in: i64,
    pub scope: String,
}

/// Mints, rotates, and revokes `oauth_refresh_tokens` rows.
pub struct OAuthRefreshTokenService {
    db: DatabaseConnection,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    signer: Arc<crate::oauth::jwt::McpOAuthJwtSigner>,
    audit_emitter: Arc<AuditEmitter>,
    issuer: String,
    #[expect(
        dead_code,
        reason = "audience is recorded for use by future call sites — access token aud is the resource passed to rotate"
    )]
    audience: String,
    access_token_ttl_secs: i64,
    refresh_token_ttl_secs: i64,
    refresh_family_max_ttl_secs: i64,
}

impl OAuthRefreshTokenService {
    /// Construct a new refresh-token service.
    #[expect(
        clippy::too_many_arguments,
        reason = "service struct carries all dependencies; alternative builder would not reduce footprint"
    )]
    pub fn new(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
        signer: Arc<crate::oauth::jwt::McpOAuthJwtSigner>,
        audit_emitter: Arc<AuditEmitter>,
        issuer: String,
        audience: String,
        access_token_ttl_secs: i64,
        refresh_token_ttl_secs: i64,
        refresh_family_max_ttl_secs: i64,
    ) -> Self {
        Self {
            db,
            clock,
            signer,
            audit_emitter,
            issuer,
            audience,
            access_token_ttl_secs,
            refresh_token_ttl_secs,
            refresh_family_max_ttl_secs,
        }
    }

    /// Construct a new refresh-token service using default TTLs:
    /// access 900 s, refresh 30 d, family 90 d.
    pub fn with_defaults(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
        signer: Arc<crate::oauth::jwt::McpOAuthJwtSigner>,
        audit_emitter: Arc<AuditEmitter>,
        issuer: String,
        audience: String,
    ) -> Self {
        Self::new(
            db,
            clock,
            signer,
            audit_emitter,
            issuer,
            audience,
            DEFAULT_ACCESS_TOKEN_TTL_SECS,
            DEFAULT_REFRESH_TOKEN_TTL_SECS,
            DEFAULT_REFRESH_FAMILY_MAX_TTL_SECS,
        )
    }

    /// Mint an initial refresh token — called after authorization-code exchange.
    pub async fn mint(
        &self,
        client_id: &str,
        user_id: Uuid,
        consent_id: Uuid,
        scope: &str,
        resource: &str,
    ) -> Result<MintOutcome> {
        let now = (self.clock)();
        let expires_at = now + Duration::seconds(self.refresh_token_ttl_secs);
        let family_expires_at = now + Duration::seconds(self.refresh_family_max_ttl_secs);
        let id = Uuid::now_v7();
        let family_id = Uuid::now_v7();

        let raw = generate_refresh_token();
        // SAFETY: raw is constructed as `format!("upr_{…}")` — prefix is guaranteed.
        #[expect(
            clippy::expect_used,
            reason = "raw is constructed with a literal upr_ prefix; parse cannot fail"
        )]
        let refresh_token =
            OpaqueRefreshToken::parse(&raw).expect("generated token always starts with upr_");

        let token_hash = hash_token(&raw);

        let model = oauth_refresh_token::ActiveModel {
            id: Set(id),
            family_id: Set(family_id),
            parent_id: Set(None),
            token_hash: Set(token_hash),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            consent_id: Set(consent_id),
            scope: Set(scope.to_string()),
            resource: Set(resource.to_string()),
            issued_at: Set(now),
            expires_at: Set(expires_at),
            family_expires_at: Set(family_expires_at),
            rotated_at: Set(None),
            revoked_at: Set(None),
        };

        model.insert(&self.db).await.context_to()?;

        Ok(MintOutcome {
            refresh_token,
            expires_in: self.refresh_token_ttl_secs,
            family_expires_at,
        })
    }

    /// Rotate a refresh token: verify, mint a new pair, mark old row rotated.
    ///
    /// On replay (reuse of already-rotated or revoked token) cascade-revokes
    /// the entire `family_id` and emits an `OAUTH_REFRESH_REPLAY_DETECTED`
    /// audit event.
    pub async fn rotate(
        &self,
        refresh_token: &str,
        client_id: &str,
        requested_scope: Option<&str>,
        resource: &str,
    ) -> Result<RotationOutcome> {
        let now = (self.clock)();
        let token_hash = hash_token(refresh_token);

        let txn = self
            .db
            .begin_with_options(TransactionOptions {
                sqlite_transaction_mode: Some(SqliteTransactionMode::Immediate),
                ..Default::default()
            })
            .await
            .context_to()?;

        let row = oauth_refresh_token::Entity::find()
            .filter(oauth_refresh_token::Column::TokenHash.eq(&token_hash))
            .one(&txn)
            .await
            .context_to()?;

        let row = match row {
            Some(r) => r,
            None => bail!(OAuthRefreshError::InvalidGrant("token_not_found")),
        };

        // Replay detection (revoked or rotated row reuse). On either signal we
        // cascade-revoke the whole family and emit the dedicated audit event.
        if row.revoked_at.is_some() || row.rotated_at.is_some() {
            revoke_family(&txn, row.family_id, now).await.context_to()?;
            txn.commit().await.context_to()?;
            self.emit_audit(
                AuditActionType::OAUTH_REFRESH_REPLAY_DETECTED,
                row.user_id,
                row.client_id.clone(),
                row.family_id,
            );
            bail!(OAuthRefreshError::InvalidGrant("replay_detected"));
        }

        if row.expires_at <= now {
            bail!(OAuthRefreshError::InvalidGrant("refresh_token_expired"));
        }
        if row.family_expires_at <= now {
            bail!(OAuthRefreshError::InvalidGrant("family_expired"));
        }
        if row.client_id != client_id {
            bail!(OAuthRefreshError::InvalidGrant("client_mismatch"));
        }
        if row.resource != resource {
            bail!(OAuthRefreshError::InvalidTarget);
        }

        // Scope check: requested must be a subset of bound.
        let effective_scope = match requested_scope {
            Some(req) if !req.is_empty() => {
                let bound: std::collections::HashSet<&str> = row.scope.split_whitespace().collect();
                for s in req.split_whitespace() {
                    if !bound.contains(s) {
                        bail!(OAuthRefreshError::InvalidScope);
                    }
                }
                req.to_string()
            }
            _ => row.scope.clone(),
        };

        // Consent must still be active.
        let consent = oauth_consent::Entity::find_by_id(row.consent_id)
            .filter(oauth_consent::Column::RevokedAt.is_null())
            .one(&txn)
            .await
            .context_to()?;
        if consent.is_none() {
            bail!(OAuthRefreshError::ConsentRevoked);
        }

        // Mint the new refresh row in the same family. Cap `expires_at` by
        // `family_expires_at` so a rotation late in the window doesn't extend
        // the absolute lifetime.
        let new_id = Uuid::now_v7();
        let raw_new = generate_refresh_token();
        // SAFETY: raw_new is constructed with the literal `upr_` prefix.
        #[expect(
            clippy::expect_used,
            reason = "raw_new is constructed with a literal upr_ prefix; parse cannot fail"
        )]
        let new_refresh_token =
            OpaqueRefreshToken::parse(&raw_new).expect("generated token always starts with upr_");
        let new_token_hash = hash_token(&raw_new);
        let proposed_expires = now + Duration::seconds(self.refresh_token_ttl_secs);
        let new_expires_at = proposed_expires.min(row.family_expires_at);
        let new_refresh_expires_in = (new_expires_at - now).whole_seconds();

        oauth_refresh_token::ActiveModel {
            id: Set(new_id),
            family_id: Set(row.family_id),
            parent_id: Set(Some(row.id)),
            token_hash: Set(new_token_hash),
            client_id: Set(row.client_id.clone()),
            user_id: Set(row.user_id),
            consent_id: Set(row.consent_id),
            scope: Set(effective_scope.clone()),
            resource: Set(row.resource.clone()),
            issued_at: Set(now),
            expires_at: Set(new_expires_at),
            family_expires_at: Set(row.family_expires_at),
            rotated_at: Set(None),
            revoked_at: Set(None),
        }
        .insert(&txn)
        .await
        .context_to()?;

        // Mark old row rotated.
        let mut active: oauth_refresh_token::ActiveModel = row.clone().into();
        active.rotated_at = Set(Some(now));
        active.update(&txn).await.context_to()?;

        // Resolve tenant_id before we commit so a missing user_role row aborts
        // the rotation without leaving a half-issued token in the DB.
        let tenant_id = lookup_user_tenant(&txn, row.user_id).await.context_to()?;

        txn.commit().await.context_to()?;

        // Mint the access token outside the transaction. JWT signing only fails
        // on serialization issues which would be a programmer bug.
        let access_token = self.mint_access_token(
            row.user_id,
            tenant_id,
            &row.client_id,
            &effective_scope,
            resource,
            now,
        )?;

        // Audit the successful rotation.
        self.emit_audit(
            AuditActionType::OAUTH_REFRESH_ROTATED,
            row.user_id,
            row.client_id.clone(),
            row.family_id,
        );

        Ok(RotationOutcome {
            access_token,
            refresh_token: new_refresh_token,
            expires_in: self.access_token_ttl_secs,
            refresh_expires_in: new_refresh_expires_in,
            scope: effective_scope,
        })
    }

    fn mint_access_token(
        &self,
        user_id: Uuid,
        tenant_id: Uuid,
        client_id: &str,
        scope: &str,
        resource: &str,
        now: OffsetDateTime,
    ) -> Result<String> {
        let iat = now.unix_timestamp();
        let exp = iat + self.access_token_ttl_secs;
        let claims = McpAccessTokenClaims::new(
            self.issuer.clone(),
            user_id.to_string(),
            resource.to_string(),
            client_id.to_string(),
            scope.to_string(),
            Uuid::now_v7().to_string(),
            iat,
            iat,
            exp,
            tenant_id.to_string(),
        );
        let token = self.signer.mint(&claims).context_to()?;
        Ok(token)
    }

    fn emit_audit(
        &self,
        action: RegisteredAuditAction,
        user_id: Uuid,
        client_id: String,
        family_id: Uuid,
    ) {
        let entry = match AuditEntry::<Event>::builder_event(action)
            .actor(uptrakit_audit_log::AuditActorType::User, Some(user_id))
            .outcome(AuditOutcome::Success)
            .target(
                "oauth_refresh_family",
                family_id.to_string(),
                Some(client_id.clone()),
            )
            .details(serde_json::json!({
                "client_id": client_id,
                "family_id": family_id.to_string(),
            }))
            .build()
        {
            Ok(e) => e,
            Err(err) => {
                tracing::warn!(error = %err, "dropping invalid refresh-token audit entry");
                return;
            }
        };
        self.audit_emitter.emit_event(entry);
    }
}

/// Generate a fresh `upr_`-prefixed refresh token (32 random bytes, base64url-no-pad).
fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill(&mut bytes);
    format!("upr_{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// Cascade-revoke every active row in a family.
async fn revoke_family<C>(
    conn: &C,
    family_id: Uuid,
    now: OffsetDateTime,
) -> std::result::Result<(), sea_orm::DbErr>
where
    C: ConnectionTrait,
{
    oauth_refresh_token::Entity::update_many()
        .col_expr(
            oauth_refresh_token::Column::RevokedAt,
            sea_orm::sea_query::Expr::value(Some(now)),
        )
        .filter(oauth_refresh_token::Column::FamilyId.eq(family_id))
        .filter(oauth_refresh_token::Column::RevokedAt.is_null())
        .exec(conn)
        .await?;
    Ok(())
}

/// Look up the tenant a user belongs to via the `user_role` table.
///
/// Refresh-token rotation requires `tenant_id` for the access-token claim;
/// a user with no role assignment cannot have tokens minted on their behalf.
async fn lookup_user_tenant<C>(conn: &C, user_id: Uuid) -> std::result::Result<Uuid, sea_orm::DbErr>
where
    C: ConnectionTrait,
{
    let row = user_role::Entity::find()
        .filter(user_role::Column::UserId.eq(user_id))
        .select_only()
        .column(user_role::Column::TenantId)
        .join(JoinType::InnerJoin, user_role::Relation::Tenant.def())
        .into_tuple::<Uuid>()
        .one(conn)
        .await?;
    row.ok_or_else(|| sea_orm::DbErr::Custom(format!("no tenant for user {user_id}")))
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
    use uptrakit_audit_log::{AuditEmitter, AuditLogDispatcher, NoopBackend};
    use uptrakit_shared_db::entity::{oauth_client, oauth_consent, role, tenant, user, user_role};
    use uptrakit_shared_types::MaskedEmail;

    const TEST_SECRET: &[u8] = b"test-secret-32-bytes-minimum-aaa";
    const TEST_ISSUER: &str = "https://issuer.example.com";
    const TEST_RESOURCE: &str = "https://mcp.example.com";

    fn make_clock(
        cell: Arc<Mutex<OffsetDateTime>>,
    ) -> Arc<dyn Fn() -> OffsetDateTime + Send + Sync> {
        Arc::new(move || *cell.lock())
    }

    fn make_emitter() -> Arc<AuditEmitter> {
        let dispatcher = AuditLogDispatcher::new(Arc::new(NoopBackend));
        Arc::new(AuditEmitter::new(dispatcher))
    }

    async fn insert_tenant(db: &DatabaseConnection) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        tenant::ActiveModel {
            id: Set(id),
            name: Set("test-tenant".into()),
            slug: Set(id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
        id
    }

    async fn insert_role(db: &DatabaseConnection) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        role::ActiveModel {
            id: Set(id),
            name: Set(format!("test-role-{id}")),
            description: Set(None),
            is_built_in: Set(false),
            created_at: Set(now),
            tenant_id: Set(None),
        }
        .insert(db)
        .await
        .expect("insert role");
        id
    }

    async fn insert_user_with_tenant(db: &DatabaseConnection, tenant_id: Uuid) -> Uuid {
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

        let role_id = insert_role(db).await;
        user_role::ActiveModel {
            tenant_id: Set(tenant_id),
            user_id: Set(user_id),
            role_id: Set(role_id),
            assigned_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert user_role");

        user_id
    }

    async fn insert_oauth_client(db: &DatabaseConnection) -> String {
        let now = OffsetDateTime::now_utc();
        let client_id = format!("test-client-{}", Uuid::now_v7());
        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Test Client".into()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set("https://example.com/callback".into()),
            default_scope: Set("openid mcp:read".into()),
            grant_types: Set("authorization_code refresh_token".into()),
            response_types: Set("code".into()),
            token_endpoint_auth_method: Set("none".into()),
            client_secret_hash: Set(None),
            registration_access_token_hash: Set(None),
            created_via: Set("test".into()),
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

    async fn insert_consent(
        db: &DatabaseConnection,
        user_id: Uuid,
        client_id: &str,
        scopes: &str,
    ) -> Uuid {
        let id = Uuid::now_v7();
        let now = OffsetDateTime::now_utc();
        oauth_consent::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            client_id: Set(client_id.to_string()),
            scopes: Set(scopes.into()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(None),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_consent");
        id
    }

    /// Test bundle: every prerequisite row a refresh test needs.
    struct Fixture {
        db: DatabaseConnection,
        clock_cell: Arc<Mutex<OffsetDateTime>>,
        svc: OAuthRefreshTokenService,
        client_id: String,
        user_id: Uuid,
        consent_id: Uuid,
    }

    async fn setup_fixture() -> Fixture {
        let db = setup_migrated_db().await;
        let tenant_id = insert_tenant(&db).await;
        let user_id = insert_user_with_tenant(&db, tenant_id).await;
        let client_id = insert_oauth_client(&db).await;
        let consent_id = insert_consent(&db, user_id, &client_id, "openid mcp:read").await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let signer = Arc::new(crate::oauth::jwt::McpOAuthJwtSigner::new(TEST_SECRET));
        let emitter = make_emitter();

        let svc = OAuthRefreshTokenService::with_defaults(
            db.clone(),
            make_clock(Arc::clone(&clock_cell)),
            signer,
            emitter,
            TEST_ISSUER.into(),
            TEST_RESOURCE.into(),
        );

        Fixture {
            db,
            clock_cell,
            svc,
            client_id,
            user_id,
            consent_id,
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 1 — mint produces a upr_-prefixed token
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn mint_produces_upr_prefixed_token() {
        let fx = setup_fixture().await;
        let out = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");
        assert!(out.refresh_token.as_str().starts_with("upr_"));
        assert_eq!(out.expires_in, DEFAULT_REFRESH_TOKEN_TTL_SECS);
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 2 — rotate happy path returns new pair
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_happy_path_returns_new_token_pair() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        let rotated = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                None,
                TEST_RESOURCE,
            )
            .await
            .expect("rotate should succeed");

        assert!(rotated.refresh_token.as_str().starts_with("upr_"));
        assert_ne!(rotated.refresh_token.as_str(), mint.refresh_token.as_str());
        assert!(!rotated.access_token.is_empty());
        assert_eq!(rotated.expires_in, DEFAULT_ACCESS_TOKEN_TTL_SECS);
        assert_eq!(rotated.scope, "openid mcp:read");

        // Old row must be marked rotated_at; new row must exist with same family.
        let rows = oauth_refresh_token::Entity::find()
            .all(&fx.db)
            .await
            .expect("query rows");
        assert_eq!(rows.len(), 2);
        let (rotated_row, fresh_row): (&oauth_refresh_token::Model, &oauth_refresh_token::Model) =
            if rows[0].rotated_at.is_some() {
                (&rows[0], &rows[1])
            } else {
                (&rows[1], &rows[0])
            };
        assert!(rotated_row.rotated_at.is_some());
        assert!(fresh_row.rotated_at.is_none());
        assert_eq!(rotated_row.family_id, fresh_row.family_id);
        assert_eq!(fresh_row.parent_id, Some(rotated_row.id));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 3 — replay detection revokes the entire family
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_replay_detection_revokes_family() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        // First rotation succeeds.
        let _rotated = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                None,
                TEST_RESOURCE,
            )
            .await
            .expect("first rotation should succeed");

        // Replay the original (already-rotated) token.
        let err = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                None,
                TEST_RESOURCE,
            )
            .await
            .expect_err("replay must fail");

        assert!(matches!(
            err.current_context(),
            OAuthRefreshError::InvalidGrant("replay_detected")
        ));

        // Every row in the family must now be revoked.
        let rows = oauth_refresh_token::Entity::find()
            .all(&fx.db)
            .await
            .expect("query rows");
        assert!(!rows.is_empty());
        for r in &rows {
            assert!(r.revoked_at.is_some(), "row {} should be revoked", r.id);
        }
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 4 — sliding TTL expiry rejects rotation
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_expired_sliding_ttl_returns_error() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        // Push clock past 30 day sliding window.
        *fx.clock_cell.lock() += Duration::seconds(DEFAULT_REFRESH_TOKEN_TTL_SECS + 1);

        let err = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                None,
                TEST_RESOURCE,
            )
            .await
            .expect_err("must fail after sliding TTL expiry");

        assert!(matches!(
            err.current_context(),
            OAuthRefreshError::InvalidGrant("refresh_token_expired")
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 5 — family-max TTL expiry rejects rotation
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_expired_family_ttl_returns_error() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_tenant(&db).await;
        let user_id = insert_user_with_tenant(&db, tenant_id).await;
        let client_id = insert_oauth_client(&db).await;
        let consent_id = insert_consent(&db, user_id, &client_id, "openid mcp:read").await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let signer = Arc::new(crate::oauth::jwt::McpOAuthJwtSigner::new(TEST_SECRET));
        let emitter = make_emitter();
        // sliding 30 d, family 35 d — so we can step the clock past family
        // without immediately tripping the sliding check first.
        let svc = OAuthRefreshTokenService::new(
            db.clone(),
            make_clock(Arc::clone(&clock_cell)),
            signer,
            emitter,
            TEST_ISSUER.into(),
            TEST_RESOURCE.into(),
            DEFAULT_ACCESS_TOKEN_TTL_SECS,
            DEFAULT_REFRESH_TOKEN_TTL_SECS,
            DEFAULT_REFRESH_TOKEN_TTL_SECS + 5 * 86_400,
        );

        let mint = svc
            .mint(
                &client_id,
                user_id,
                consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        // Rotate at 29 days — sliding intact, family intact, fresh row will
        // get capped at family_expires_at. Then push past family.
        *clock_cell.lock() += Duration::days(29);
        let rotated = svc
            .rotate(mint.refresh_token.as_str(), &client_id, None, TEST_RESOURCE)
            .await
            .expect("rotation at 29d should succeed");

        // Now step past family expiry (35 days from t0 → 6 more days).
        *clock_cell.lock() += Duration::days(7);
        let err = svc
            .rotate(
                rotated.refresh_token.as_str(),
                &client_id,
                None,
                TEST_RESOURCE,
            )
            .await
            .expect_err("must fail after family TTL expiry");
        assert!(
            matches!(
                err.current_context(),
                OAuthRefreshError::InvalidGrant("refresh_token_expired")
                    | OAuthRefreshError::InvalidGrant("family_expired")
            ),
            "expected sliding-or-family expiry, got {:?}",
            err.current_context()
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 6 — client_id mismatch
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_client_id_mismatch_returns_error() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        let err = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                "other-client",
                None,
                TEST_RESOURCE,
            )
            .await
            .expect_err("must fail on client mismatch");

        assert!(matches!(
            err.current_context(),
            OAuthRefreshError::InvalidGrant("client_mismatch")
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 7 — resource mismatch
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_resource_mismatch_returns_error() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        let err = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                None,
                "https://other-resource.example.com",
            )
            .await
            .expect_err("must fail on resource mismatch");

        assert!(matches!(
            err.current_context(),
            OAuthRefreshError::InvalidTarget
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 8 — scope superset → invalid_scope
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_scope_superset_returns_invalid_scope() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        let err = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                Some("openid mcp:read mcp:admin"),
                TEST_RESOURCE,
            )
            .await
            .expect_err("must fail on scope superset");

        assert!(matches!(
            err.current_context(),
            OAuthRefreshError::InvalidScope
        ));
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 9 — revoked consent → ConsentRevoked
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn rotate_revoked_consent_returns_error() {
        let fx = setup_fixture().await;
        let mint = fx
            .svc
            .mint(
                &fx.client_id,
                fx.user_id,
                fx.consent_id,
                "openid mcp:read",
                TEST_RESOURCE,
            )
            .await
            .expect("mint should succeed");

        // Revoke the consent row.
        let consent = oauth_consent::Entity::find_by_id(fx.consent_id)
            .one(&fx.db)
            .await
            .expect("query consent")
            .expect("consent must exist");
        let mut active: oauth_consent::ActiveModel = consent.into();
        active.revoked_at = Set(Some(OffsetDateTime::now_utc()));
        active.update(&fx.db).await.expect("revoke consent");

        let err = fx
            .svc
            .rotate(
                mint.refresh_token.as_str(),
                &fx.client_id,
                None,
                TEST_RESOURCE,
            )
            .await
            .expect_err("must fail on revoked consent");

        assert!(matches!(
            err.current_context(),
            OAuthRefreshError::ConsentRevoked
        ));
    }
}
