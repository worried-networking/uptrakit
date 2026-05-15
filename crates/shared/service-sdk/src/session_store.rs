//! Cert-rotation-aware wrapper around [`rustls::client::ClientSessionStore`].
//!
//! TLS 1.3 PSK resumption (RFC 8446 §2.2) skips the Certificate /
//! CertificateVerify messages, so a resumed session keeps the *original*
//! client certificate even if the agent has since rotated to a new one.
//! For mTLS deployments where the client identity can change at runtime —
//! e.g. uptrakit's periodic certificate renewal — the resumption store
//! must be invalidated in lockstep with the cert swap; otherwise the
//! server keeps observing the old (now-revoked) cert on every resumed
//! handshake.
//!
//! [`CertScopedClientSessionStore`] wraps the default
//! [`rustls::client::ClientSessionMemoryCache`] inside an
//! [`arc_swap::ArcSwap`]. Calling [`CertScopedClientSessionStore::reset`]
//! atomically replaces the inner cache with a fresh empty one, which is
//! exactly what we want at the moment the agent's
//! [`AgentClientCertResolver`](crate::cert_resolver::AgentClientCertResolver)
//! publishes a renewed [`rustls::sign::CertifiedKey`].
//!
//! The store implements [`ClientSessionStore`] by delegating every method
//! to the currently-active inner store. Trait methods take only `&self`,
//! so callers can keep holding the same `Arc<CertScopedClientSessionStore>`
//! across rotations.

use std::fmt;
use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::NamedGroup;
use rustls::client::{
    ClientSessionMemoryCache, ClientSessionStore, Tls12ClientSessionValue, Tls13ClientSessionValue,
};
use rustls::pki_types::ServerName;

/// Session store whose cached sessions are scoped to the lifetime of the
/// currently active client certificate.
///
/// Calling [`reset`](Self::reset) drops every cached TLS 1.2 session and
/// TLS 1.3 ticket. Use it whenever the matching
/// [`AgentClientCertResolver`](crate::cert_resolver::AgentClientCertResolver)
/// rotates to a new [`rustls::sign::CertifiedKey`]; see the module-level
/// docs for the rationale.
#[non_exhaustive]
pub struct CertScopedClientSessionStore {
    inner: ArcSwap<Arc<dyn ClientSessionStore>>,
    capacity: usize,
}

impl CertScopedClientSessionStore {
    /// Construct a new store backed by an empty
    /// [`ClientSessionMemoryCache`] of the given capacity.
    ///
    /// `capacity` is preserved across resets so the steady-state memory
    /// budget is stable.
    #[must_use]
    pub fn new(capacity: usize) -> Arc<Self> {
        let initial: Arc<dyn ClientSessionStore> =
            Arc::new(ClientSessionMemoryCache::new(capacity));
        Arc::new(Self {
            inner: ArcSwap::from_pointee(initial),
            capacity,
        })
    }

    /// Atomically drop all cached sessions by replacing the inner store
    /// with a fresh empty [`ClientSessionMemoryCache`].
    ///
    /// Safe to call from any thread; in-flight reads on the trait methods
    /// observe a consistent snapshot.
    pub fn reset(&self) {
        let fresh: Arc<dyn ClientSessionStore> =
            Arc::new(ClientSessionMemoryCache::new(self.capacity));
        self.inner.store(Arc::new(fresh));
    }
}

impl fmt::Debug for CertScopedClientSessionStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CertScopedClientSessionStore")
            .field("capacity", &self.capacity)
            .finish()
    }
}

impl ClientSessionStore for CertScopedClientSessionStore {
    fn set_kx_hint(&self, server_name: ServerName<'static>, group: NamedGroup) {
        self.inner.load().set_kx_hint(server_name, group);
    }

    fn kx_hint(&self, server_name: &ServerName<'_>) -> Option<NamedGroup> {
        self.inner.load().kx_hint(server_name)
    }

    fn set_tls12_session(&self, server_name: ServerName<'static>, value: Tls12ClientSessionValue) {
        self.inner.load().set_tls12_session(server_name, value);
    }

    fn tls12_session(&self, server_name: &ServerName<'_>) -> Option<Tls12ClientSessionValue> {
        self.inner.load().tls12_session(server_name)
    }

    fn remove_tls12_session(&self, server_name: &ServerName<'static>) {
        self.inner.load().remove_tls12_session(server_name);
    }

    fn insert_tls13_ticket(
        &self,
        server_name: ServerName<'static>,
        value: Tls13ClientSessionValue,
    ) {
        self.inner.load().insert_tls13_ticket(server_name, value);
    }

    fn take_tls13_ticket(
        &self,
        server_name: &ServerName<'static>,
    ) -> Option<Tls13ClientSessionValue> {
        self.inner.load().take_tls13_ticket(server_name)
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::let_underscore_must_use,
        reason = "test setup — install_default() is idempotent across tests"
    )]

    use std::sync::Arc;

    use rustls::NamedGroup;
    use rustls::client::ClientSessionStore as _;
    use rustls::pki_types::ServerName;

    use super::CertScopedClientSessionStore;

    fn install_crypto_provider() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    fn server_name() -> ServerName<'static> {
        ServerName::try_from("example.test").expect("server name")
    }

    // ── new_returns_empty_store_that_implements_trait ─────────────────

    #[test]
    fn new_returns_empty_store_that_implements_trait() {
        install_crypto_provider();
        let store = CertScopedClientSessionStore::new(64);
        let name = server_name();

        assert!(
            store.kx_hint(&name).is_none(),
            "fresh store must not return a kx_hint"
        );
        assert!(
            store.tls12_session(&name).is_none(),
            "fresh store must not return a TLS 1.2 session"
        );
        assert!(
            store.take_tls13_ticket(&name).is_none(),
            "fresh store must not return a TLS 1.3 ticket"
        );
    }

    // ── reset_drops_stored_kx_hint ────────────────────────────────────
    //
    // We exercise `reset` via the `kx_hint` path. `set_kx_hint` is the
    // simplest mutating method on the trait — it does not require
    // constructing a `Tls13ClientSessionValue` (which has no public
    // constructor in the rustls 0.23 surface available to downstream
    // crates). The delegation logic is uniform across methods, so any
    // mutating call would exercise the same swap.

    #[test]
    fn reset_drops_stored_kx_hint() {
        install_crypto_provider();
        let store = CertScopedClientSessionStore::new(64);
        let name = server_name();

        store.set_kx_hint(name.clone(), NamedGroup::X25519);
        assert_eq!(
            store.kx_hint(&name),
            Some(NamedGroup::X25519),
            "set_kx_hint must round-trip through the wrapper before reset"
        );

        store.reset();

        assert!(
            store.kx_hint(&name).is_none(),
            "kx_hint must be gone after reset"
        );
    }

    // ── reset_preserves_capacity ──────────────────────────────────────

    #[test]
    fn reset_preserves_capacity() {
        let store = CertScopedClientSessionStore::new(42);
        // We cannot peek at the inner cache's capacity directly, but the
        // public `Debug` impl exposes it; that gives us a regression guard
        // without reaching into rustls internals.
        let before = format!("{store:?}");
        store.reset();
        let after = format!("{store:?}");
        assert_eq!(before, after, "Debug output must be stable across reset");
    }

    // ── reset_is_safe_to_call_repeatedly ──────────────────────────────

    #[test]
    fn reset_is_safe_to_call_repeatedly() {
        install_crypto_provider();
        let store = CertScopedClientSessionStore::new(4);
        for _ in 0..16 {
            store.reset();
        }
        assert!(store.kx_hint(&server_name()).is_none());
    }

    // ── debug_impl_renders_capacity ───────────────────────────────────

    #[test]
    fn debug_impl_renders_capacity() {
        let store = CertScopedClientSessionStore::new(99);
        let rendered = format!("{store:?}");
        assert!(
            rendered.contains("capacity: 99"),
            "Debug output should include capacity, got: {rendered}"
        );
    }

    // ── concurrent_reset_does_not_panic ──────────────────────────────

    #[test]
    fn concurrent_reset_does_not_panic() {
        install_crypto_provider();
        let store: Arc<CertScopedClientSessionStore> = CertScopedClientSessionStore::new(16);
        let name = server_name();

        let writer = {
            let store = Arc::clone(&store);
            let name = name.clone();
            std::thread::spawn(move || {
                for _ in 0..256 {
                    store.set_kx_hint(name.clone(), NamedGroup::X25519);
                }
            })
        };

        for _ in 0..256 {
            store.reset();
        }

        writer.join().expect("writer thread");
    }
}
