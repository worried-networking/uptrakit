//! Service for managing `oauth_consents` table rows.
//!
//! Per spec §12.3 (skip-prompt logic) and §10.5 (revoke + cascade).

use rootcause::prelude::*;
use sea_orm::{ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set};
use std::sync::Arc;
use thiserror::Error;
use time::OffsetDateTime;
use uptrakit_shared_db::begin_immediate;
use uptrakit_shared_db::entity::{oauth_client, oauth_consent, oauth_refresh_token};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

/// Errors produced by [`OAuthConsentService`].
#[non_exhaustive]
#[derive(Debug, Error)]
pub enum OAuthConsentError {
    #[error("consent not found or already revoked")]
    NotFound,
    #[error("database error")]
    Database(sea_orm::DbErr),
}

pub(crate) type Result<T> = std::result::Result<T, Report<OAuthConsentError>>;

impl_report_conversion! {
    sea_orm::DbErr => OAuthConsentError::Database,
}

/// Service that manages `oauth_consents` table rows.
pub struct OAuthConsentService {
    db: DatabaseConnection,
    clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
}

impl OAuthConsentService {
    pub fn new(
        db: DatabaseConnection,
        clock: Arc<dyn Fn() -> OffsetDateTime + Send + Sync>,
    ) -> Self {
        Self { db, clock }
    }

    /// Returns `true` when the consent screen may be skipped for this
    /// `(user_id, client_id, requested_scope)` triple.
    ///
    /// Per spec §12.3, all five conditions must hold:
    /// 1. A consent row exists for `(user_id, client_id)`.
    /// 2. `revoked_at IS NULL` — consent is active.
    /// 3. `revalidation_required_at IS NULL` — no forced re-prompt.
    /// 4. The granted scope set is a superset of `requested_scope`.
    /// 5. The client is not unverified (`oauth_clients.trusted_at IS NOT NULL`).
    pub async fn should_skip_prompt(
        &self,
        user_id: Uuid,
        client_id: &str,
        requested_scope: &str,
    ) -> Result<bool> {
        // Condition 5: unverified clients always require re-prompt.
        let trusted = oauth_client::Entity::find_by_id(client_id)
            .one(&self.db)
            .await
            .context_to()?
            .map(|c| c.trusted_at.is_some())
            .unwrap_or(false);

        if !trusted {
            return Ok(false);
        }

        let row = oauth_consent::Entity::find()
            .filter(oauth_consent::Column::UserId.eq(user_id))
            .filter(oauth_consent::Column::ClientId.eq(client_id))
            .filter(oauth_consent::Column::RevokedAt.is_null())
            .one(&self.db)
            .await
            .context_to()?;

        let row = match row {
            Some(r) => r,
            None => return Ok(false),
        };

        // Condition 3: revalidation must not be pending.
        if row.revalidation_required_at.is_some() {
            return Ok(false);
        }

        // Condition 4: granted scopes must be a superset of requested scopes.
        let granted: std::collections::HashSet<&str> = row.scopes.split_whitespace().collect();
        let all_covered = requested_scope
            .split_whitespace()
            .all(|s| granted.contains(s));

        Ok(all_covered)
    }

    /// Insert or update a consent record for `(user_id, client_id)`.
    ///
    /// If an active (non-revoked) consent already exists for the pair,
    /// updates `scopes`, `cimd_content_hash_at_grant`, clears
    /// `revalidation_required_at`, and refreshes `granted_at`.
    /// Otherwise inserts a new row.
    ///
    /// Returns the `consent_id` of the upserted row.
    pub async fn grant(
        &self,
        user_id: Uuid,
        client_id: &str,
        scopes: &str,
        cimd_content_hash: Option<&str>,
    ) -> Result<Uuid> {
        let now = (self.clock)();

        let existing = oauth_consent::Entity::find()
            .filter(oauth_consent::Column::UserId.eq(user_id))
            .filter(oauth_consent::Column::ClientId.eq(client_id))
            .filter(oauth_consent::Column::RevokedAt.is_null())
            .one(&self.db)
            .await
            .context_to()?;

        if let Some(row) = existing {
            let id = row.id;
            let mut active: oauth_consent::ActiveModel = row.into();
            active.scopes = Set(scopes.to_string());
            active.cimd_content_hash_at_grant = Set(cimd_content_hash.map(ToString::to_string));
            active.revalidation_required_at = Set(None);
            active.granted_at = Set(now);
            active.update(&self.db).await.context_to()?;
            Ok(id)
        } else {
            let id = Uuid::now_v7();
            oauth_consent::ActiveModel {
                id: Set(id),
                user_id: Set(user_id),
                client_id: Set(client_id.to_string()),
                scopes: Set(scopes.to_string()),
                cimd_content_hash_at_grant: Set(cimd_content_hash.map(ToString::to_string)),
                revalidation_required_at: Set(None),
                granted_at: Set(now),
                revoked_at: Set(None),
            }
            .insert(&self.db)
            .await
            .context_to()?;
            Ok(id)
        }
    }

    /// Revoke a consent and cascade to all active refresh tokens.
    ///
    /// Multi-statement atomic via `begin_immediate()/txn.commit()` (write-only;
    /// no read-then-write TOCTOU risk so `BEGIN DEFERRED` suffices).
    ///
    /// Steps:
    /// 1. UPDATE `oauth_consents SET revoked_at = now` WHERE `id` matches,
    ///    `user_id` matches (ownership check), and `revoked_at IS NULL`.
    /// 2. If no rows updated → `OAuthConsentError::NotFound`.
    /// 3. UPDATE `oauth_refresh_tokens SET revoked_at = now` WHERE
    ///    `consent_id` matches and `revoked_at IS NULL`.
    /// 4. Commit.
    pub async fn revoke(&self, consent_id: Uuid, user_id: Uuid) -> Result<()> {
        let now = (self.clock)();

        let txn = begin_immediate(&self.db).await.context_to()?;

        let result = oauth_consent::Entity::update_many()
            .col_expr(
                oauth_consent::Column::RevokedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(oauth_consent::Column::Id.eq(consent_id))
            .filter(oauth_consent::Column::UserId.eq(user_id))
            .filter(oauth_consent::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        if result.rows_affected == 0 {
            bail!(OAuthConsentError::NotFound);
        }

        oauth_refresh_token::Entity::update_many()
            .col_expr(
                oauth_refresh_token::Column::RevokedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .filter(oauth_refresh_token::Column::ConsentId.eq(consent_id))
            .filter(oauth_refresh_token::Column::RevokedAt.is_null())
            .exec(&txn)
            .await
            .context_to()?;

        txn.commit().await.context_to()?;

        Ok(())
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
    use uptrakit_shared_db::entity::{oauth_client, oauth_consent, oauth_refresh_token, user};
    use uptrakit_shared_types::MaskedEmail;

    fn make_clock(
        cell: Arc<Mutex<OffsetDateTime>>,
    ) -> Arc<dyn Fn() -> OffsetDateTime + Send + Sync> {
        Arc::new(move || *cell.lock())
    }

    async fn insert_user(db: &DatabaseConnection) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        user::ActiveModel {
            id: Set(id),
            email: Set(MaskedEmail::new(format!("test-{id}@example.com"))),
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

    async fn insert_oauth_client_with_trust(db: &DatabaseConnection, trusted: bool) -> String {
        let now = OffsetDateTime::now_utc();
        let client_id = format!("test-client-{}", Uuid::now_v7());
        oauth_client::ActiveModel {
            id: Set(client_id.clone()),
            client_name: Set("Test Client".to_string()),
            client_uri: Set(None),
            logo_uri: Set(None),
            redirect_uris: Set("https://example.com/callback".to_string()),
            default_scope: Set("openid mcp:read".to_string()),
            grant_types: Set("authorization_code refresh_token".to_string()),
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
            trusted_at: Set(if trusted { Some(now) } else { None }),
        }
        .insert(db)
        .await
        .expect("insert oauth_client");
        client_id
    }

    async fn insert_oauth_client(db: &DatabaseConnection) -> String {
        insert_oauth_client_with_trust(db, true).await
    }

    async fn insert_consent(
        db: &DatabaseConnection,
        user_id: Uuid,
        client_id: &str,
        scopes: &str,
        revalidation_required_at: Option<OffsetDateTime>,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        oauth_consent::ActiveModel {
            id: Set(id),
            user_id: Set(user_id),
            client_id: Set(client_id.to_string()),
            scopes: Set(scopes.to_string()),
            cimd_content_hash_at_grant: Set(None),
            revalidation_required_at: Set(revalidation_required_at),
            granted_at: Set(now),
            revoked_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_consent");
        id
    }

    async fn insert_refresh_token(
        db: &DatabaseConnection,
        consent_id: Uuid,
        user_id: Uuid,
        client_id: &str,
    ) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let id = Uuid::now_v7();
        let family_id = Uuid::now_v7();
        let token_hash = format!("hash-{id}");
        oauth_refresh_token::ActiveModel {
            id: Set(id),
            family_id: Set(family_id),
            parent_id: Set(None),
            token_hash: Set(token_hash),
            client_id: Set(client_id.to_string()),
            user_id: Set(user_id),
            consent_id: Set(consent_id),
            scope: Set("openid mcp:read".to_string()),
            resource: Set("https://mcp.example.com".to_string()),
            issued_at: Set(now),
            expires_at: Set(now + time::Duration::days(30)),
            family_expires_at: Set(now + time::Duration::days(90)),
            rotated_at: Set(None),
            revoked_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert oauth_refresh_token");
        id
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 1 — should_skip returns true when all conditions hold
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_skip_returns_true_when_all_conditions_hold() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;
        insert_consent(&db, user_id, &client_id, "openid mcp:read mcp:write", None).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db, make_clock(Arc::clone(&clock_cell)));

        let skip = svc
            .should_skip_prompt(user_id, &client_id, "openid mcp:read")
            .await
            .expect("should_skip_prompt should succeed");

        assert!(skip, "should return true when all conditions hold");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 2 — should_skip returns false when scope expansion needed
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_skip_returns_false_when_scope_expansion_needed() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;
        insert_consent(&db, user_id, &client_id, "openid", None).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db, make_clock(Arc::clone(&clock_cell)));

        let skip = svc
            .should_skip_prompt(user_id, &client_id, "openid mcp:read")
            .await
            .expect("should_skip_prompt should succeed");

        assert!(
            !skip,
            "should return false when requested scope not in granted"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 3 — should_skip returns false when revalidation required
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_skip_returns_false_when_revalidation_required() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;
        let revalidation_at = OffsetDateTime::now_utc();
        insert_consent(
            &db,
            user_id,
            &client_id,
            "openid mcp:read",
            Some(revalidation_at),
        )
        .await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db, make_clock(Arc::clone(&clock_cell)));

        let skip = svc
            .should_skip_prompt(user_id, &client_id, "openid mcp:read")
            .await
            .expect("should_skip_prompt should succeed");

        assert!(
            !skip,
            "should return false when revalidation_required_at IS NOT NULL"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 4 — should_skip returns false when no consent exists
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_skip_returns_false_when_no_consent_exists() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db, make_clock(Arc::clone(&clock_cell)));

        let skip = svc
            .should_skip_prompt(user_id, &client_id, "openid")
            .await
            .expect("should_skip_prompt should succeed");

        assert!(!skip, "should return false when no consent row exists");
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 5 — grant inserts a new consent row
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn grant_inserts_new_consent() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db.clone(), make_clock(Arc::clone(&clock_cell)));

        let consent_id = svc
            .grant(user_id, &client_id, "openid mcp:read", None)
            .await
            .expect("grant should succeed");

        let row = oauth_consent::Entity::find_by_id(consent_id)
            .one(&db)
            .await
            .expect("query consent")
            .expect("consent row must exist");

        assert_eq!(row.user_id, user_id);
        assert_eq!(row.client_id, client_id);
        assert_eq!(row.scopes, "openid mcp:read");
        assert!(row.revoked_at.is_none());
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 6 — grant updates existing active consent
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn grant_updates_existing_consent() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;

        // Insert a pre-existing consent with revalidation_required_at set.
        let original_id = insert_consent(
            &db,
            user_id,
            &client_id,
            "openid",
            Some(OffsetDateTime::now_utc()),
        )
        .await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db.clone(), make_clock(Arc::clone(&clock_cell)));

        let returned_id = svc
            .grant(user_id, &client_id, "openid mcp:read", Some("hash-abc"))
            .await
            .expect("grant should succeed");

        // Must return the original row's ID.
        assert_eq!(returned_id, original_id);

        let row = oauth_consent::Entity::find_by_id(original_id)
            .one(&db)
            .await
            .expect("query consent")
            .expect("consent row must exist");

        assert_eq!(row.scopes, "openid mcp:read");
        assert_eq!(row.cimd_content_hash_at_grant.as_deref(), Some("hash-abc"));
        assert!(
            row.revalidation_required_at.is_none(),
            "revalidation_required_at must be cleared"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 7 — revoke cascades to refresh tokens
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn revoke_cascades_to_refresh_tokens() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;
        let consent_id = insert_consent(&db, user_id, &client_id, "openid mcp:read", None).await;
        let token_id = insert_refresh_token(&db, consent_id, user_id, &client_id).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db.clone(), make_clock(Arc::clone(&clock_cell)));

        svc.revoke(consent_id, user_id)
            .await
            .expect("revoke should succeed");

        // Consent must be revoked.
        let consent_row = oauth_consent::Entity::find_by_id(consent_id)
            .one(&db)
            .await
            .expect("query consent")
            .expect("consent must exist");
        assert!(
            consent_row.revoked_at.is_some(),
            "consent must have revoked_at set"
        );

        // Refresh token must be revoked.
        let token_row = oauth_refresh_token::Entity::find_by_id(token_id)
            .one(&db)
            .await
            .expect("query refresh token")
            .expect("refresh token must exist");
        assert!(
            token_row.revoked_at.is_some(),
            "refresh token must have revoked_at set"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 8 — should_skip returns false when client is unverified
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn should_skip_returns_false_when_client_unverified() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let client_id = insert_oauth_client_with_trust(&db, false).await;
        insert_consent(&db, user_id, &client_id, "openid mcp:read mcp:write", None).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db, make_clock(Arc::clone(&clock_cell)));

        let skip = svc
            .should_skip_prompt(user_id, &client_id, "openid mcp:read")
            .await
            .expect("should_skip_prompt should succeed");

        assert!(
            !skip,
            "unverified client (trusted_at IS NULL) must always re-prompt"
        );
    }

    // ──────────────────────────────────────────────────────────────────────
    // Test 9 — revoke with wrong user returns NotFound
    // ──────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn revoke_with_wrong_user_returns_not_found() {
        let db = setup_migrated_db().await;
        let user_id = insert_user(&db).await;
        let other_user_id = insert_user(&db).await;
        let client_id = insert_oauth_client(&db).await;
        let consent_id = insert_consent(&db, user_id, &client_id, "openid mcp:read", None).await;

        let clock_cell = Arc::new(Mutex::new(OffsetDateTime::now_utc()));
        let svc = OAuthConsentService::new(db, make_clock(Arc::clone(&clock_cell)));

        let err = svc
            .revoke(consent_id, other_user_id)
            .await
            .expect_err("revoke with wrong user must fail");

        assert!(
            matches!(err.current_context(), OAuthConsentError::NotFound),
            "expected NotFound, got {:?}",
            err.current_context()
        );
    }
}
