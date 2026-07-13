use std::collections::HashMap;
use std::sync::Arc;

use sea_orm::{
    ActiveModelTrait, ActiveValue, DatabaseConnection, EntityTrait, ExprTrait,
    sea_query::{Expr, OnConflict},
};
use tokio::sync::RwLock;
use uptrakit_shared_db::entity::{revoked_token_jti, revoked_token_user};
use uuid::Uuid;

/// In-memory denylist for JWT access tokens.
///
/// Supports two revocation modes:
/// - **By JTI**: denies a specific token until its expiry time.
/// - **By user**: denies all tokens for a user issued before a given timestamp.
///
/// Entries auto-expire and are periodically purged. This provides immediate
/// revocation on the same controller instance.
///
/// When a [`DatabaseConnection`] is provided (production mode), revocations
/// are also persisted to the `revoked_token_jtis` / `revoked_token_users`
/// tables so they survive controller restarts. Call [`load_from_db`] once
/// during startup to seed the in-memory cache from the DB before accepting
/// traffic.
///
/// Cross-instance propagation is handled by the caller: after calling
/// [`deny_token`] or [`deny_user`], publish a
/// `ControllerMessage::TokenRevoked` event via
/// `notification_service.publish_controller_event(...)`. Receiving
/// controllers call [`deny_token_remote`] / [`deny_user_remote`] which update
/// only the in-memory cache (the DB was already written by the origin).
///
/// [`load_from_db`]: TokenDenylist::load_from_db
/// [`deny_token`]: TokenDenylist::deny_token
/// [`deny_user`]: TokenDenylist::deny_user
/// [`deny_token_remote`]: TokenDenylist::deny_token_remote
/// [`deny_user_remote`]: TokenDenylist::deny_user_remote
pub struct TokenDenylist {
    inner: Arc<RwLock<DenylistInner>>,
    db: Option<DatabaseConnection>,
}

/// Tracks a user-level token revocation.
///
/// `iat_cutoff` is the revocation timestamp: tokens with `iat < iat_cutoff`
/// are denied. `purge_after` is when this entry can be removed — set to
/// `iat_cutoff + ACCESS_TOKEN_EXPIRY_SECS` so that pre-revocation tokens
/// (which can live up to 15 minutes) are still blocked until they naturally
/// expire.
#[derive(Clone, Copy)]
struct UserDenyEntry {
    /// Deny tokens issued strictly before this unix timestamp.
    iat_cutoff: i64,
    /// Remove this entry from the denylist after this unix timestamp.
    purge_after: i64,
}

struct DenylistInner {
    /// JTI → expiry timestamp (unix seconds). Token is denied until it would
    /// have expired anyway.
    jti_entries: HashMap<String, i64>,
    /// JTI → expiry timestamp (unix seconds). Token is explicitly allowed even
    /// when a user-level revocation would otherwise block it.
    jti_allowlist: HashMap<String, i64>,
    /// user_id → revocation entry. All tokens for this user with
    /// `iat < entry.iat_cutoff` are denied.
    user_entries: HashMap<Uuid, UserDenyEntry>,
}

impl Default for TokenDenylist {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenDenylist {
    /// In-memory only constructor.
    ///
    /// Used in unit tests and contexts where no DB is available. Revocations
    /// are not persisted and do not survive restarts.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(DenylistInner {
                jti_entries: HashMap::new(),
                jti_allowlist: HashMap::new(),
                user_entries: HashMap::new(),
            })),
            db: None,
        }
    }

    /// Production constructor.
    ///
    /// Writes go to the `revoked_token_jtis` / `revoked_token_users` DB
    /// tables. Call [`load_from_db`](Self::load_from_db) once after
    /// construction to seed the in-memory cache before accepting traffic.
    pub fn new_with_db(db: DatabaseConnection) -> Self {
        Self {
            inner: Arc::new(RwLock::new(DenylistInner {
                jti_entries: HashMap::new(),
                jti_allowlist: HashMap::new(),
                user_entries: HashMap::new(),
            })),
            db: Some(db),
        }
    }

    /// Seed the in-memory cache from the DB.
    ///
    /// Call once at startup after [`new_with_db`](Self::new_with_db) and
    /// before the server begins accepting requests. The in-memory cache is
    /// populated from both DB tables; subsequent [`is_denied`](Self::is_denied)
    /// calls do not touch the DB.
    pub async fn load_from_db(&self) -> Result<(), sea_orm::DbErr> {
        let Some(db) = &self.db else {
            return Ok(());
        };

        let jti_rows = revoked_token_jti::Entity::find().all(db).await?;
        let user_rows = revoked_token_user::Entity::find().all(db).await?;

        let mut inner = self.inner.write().await;
        for row in jti_rows {
            inner.jti_entries.insert(row.jti, row.expires_at);
        }
        for row in user_rows {
            inner.user_entries.insert(
                row.user_id,
                UserDenyEntry {
                    iat_cutoff: row.iat_cutoff,
                    purge_after: row.purge_after,
                },
            );
        }
        Ok(())
    }

    /// Deny a specific token by its JTI.
    ///
    /// The in-memory cache is updated immediately. When a DB is configured the
    /// revocation is also persisted (upsert). The caller should then publish a
    /// `ControllerMessage::TokenRevoked` NATS event so that other controller
    /// instances update their in-memory caches via [`deny_token_remote`](Self::deny_token_remote).
    pub async fn deny_token(&self, jti: &str, exp: i64) {
        self.inner
            .write()
            .await
            .jti_entries
            .insert(jti.to_string(), exp);

        if let Some(db) = &self.db {
            let model = revoked_token_jti::ActiveModel {
                jti: ActiveValue::Set(jti.to_string()),
                expires_at: ActiveValue::Set(exp),
            };
            if let Err(e) = model.insert(db).await {
                // Non-fatal: in-memory cache is already updated. Log and
                // continue — the revocation is effective on this instance.
                tracing::warn!(jti, error = %e, "failed to persist JTI revocation to DB");
            }
        }
    }

    /// Deny all tokens for a user issued before `iat_cutoff` (unix timestamp).
    ///
    /// The in-memory cache is updated immediately. When a DB is configured the
    /// revocation is also persisted (upsert with monotonic `iat_cutoff` wins).
    /// The caller should then publish a `ControllerMessage::TokenRevoked` NATS
    /// event.
    ///
    /// If called multiple times for the same user, the entry with the latest
    /// `iat_cutoff` wins (monotonically advancing revocation).
    pub async fn deny_user(&self, user_id: Uuid, iat_cutoff: i64, purge_after: i64) {
        let updated = {
            let mut inner = self.inner.write().await;
            let entry = inner.user_entries.entry(user_id).or_insert(UserDenyEntry {
                iat_cutoff: 0,
                purge_after: 0,
            });
            if iat_cutoff > entry.iat_cutoff {
                *entry = UserDenyEntry {
                    iat_cutoff,
                    purge_after,
                };
                true
            } else {
                false
            }
        };

        if updated && let Some(db) = &self.db {
            let model = revoked_token_user::ActiveModel {
                user_id: ActiveValue::Set(user_id),
                iat_cutoff: ActiveValue::Set(iat_cutoff),
                purge_after: ActiveValue::Set(purge_after),
            };
            // Monotonic upsert: on PK conflict, advance the row ONLY when the incoming
            // cutoff is newer than the stored one. Guards against a stale-cache instance
            // regressing the revocation horizon (single-column PK => OnConflict::column).
            // exec_without_returning() gives the raw rows-affected count directly — needed
            // to detect a WHERE-guard-suppressed no-op, which plain exec()'s InsertResult
            // (last_insert_id only) cannot distinguish on this Set-supplied PK.
            let result = revoked_token_user::Entity::insert(model)
                .on_conflict(
                    OnConflict::column(revoked_token_user::Column::UserId)
                        .update_columns([
                            revoked_token_user::Column::IatCutoff,
                            revoked_token_user::Column::PurgeAfter,
                        ])
                        .action_and_where(
                            Expr::col(revoked_token_user::Column::IatCutoff).lt(iat_cutoff),
                        )
                        .to_owned(),
                )
                .exec_without_returning(db)
                .await;
            match result {
                Ok(0) => {
                    // The WHERE guard suppressed the write: another instance already holds a
                    // HIGHER cutoff in the DB than the one we just tried to write (this
                    // instance's own cache was stale). Reconcile our in-memory entry UPWARD
                    // to the stored value so this instance stops under-revoking in the gap
                    // — do not wait for load_from_db at restart or a NATS TokenRevoked catch-up.
                    match revoked_token_user::Entity::find_by_id(user_id)
                        .one(db)
                        .await
                    {
                        Ok(Some(row)) => {
                            let mut inner = self.inner.write().await;
                            let entry =
                                inner.user_entries.entry(user_id).or_insert(UserDenyEntry {
                                    iat_cutoff: 0,
                                    purge_after: 0,
                                });
                            if row.iat_cutoff > entry.iat_cutoff {
                                *entry = UserDenyEntry {
                                    iat_cutoff: row.iat_cutoff,
                                    purge_after: row.purge_after,
                                };
                            }
                        }
                        Ok(None) => {
                            // Should not happen (we just conflicted on this PK) — leave the
                            // cache as-is; not fatal, next load_from_db/NATS event corrects it.
                            tracing::warn!(
                                %user_id,
                                "denylist upsert no-op but row not found on re-read"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                %user_id,
                                error = %e,
                                "failed to re-read denylist row after suppressed upsert"
                            );
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!(%user_id, error = %e, "failed to persist user revocation to DB");
                }
            }
        }
    }

    /// Deny all tokens for a user issued before `iat_cutoff`, but explicitly
    /// allow the token identified by `jti` (which expires at `jti_exp`).
    ///
    /// This is used during token rotation (e.g. password change or email change)
    /// where the caller already holds a fresh token and needs it to remain valid
    /// while all pre-rotation tokens are revoked.
    pub async fn deny_user_except(
        &self,
        user_id: Uuid,
        jti: &str,
        jti_exp: i64,
        iat_cutoff: i64,
        purge_after: i64,
    ) {
        self.deny_user(user_id, iat_cutoff, purge_after).await;
        self.inner
            .write()
            .await
            .jti_allowlist
            .insert(jti.to_string(), jti_exp);
    }

    /// Apply a JTI revocation received from another controller via NATS.
    ///
    /// Updates the in-memory cache only — the originating controller already
    /// wrote to the DB. This avoids double-writes in multi-instance deployments.
    pub async fn deny_token_remote(&self, jti: &str, exp: i64) {
        self.inner
            .write()
            .await
            .jti_entries
            .insert(jti.to_string(), exp);
    }

    /// Apply a user revocation received from another controller via NATS.
    ///
    /// Updates the in-memory cache only (monotonic `iat_cutoff` wins).
    pub async fn deny_user_remote(&self, user_id: Uuid, iat_cutoff: i64, purge_after: i64) {
        let mut inner = self.inner.write().await;
        let entry = inner.user_entries.entry(user_id).or_insert(UserDenyEntry {
            iat_cutoff: 0,
            purge_after: 0,
        });
        if iat_cutoff > entry.iat_cutoff {
            *entry = UserDenyEntry {
                iat_cutoff,
                purge_after,
            };
        }
    }

    /// Check if a token is denied.
    ///
    /// In-memory only — authoritative after startup seeding (`load_from_db`)
    /// and NATS-based cross-instance propagation.
    ///
    /// Returns `true` if:
    /// - The token's JTI is in the denylist, OR
    /// - The token's user has a user-level revocation where `iat < iat_cutoff`
    ///   (the token was issued before the revocation event).
    pub async fn is_denied(&self, jti: &str, user_id: &Uuid, iat: i64) -> bool {
        let inner = self.inner.read().await;

        // JTI-level denial
        if inner.jti_entries.contains_key(jti) {
            return true;
        }

        // JTI allowlist — bypasses user-level denial
        if inner.jti_allowlist.contains_key(jti) {
            return false;
        }

        // User-level denial
        if let Some(entry) = inner.user_entries.get(user_id)
            && iat < entry.iat_cutoff
        {
            return true;
        }

        false
    }

    /// Remove expired entries from the in-memory cache and the DB (if configured).
    ///
    /// Should be called periodically (e.g. every 5 minutes).
    pub async fn purge_expired(&self) {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        {
            let mut inner = self.inner.write().await;
            inner.jti_entries.retain(|_, exp| *exp > now);
            inner.jti_allowlist.retain(|_, exp| *exp > now);
            inner
                .user_entries
                .retain(|_, entry| entry.purge_after > now);
        }

        if let Some(db) = &self.db {
            use sea_orm::{ColumnTrait, QueryFilter};

            if let Err(e) = revoked_token_jti::Entity::delete_many()
                .filter(revoked_token_jti::Column::ExpiresAt.lte(now))
                .exec(db)
                .await
            {
                tracing::warn!(error = %e, "failed to purge expired JTI revocations from DB");
            }

            if let Err(e) = revoked_token_user::Entity::delete_many()
                .filter(revoked_token_user::Column::PurgeAfter.lte(now))
                .exec(db)
                .await
            {
                tracing::warn!(error = %e, "failed to purge expired user revocations from DB");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectionTrait, Database, Schema};

    async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.expect("test db");

        let schema = Schema::new(db.get_database_backend());

        db.execute(&schema.create_table_from_entity(revoked_token_jti::Entity))
            .await
            .expect("create revoked_token_jtis");
        db.execute(&schema.create_table_from_entity(revoked_token_user::Entity))
            .await
            .expect("create revoked_token_users");

        db
    }

    #[tokio::test]
    async fn denied_jti_is_rejected() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::nil();
        let future = time::OffsetDateTime::now_utc().unix_timestamp() + 900;

        denylist.deny_token("token-123", future).await;

        assert!(denylist.is_denied("token-123", &user_id, 0).await);
        assert!(!denylist.is_denied("token-other", &user_id, 0).await);
    }

    #[tokio::test]
    async fn deny_user_revokes_tokens_issued_before_cutoff() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([1; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Deny all tokens issued before `now` (logout time), keep entry for 900 s.
        denylist.deny_user(user_id, now, now + 900).await;

        // Token issued before logout → denied
        assert!(denylist.is_denied("jti-old", &user_id, now - 60).await);

        // Token issued exactly at logout time → allowed (strict less-than)
        assert!(!denylist.is_denied("jti-exact", &user_id, now).await);

        // Token issued after logout → allowed
        assert!(!denylist.is_denied("jti-new", &user_id, now + 1).await);
    }

    #[tokio::test]
    async fn tokens_issued_after_revocation_are_valid() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([2; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        denylist.deny_user(user_id, now, now + 900).await;

        // Token issued exactly at the revocation time → allowed (iat == iat_cutoff, not <)
        assert!(!denylist.is_denied("jti-new", &user_id, now).await);

        // Token issued after → allowed
        assert!(!denylist.is_denied("jti-newer", &user_id, now + 1).await);
    }

    #[tokio::test]
    async fn expired_entries_are_purged() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([3; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // purge_after in the past — entry should be removed on next purge
        let past_cutoff = now - 1000;
        let past_purge = now - 100;

        denylist.deny_token("old-jti", past_purge).await;
        denylist.deny_user(user_id, past_cutoff, past_purge).await;

        denylist.purge_expired().await;

        // JTI entry purged
        assert!(!denylist.is_denied("old-jti", &user_id, 0).await);
        // User entry also purged
        assert!(!denylist.is_denied("any", &user_id, past_cutoff - 1).await);
    }

    #[tokio::test]
    async fn user_entry_not_purged_while_purge_after_is_future() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([5; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        // iat_cutoff is in the past but purge_after is in the future
        let iat_cutoff = now - 5;
        let purge_after = now + 900;

        denylist.deny_user(user_id, iat_cutoff, purge_after).await;
        denylist.purge_expired().await;

        // Entry should still be present — tokens before iat_cutoff are still denied
        assert!(
            denylist
                .is_denied("jti-old", &user_id, iat_cutoff - 1)
                .await
        );
        // But tokens at or after iat_cutoff are allowed
        assert!(!denylist.is_denied("jti-new", &user_id, iat_cutoff).await);
    }

    #[tokio::test]
    async fn deny_user_keeps_latest_cutoff() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([4; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // First logout at now + 100
        denylist.deny_user(user_id, now + 100, now + 1000).await;
        // Second (earlier) logout — should NOT reduce the cutoff
        denylist.deny_user(user_id, now + 50, now + 950).await;

        // Token issued at now + 99 should still be denied (cutoff is now + 100)
        assert!(denylist.is_denied("jti", &user_id, now + 99).await);
        // Token at now + 100 is allowed
        assert!(!denylist.is_denied("jti2", &user_id, now + 100).await);
    }

    // ── DB-backed tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn deny_token_writes_to_db() {
        let db = setup_db().await;
        let denylist = TokenDenylist::new_with_db(db.clone());
        let exp = time::OffsetDateTime::now_utc().unix_timestamp() + 900;

        denylist.deny_token("db-jti", exp).await;

        // Row must exist in DB
        let row = revoked_token_jti::Entity::find_by_id("db-jti".to_string())
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.expires_at, exp);
    }

    #[tokio::test]
    async fn deny_user_writes_to_db() {
        let db = setup_db().await;
        let denylist = TokenDenylist::new_with_db(db.clone());
        let user_id = Uuid::from_bytes([10; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        denylist.deny_user(user_id, now, now + 900).await;

        let row = revoked_token_user::Entity::find_by_id(user_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.iat_cutoff, now);
        assert_eq!(row.purge_after, now + 900);
    }

    #[tokio::test]
    async fn load_from_db_seeds_memory() {
        let db = setup_db().await;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let user_id = Uuid::from_bytes([11; 16]);
        let exp = now + 900;

        // Pre-populate DB rows directly.
        revoked_token_jti::ActiveModel {
            jti: ActiveValue::Set("seeded-jti".to_string()),
            expires_at: ActiveValue::Set(exp),
        }
        .insert(&db)
        .await
        .expect("insert jti");

        revoked_token_user::ActiveModel {
            user_id: ActiveValue::Set(user_id),
            iat_cutoff: ActiveValue::Set(now),
            purge_after: ActiveValue::Set(now + 900),
        }
        .insert(&db)
        .await
        .expect("insert user");

        // Create a fresh denylist and seed from DB.
        let denylist = TokenDenylist::new_with_db(db.clone());
        denylist.load_from_db().await.expect("load_from_db");

        // DB-seeded JTI must be blocked.
        assert!(denylist.is_denied("seeded-jti", &Uuid::nil(), 0).await);
        // Unknown JTI must be allowed.
        assert!(!denylist.is_denied("unknown-jti", &Uuid::nil(), 0).await);
        // User tokens issued before cutoff must be blocked.
        assert!(denylist.is_denied("any", &user_id, now - 1).await);
        // User tokens issued at cutoff must be allowed.
        assert!(!denylist.is_denied("any", &user_id, now).await);
    }

    #[tokio::test]
    async fn purge_expired_cleans_db() {
        let db = setup_db().await;
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let past = now - 100;
        let user_id = Uuid::from_bytes([12; 16]);

        let denylist = TokenDenylist::new_with_db(db.clone());
        // Insert entries with expiry in the past.
        denylist.deny_token("expired-jti", past).await;
        denylist.deny_user(user_id, past - 1000, past).await;

        denylist.purge_expired().await;

        // Both DB rows must be deleted.
        assert!(
            revoked_token_jti::Entity::find_by_id("expired-jti".to_string())
                .one(&db)
                .await
                .expect("query")
                .is_none()
        );
        assert!(
            revoked_token_user::Entity::find_by_id(user_id)
                .one(&db)
                .await
                .expect("query")
                .is_none()
        );
    }

    #[tokio::test]
    async fn deny_user_except_keeps_allowlisted_jti_valid() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([20; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Allow current JTI, deny all others issued before now.
        denylist
            .deny_user_except(user_id, "current-jti", now + 900, now, now + 900)
            .await;

        // Current JTI must pass even though iat < iat_cutoff.
        assert!(!denylist.is_denied("current-jti", &user_id, now - 1).await);

        // Any other JTI issued before cutoff must be denied.
        assert!(denylist.is_denied("old-jti", &user_id, now - 1).await);

        // Token issued at or after cutoff must be allowed regardless.
        assert!(!denylist.is_denied("new-jti", &user_id, now).await);
    }

    #[tokio::test]
    async fn purge_expired_removes_expired_allowlist_entries() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([21; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let past = now - 100;

        // Add an already-expired allowlist entry.
        denylist
            .deny_user_except(user_id, "expired-allowed-jti", past, now - 200, past)
            .await;

        denylist.purge_expired().await;

        // After purge, allowlist entry is gone. user_entries also pruned (purge_after = past).
        // Both are gone so no block applies — token must be allowed.
        assert!(
            !denylist
                .is_denied("expired-allowed-jti", &user_id, now - 300)
                .await
        );
    }

    #[tokio::test]
    async fn deny_token_remote_updates_memory_only() {
        let db = setup_db().await;
        let denylist = TokenDenylist::new_with_db(db.clone());
        let exp = time::OffsetDateTime::now_utc().unix_timestamp() + 900;

        // Remote revocation must NOT write to DB.
        denylist.deny_token_remote("remote-jti", exp).await;

        // In-memory check must return true.
        assert!(denylist.is_denied("remote-jti", &Uuid::nil(), 0).await);

        // DB must remain empty.
        assert!(
            revoked_token_jti::Entity::find_by_id("remote-jti".to_string())
                .one(&db)
                .await
                .expect("query")
                .is_none(),
            "deny_token_remote must not write to DB"
        );
    }

    #[tokio::test]
    async fn deny_user_db_write_is_monotonic() {
        let db = setup_db().await;
        let user_id = Uuid::new_v4();

        // Instance A persists a high cutoff.
        let denylist_a = TokenDenylist::new_with_db(db.clone());
        denylist_a.deny_user(user_id, 1_000, 1_900).await;

        // Instance B has a stale in-memory view (fresh instance, entry defaults to 0)
        // and tries to write a LOWER cutoff. The DB row must not regress.
        let denylist_b = TokenDenylist::new_with_db(db.clone());
        denylist_b.deny_user(user_id, 500, 1_400).await;

        let row = revoked_token_user::Entity::find_by_id(user_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(
            row.iat_cutoff, 1_000,
            "lower cutoff must not overwrite higher"
        );
        assert_eq!(row.purge_after, 1_900);

        // A legitimately higher cutoff DOES advance the row.
        denylist_b.deny_user(user_id, 2_000, 2_900).await;
        let row = revoked_token_user::Entity::find_by_id(user_id)
            .one(&db)
            .await
            .expect("query")
            .expect("row exists");
        assert_eq!(row.iat_cutoff, 2_000);
        assert_eq!(row.purge_after, 2_900);
    }

    /// Regression test for the losing-instance cache-divergence fix (spec-directed): when
    /// instance B's suppressed upsert no-ops against a higher DB cutoff set by instance A,
    /// B's own in-memory entry must be reconciled UPWARD to the DB value — not left at the
    /// lower value B tried to write. Otherwise B would under-revoke in the gap until restart
    /// or a NATS TokenRevoked catch-up.
    #[tokio::test]
    async fn deny_user_reconciles_cache_upward_on_suppressed_upsert() {
        let db = setup_db().await;
        let user_id = Uuid::new_v4();

        // Instance A persists a high cutoff.
        let denylist_a = TokenDenylist::new_with_db(db.clone());
        denylist_a.deny_user(user_id, 1_000, 1_900).await;

        // Instance B has a stale (default-zero) in-memory entry and writes a LOWER cutoff.
        // Its own gate passes locally (500 > 0), so its cache is provisionally set to 500,
        // but the DB upsert is suppressed by the WHERE guard (DB stays at 1000).
        let denylist_b = TokenDenylist::new_with_db(db.clone());
        denylist_b.deny_user(user_id, 500, 1_400).await;

        // B's in-memory entry must now reflect the DB's higher cutoff (1000), not the 500
        // it tried to write — i.e. B must not accept a token with iat in [500, 1000).
        assert!(
            denylist_b.is_denied("unrelated-jti", &user_id, 750).await,
            "losing instance's cache must be reconciled upward to the DB cutoff"
        );
        assert!(!denylist_b.is_denied("unrelated-jti", &user_id, 1_500).await);
    }
}
