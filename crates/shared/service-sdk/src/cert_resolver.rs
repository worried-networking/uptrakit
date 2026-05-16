//! Hot-swappable client certificate resolver for mTLS connections.
//!
//! [`AgentClientCertResolver`] wraps an [`arc_swap::ArcSwap`] so that any
//! thread can atomically replace the active [`rustls::sign::CertifiedKey`]
//! while ongoing TLS handshakes continue to observe a consistent snapshot.
//!
//! Use [`AgentClientCertResolver::swap`] to rotate to a freshly-issued
//! certificate without tearing down existing connections.
//!
//! The resolver also owns the matching
//! [`CertScopedClientSessionStore`](crate::session_store::CertScopedClientSessionStore)
//! so that every rotation flushes cached TLS 1.3 tickets in lockstep with
//! the cert swap. Without that coupling, resumed sessions would keep the
//! pre-rotation cert visible to the server (see `session_store` module
//! docs for the protocol-level reason).

use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::SignatureScheme;
use rustls::client::ResolvesClientCert;
use rustls::sign::CertifiedKey;

use crate::session_store::CertScopedClientSessionStore;

/// A TLS client-certificate resolver that supports atomic hot-swap.
///
/// Wrap an instance in `Arc` and pass it to
/// [`build_client_config_with_resolver`](crate::tls::build_client_config_with_resolver).
/// Call [`swap`](Self::swap) at any time to atomically replace the active
/// certificate without disrupting in-flight handshakes.
#[derive(Debug)]
#[non_exhaustive]
pub struct AgentClientCertResolver {
    current: ArcSwap<CertifiedKey>,
    session_store: Arc<CertScopedClientSessionStore>,
}

impl AgentClientCertResolver {
    /// Create a new resolver with the given initial certificate and the
    /// session store that must be flushed on every rotation.
    ///
    /// The same `session_store` instance must also be registered with the
    /// `ClientConfig` that wraps this resolver (see
    /// [`build_client_config_with_resolver`](crate::tls::build_client_config_with_resolver));
    /// otherwise [`swap`](Self::swap) will reset an orphan cache while
    /// rustls keeps replaying the old tickets.
    pub fn new(
        initial: Arc<CertifiedKey>,
        session_store: Arc<CertScopedClientSessionStore>,
    ) -> Self {
        Self {
            current: ArcSwap::new(initial),
            session_store,
        }
    }

    /// Atomically replace the active certificate, then flush all cached
    /// TLS sessions.
    ///
    /// Ongoing TLS handshakes that already called [`resolve`](Self::resolve)
    /// keep their snapshot; the next handshake will use `next` and start
    /// from an empty session cache, forcing a full handshake (which is
    /// what we want — TLS 1.3 PSK resumption would otherwise keep
    /// presenting the pre-rotation cert).
    ///
    /// Invariant: `swap` is invoked from `handle_certificate` after the
    /// WS `Certificate` frame is processed and right before the
    /// connection closes — there is no concurrent reconnect attempt in
    /// flight against this connector. A future refactor that shares the
    /// resolver across concurrent connectors must revisit this
    /// publish-then-reset ordering.
    pub fn swap(&self, next: Arc<CertifiedKey>) {
        self.current.store(next);
        self.session_store.reset();
        tracing::debug!("client TLS session cache reset on cert rotation");
    }

    /// Borrow the session store that this resolver flushes on rotation.
    ///
    /// Callers that build the matching `ClientConfig` need the same
    /// `Arc` instance so the resumption store wired into rustls is the
    /// one that `swap` will reset.
    #[must_use]
    pub fn session_store(&self) -> &Arc<CertScopedClientSessionStore> {
        &self.session_store
    }
}

impl ResolvesClientCert for AgentClientCertResolver {
    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[SignatureScheme],
    ) -> Option<Arc<CertifiedKey>> {
        Some(self.current.load_full())
    }

    fn has_certs(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use rustls::NamedGroup;
    use rustls::client::ClientSessionStore as _;
    use rustls::client::ResolvesClientCert as _;
    use rustls::pki_types::ServerName;
    use rustls::sign::CertifiedKey;

    use super::AgentClientCertResolver;
    use crate::session_store::CertScopedClientSessionStore;

    /// Install the aws-lc-rs crypto provider (idempotent, safe to call multiple times).
    fn install_crypto_provider() {
        let _ignored = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    /// Generate a throwaway [`CertifiedKey`] using rcgen + aws-lc-rs.
    fn make_dummy_certified_key() -> Arc<CertifiedKey> {
        install_crypto_provider();

        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P384_SHA384).expect("keygen");
        let mut params =
            rcgen::CertificateParams::new(vec!["test.local".to_string()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "test");
        let cert = params.self_signed(&key).expect("self-sign");

        let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
        );
        let signing_key =
            rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der).expect("signing key");

        Arc::new(CertifiedKey::new(vec![cert_der], signing_key))
    }

    /// Build a resolver paired with a fresh session store for tests.
    fn make_test_resolver(
        key: Arc<CertifiedKey>,
    ) -> (AgentClientCertResolver, Arc<CertScopedClientSessionStore>) {
        let store = CertScopedClientSessionStore::new(64);
        let resolver = AgentClientCertResolver::new(key, Arc::clone(&store));
        (resolver, store)
    }

    // ── swap_visible_on_next_resolve ─────────────────────────────────

    #[test]
    fn swap_visible_on_next_resolve() {
        let key_a = make_dummy_certified_key();
        let key_b = make_dummy_certified_key();

        let (resolver, _store) = make_test_resolver(Arc::clone(&key_a));

        // Initial resolve returns key_a.
        let resolved_a = resolver.resolve(&[], &[]).expect("initial resolve");
        assert!(
            Arc::ptr_eq(&resolved_a, &key_a),
            "initial resolve should return key_a"
        );

        // After swap, resolve returns key_b.
        resolver.swap(Arc::clone(&key_b));
        let resolved_b = resolver.resolve(&[], &[]).expect("post-swap resolve");
        assert!(
            Arc::ptr_eq(&resolved_b, &key_b),
            "post-swap resolve should return key_b"
        );
    }

    // ── swap_resets_attached_session_store ───────────────────────────

    #[test]
    fn swap_resets_attached_session_store() {
        let key_a = make_dummy_certified_key();
        let key_b = make_dummy_certified_key();
        let (resolver, store) = make_test_resolver(Arc::clone(&key_a));

        let name = ServerName::try_from("example.test").expect("server name");
        store.set_kx_hint(name.clone(), NamedGroup::X25519);
        assert_eq!(
            store.kx_hint(&name),
            Some(NamedGroup::X25519),
            "pre-swap state must be observable"
        );

        resolver.swap(Arc::clone(&key_b));

        assert!(
            store.kx_hint(&name).is_none(),
            "swap must flush the session store"
        );
    }

    // ── concurrent_swap_and_resolve_never_returns_none ───────────────

    #[test]
    fn concurrent_swap_and_resolve_never_returns_none() {
        let initial = make_dummy_certified_key();
        let (resolver, _store) = make_test_resolver(Arc::clone(&initial));
        let resolver = Arc::new(resolver);

        const RESOLVE_THREADS: usize = 4;
        const RESOLVES_PER_THREAD: usize = 1_000;
        const SWAP_COUNT: usize = 100;

        let mut handles = Vec::new();

        // Spawn resolver threads.
        for _ in 0..RESOLVE_THREADS {
            let r = Arc::clone(&resolver);
            handles.push(thread::spawn(move || {
                for _ in 0..RESOLVES_PER_THREAD {
                    assert!(
                        r.resolve(&[], &[]).is_some(),
                        "resolve must never return None"
                    );
                }
            }));
        }

        // Perform swaps on the current thread concurrently with the resolvers.
        for _ in 0..SWAP_COUNT {
            let next = make_dummy_certified_key();
            resolver.swap(next);
        }

        for handle in handles {
            handle.join().expect("resolver thread panicked");
        }
    }
}
