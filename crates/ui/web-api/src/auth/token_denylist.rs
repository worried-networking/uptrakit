use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use uuid::Uuid;

/// In-memory denylist for JWT access tokens.
///
/// Supports two revocation modes:
/// - **By JTI**: denies a specific token until its expiry time.
/// - **By user**: denies all tokens for a user issued before a given timestamp.
///
/// Entries auto-expire and are periodically purged. This provides immediate
/// revocation on the same controller instance. Cross-instance revocation relies
/// on the natural JWT expiry (15 min) — a future enhancement could sync via a
/// DB-backed revocations table.
pub struct TokenDenylist {
    inner: Arc<RwLock<DenylistInner>>,
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
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(DenylistInner {
                jti_entries: HashMap::new(),
                user_entries: HashMap::new(),
            })),
        }
    }

    /// Deny a specific token by its JTI. It will remain denied until `exp`.
    pub async fn deny_token(&self, jti: &str, exp: i64) {
        self.inner
            .write()
            .await
            .jti_entries
            .insert(jti.to_string(), exp);
    }

    /// Deny all tokens for a user issued before `iat_cutoff` (unix timestamp).
    ///
    /// `purge_after` controls when this entry is eligible for removal. Callers
    /// should pass `iat_cutoff + ACCESS_TOKEN_EXPIRY_SECS` so that tokens
    /// issued just before revocation remain blocked until they expire naturally.
    ///
    /// If called multiple times for the same user, the entry with the latest
    /// `iat_cutoff` wins (monotonically advancing revocation).
    pub async fn deny_user(&self, user_id: Uuid, iat_cutoff: i64, purge_after: i64) {
        let mut inner = self.inner.write().await;
        let entry = inner.user_entries.entry(user_id).or_insert(UserDenyEntry {
            iat_cutoff: 0,
            purge_after: 0,
        });
        // Keep the most recent revocation (furthest-forward iat_cutoff).
        if iat_cutoff > entry.iat_cutoff {
            *entry = UserDenyEntry {
                iat_cutoff,
                purge_after,
            };
        }
    }

    /// Check if a token is denied.
    ///
    /// Returns `true` if:
    /// - The token's JTI is in the denylist, OR
    /// - The token's user has a user-level revocation where `iat < iat_cutoff`
    ///   (the token was issued before the revocation event).
    pub async fn is_denied(&self, jti: &str, user_id: &Uuid, iat: i64) -> bool {
        let inner = self.inner.read().await;

        // Check JTI-level denial
        if inner.jti_entries.contains_key(jti) {
            return true;
        }

        // Check user-level denial
        if let Some(entry) = inner.user_entries.get(user_id)
            && iat < entry.iat_cutoff
        {
            return true;
        }

        false
    }

    /// Remove expired entries from the denylist.
    ///
    /// Should be called periodically (e.g. every 5 minutes).
    pub async fn purge_expired(&self) {
        let now = time::OffsetDateTime::now_utc().unix_timestamp();
        let mut inner = self.inner.write().await;
        inner.jti_entries.retain(|_, exp| *exp > now);
        inner
            .user_entries
            .retain(|_, entry| entry.purge_after > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
