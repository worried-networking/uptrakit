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

struct DenylistInner {
    /// JTI → expiry timestamp (unix seconds). Token is denied until it would
    /// have expired anyway.
    jti_entries: HashMap<String, i64>,
    /// "user:{user_id}" → until timestamp. All tokens for this user issued
    /// before `until` are denied.
    user_entries: HashMap<Uuid, i64>,
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

    /// Deny all tokens for a user issued before `until` (unix timestamp).
    ///
    /// Typically called with `now + ACCESS_TOKEN_EXPIRY_SECS` to revoke all
    /// currently-valid tokens.
    pub async fn deny_user(&self, user_id: Uuid, until: i64) {
        let mut inner = self.inner.write().await;
        // Keep the latest (furthest into the future) revocation
        let entry = inner.user_entries.entry(user_id).or_insert(0);
        if until > *entry {
            *entry = until;
        }
    }

    /// Check if a token is denied.
    ///
    /// Returns `true` if:
    /// - The token's JTI is in the denylist, OR
    /// - The token's user has a user-level revocation with `until > iat`
    ///   (meaning the token was issued before the revocation).
    pub async fn is_denied(&self, jti: &str, user_id: &Uuid, iat: i64) -> bool {
        let inner = self.inner.read().await;

        // Check JTI-level denial
        if inner.jti_entries.contains_key(jti) {
            return true;
        }

        // Check user-level denial
        if let Some(&until) = inner.user_entries.get(user_id)
            && iat < until
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
        inner.user_entries.retain(|_, until| *until > now);
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
    async fn deny_user_revokes_tokens_issued_before() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([1; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        // Deny all tokens for this user issued before now + 900
        denylist.deny_user(user_id, now + 900).await;

        // Token issued at `now - 60` (before revocation) → denied
        assert!(denylist.is_denied("jti-1", &user_id, now - 60).await);

        // Token issued at `now + 901` (after revocation window) → allowed
        assert!(!denylist.is_denied("jti-2", &user_id, now + 901).await);
    }

    #[tokio::test]
    async fn tokens_issued_after_revocation_are_valid() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([2; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        denylist.deny_user(user_id, now).await;

        // Token issued exactly at the revocation time → allowed (iat == until, not <)
        assert!(!denylist.is_denied("jti-new", &user_id, now).await);

        // Token issued after → allowed
        assert!(!denylist.is_denied("jti-newer", &user_id, now + 1).await);
    }

    #[tokio::test]
    async fn expired_entries_are_purged() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([3; 16]);
        let past = time::OffsetDateTime::now_utc().unix_timestamp() - 100;

        // Add entries that are already expired
        denylist.deny_token("old-jti", past).await;
        denylist.deny_user(user_id, past).await;

        denylist.purge_expired().await;

        // Both should be removed — no longer denied
        assert!(!denylist.is_denied("old-jti", &user_id, 0).await);
        // User entry also purged (past < now)
        assert!(!denylist.is_denied("any", &user_id, past - 1).await);
    }

    #[tokio::test]
    async fn deny_user_keeps_latest_timestamp() {
        let denylist = TokenDenylist::new();
        let user_id = Uuid::from_bytes([4; 16]);
        let now = time::OffsetDateTime::now_utc().unix_timestamp();

        denylist.deny_user(user_id, now + 100).await;
        denylist.deny_user(user_id, now + 50).await; // earlier — should NOT reduce

        // Token issued at now + 99 should still be denied (because until is now + 100)
        assert!(denylist.is_denied("jti", &user_id, now + 99).await);
    }
}
