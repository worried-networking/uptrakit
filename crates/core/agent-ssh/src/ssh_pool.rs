//! Persistent SSH connection pool for the SSH agent.
//!
//! [`SshConnectionPool`] maintains one authenticated SSH session per enrolled
//! host and hands out `Arc<SshSession>` to callers.  Because
//! [`SshSession::exec_command_streaming`] takes `&self`, multiple holders of
//! the same `Arc` can open independent SSH channels on the same TCP connection
//! concurrently — this is the multiplexing benefit.
//!
//! Sessions that have been idle for longer than [`IDLE_TTL`] are evicted when
//! the next [`acquire`] for that host is requested.  Callers should also call
//! [`evict`] explicitly when they detect a connection-level error so that the
//! next request gets a fresh session rather than the stale one.
//!
//! [`acquire`]: SshConnectionPool::acquire
//! [`evict`]: SshConnectionPool::evict

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;
use uuid::Uuid;

use crate::db::entity::ssh_host::Model;
use crate::error::Result;
use crate::ssh_transport::{AuthMethod, SshConnectionConfig, SshSession};

/// Maximum idle time before a pooled session is considered stale and
/// re-established.  Chosen to be safely below typical SSH server idle
/// timeouts (OpenSSH default: `ClientAliveInterval 0` = no server-side
/// timeout, but many production configurations use 300–600 s).
pub(crate) const IDLE_TTL: Duration = Duration::from_secs(300);

/// Timeout for establishing a new SSH connection from the pool.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

// ── TTL helper ────────────────────────────────────────────────────────────────

/// Return `true` if the session last used at `last_used` has exceeded
/// [`IDLE_TTL`] and should be evicted.
///
/// Extracted as a free function so it can be tested independently using
/// virtual time (`tokio::test(start_paused = true)`).
fn is_expired(last_used: Instant) -> bool {
    last_used.elapsed() >= IDLE_TTL
}

// ── Pool entry ────────────────────────────────────────────────────────────────

struct PoolEntry {
    session: Arc<SshSession>,
    /// Time the session was last returned to a caller.
    last_used: Instant,
}

// ── Pool ─────────────────────────────────────────────────────────────────────

/// A pool of authenticated SSH sessions, keyed by SSH host ID.
///
/// `SshConnectionPool` is a cheaply-cloneable handle: cloning it produces a
/// second handle that shares the same underlying session map.  This allows
/// the pool to be moved into `tokio::spawn` tasks without wrapping the entire
/// handler in an `Arc`.
///
/// A single pool instance lives on `SshAgentHandler` for the lifetime of the
/// controller connection.
#[derive(Clone)]
pub struct SshConnectionPool {
    sessions: Arc<Mutex<HashMap<Uuid, PoolEntry>>>,
}

impl SshConnectionPool {
    /// Create a new, empty pool.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Acquire a session for the given SSH host.
    ///
    /// Returns an existing session if one is cached and was used within
    /// [`IDLE_TTL`].  If the cached session has expired, or no session exists,
    /// a new SSH connection is established and stored in the pool.
    ///
    /// Returns an error if the connection attempt fails.  Nothing is stored in
    /// the pool on failure.
    pub async fn acquire(&self, host: &Model) -> Result<Arc<SshSession>> {
        // Check the pool under the lock first.
        {
            let mut pool = self.sessions.lock();
            if let Some(entry) = pool.get_mut(&host.id) {
                if !is_expired(entry.last_used) {
                    entry.last_used = Instant::now();
                    tracing::debug!(
                        host_id = %host.id,
                        host_name = %host.name,
                        hostname = %host.hostname,
                        "reusing pooled SSH session"
                    );
                    return Ok(Arc::clone(&entry.session));
                }
                // TTL expired — remove now and reconnect below.
                tracing::debug!(
                    host_id = %host.id,
                    host_name = %host.name,
                    hostname = %host.hostname,
                    "pooled SSH session expired (idle TTL exceeded), reconnecting"
                );
                pool.remove(&host.id);
            }
        }

        // Establish a new connection outside the lock so other hosts are not
        // blocked while we wait for the TCP + SSH handshake.
        tracing::debug!(
            host_id = %host.id,
            host_name = %host.name,
            hostname = %host.hostname,
            "establishing new SSH session for pool"
        );
        let session = establish_session(host).await?;

        // Re-acquire the lock and use the Entry API to handle a concurrent
        // acquire() that may have inserted a session while we were connecting.
        let session = {
            let mut pool = self.sessions.lock();
            match pool.entry(host.id) {
                std::collections::hash_map::Entry::Vacant(v) => Arc::clone(
                    &v.insert(PoolEntry {
                        session,
                        last_used: Instant::now(),
                    })
                    .session,
                ),
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    // A concurrent acquire() beat us to the insert — reuse its
                    // session and let our newly-established one drop (russh
                    // closes the TCP connection on Drop).
                    o.get_mut().last_used = Instant::now();
                    Arc::clone(&o.get().session)
                }
            }
        };

        Ok(session)
    }

    /// Evict the pool entry for `host_id`.
    ///
    /// Call this after a connection-level error so the next [`acquire`]
    /// for that host opens a fresh connection rather than returning the
    /// defunct session.
    ///
    /// [`acquire`]: Self::acquire
    pub async fn evict(&self, host_id: Uuid) {
        let removed = self.sessions.lock().remove(&host_id).is_some();
        if removed {
            tracing::debug!(host_id = %host_id, "evicted SSH session from pool");
        }
    }

    /// Disconnect all pooled sessions gracefully and clear the pool.
    ///
    /// Should be called on service shutdown so remote hosts receive a clean
    /// SSH disconnect instead of a silent idle drop.
    pub async fn disconnect_all(&self) {
        let sessions: Vec<Arc<SshSession>> = {
            let mut pool = self.sessions.lock();
            pool.drain().map(|(_, e)| e.session).collect()
        };

        if sessions.is_empty() {
            return;
        }

        tracing::debug!(
            count = sessions.len(),
            "disconnecting all pooled SSH sessions"
        );

        for session in sessions {
            // At shutdown there should be no other Arc holders.  Skip
            // graceful disconnect if there are (the OS will close the socket).
            if let Ok(owned) = Arc::try_unwrap(session) {
                owned.disconnect().await;
            }
        }
    }

    /// Return whether `host_id` has a cached session.
    #[cfg(test)]
    pub async fn is_cached(&self, host_id: Uuid) -> bool {
        self.sessions.lock().contains_key(&host_id)
    }

    /// Return the number of currently cached sessions.
    #[cfg(test)]
    pub async fn len(&self) -> usize {
        self.sessions.lock().len()
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Establish a fresh authenticated SSH session for `host`.
pub(crate) async fn establish_session(host: &Model) -> Result<Arc<SshSession>> {
    let config = SshConnectionConfig {
        hostname: host.hostname.clone(),
        port: host.port as u16,
        connect_timeout: CONNECT_TIMEOUT,
    };
    let private_key_pem = host.private_key.expose_secret();
    let auth = AuthMethod::PrivateKey(private_key_pem);

    let (session, _fingerprint) = crate::ssh_transport::connect_and_authenticate(
        &config,
        &host.username,
        &auth,
        host.host_key_fingerprint.as_deref(),
    )
    .await?;

    Ok(Arc::new(session))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── TTL helper ───────────────────────────────────────────────────────────
    //
    // is_expired() uses tokio::time::Instant so virtual time (start_paused)
    // works without a real SSH connection.

    #[tokio::test(start_paused = true)]
    async fn fresh_entry_is_not_expired() {
        let last_used = Instant::now();
        assert!(!is_expired(last_used));
    }

    #[tokio::test(start_paused = true)]
    async fn entry_expires_after_idle_ttl() {
        let last_used = Instant::now();
        tokio::time::advance(IDLE_TTL).await;
        // Exactly at TTL boundary: expired.
        assert!(is_expired(last_used));
    }

    #[tokio::test(start_paused = true)]
    async fn entry_not_expired_just_before_ttl() {
        let last_used = Instant::now();
        tokio::time::advance(IDLE_TTL - Duration::from_millis(1)).await;
        assert!(!is_expired(last_used));
    }

    // ── evict ────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn evict_nonexistent_is_noop() {
        let pool = SshConnectionPool::new();
        pool.evict(Uuid::nil()).await;
        assert_eq!(pool.len().await, 0);
    }

    // ── disconnect_all ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn disconnect_all_empty_pool_is_noop() {
        let pool = SshConnectionPool::new();
        pool.disconnect_all().await;
        assert_eq!(pool.len().await, 0);
    }

    // ── is_cached / len ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn is_cached_false_for_missing_host() {
        let pool = SshConnectionPool::new();
        assert!(!pool.is_cached(Uuid::nil()).await);
    }

    #[tokio::test]
    async fn len_zero_for_empty_pool() {
        let pool = SshConnectionPool::new();
        assert_eq!(pool.len().await, 0);
    }

    // ── acquire() TOCTOU fix — Entry API semantics ───────────────────────────
    //
    // The TOCTOU race in acquire() is: two concurrent callers both see no
    // cached entry, both call establish_session(), and then both try to insert.
    // The fix uses HashMap::entry() so only one session is kept and the other
    // is dropped immediately.
    //
    // SshSession requires a real TCP connection so we cannot create it in unit
    // tests.  The Entry API logic is tested here at the HashMap level using a
    // plain sentinel value as a proxy — this verifies that whichever "session"
    // arrives at the occupied branch is discarded and the first-inserted value
    // is returned, maintaining pool.len() == 1.

    #[test]
    fn entry_api_first_insert_wins_and_pool_stays_at_one() {
        // Simulate the fixed code path in acquire() using a plain HashMap
        // (same Entry API, same branch logic, no real SSH needed).
        let mut map: std::collections::HashMap<Uuid, u32> = std::collections::HashMap::new();
        let host_id = Uuid::nil();

        // --- First "acquire" (vacant) ---
        let first_id = 1u32;
        let result1 = match map.entry(host_id) {
            std::collections::hash_map::Entry::Vacant(v) => *v.insert(first_id),
            std::collections::hash_map::Entry::Occupied(o) => *o.get(),
        };
        assert_eq!(
            result1, first_id,
            "first insert should return inserted value"
        );
        assert_eq!(
            map.len(),
            1,
            "pool must have exactly one entry after first insert"
        );

        // --- Second "acquire" (occupied — concurrent caller beat us) ---
        let second_id = 2u32;
        let result2 = match map.entry(host_id) {
            std::collections::hash_map::Entry::Vacant(v) => *v.insert(second_id),
            // Occupied → reuse existing; second_id is dropped (never stored).
            std::collections::hash_map::Entry::Occupied(o) => *o.get(),
        };
        assert_eq!(
            result2, first_id,
            "concurrent insert must return the first-inserted session"
        );
        assert_eq!(
            map.len(),
            1,
            "pool must still have exactly one entry after concurrent insert"
        );
    }

    // Acquire with a real SSH server is covered by ignored integration tests.
}
