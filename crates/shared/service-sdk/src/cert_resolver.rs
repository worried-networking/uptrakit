//! Hot-swappable client certificate resolver for mTLS connections.
//!
//! [`AgentClientCertResolver`] wraps an [`arc_swap::ArcSwap`] so that any
//! thread can atomically replace the active [`rustls::sign::CertifiedKey`]
//! while ongoing TLS handshakes continue to observe a consistent snapshot.
//!
//! Use [`AgentClientCertResolver::swap`] to rotate to a freshly-issued
//! certificate without tearing down existing connections.

use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::SignatureScheme;
use rustls::client::ResolvesClientCert;
use rustls::sign::CertifiedKey;

/// A TLS client-certificate resolver that supports atomic hot-swap.
///
/// Wrap an instance in `Arc` and pass it to
/// [`build_client_config_with_resolver`](crate::tls::build_client_config_with_resolver).
/// Call [`swap`](Self::swap) at any time to atomically replace the active
/// certificate without disrupting in-flight handshakes.
#[derive(Debug)]
pub struct AgentClientCertResolver {
    current: ArcSwap<CertifiedKey>,
}

impl AgentClientCertResolver {
    /// Create a new resolver with the given initial certificate.
    pub fn new(initial: Arc<CertifiedKey>) -> Self {
        Self {
            current: ArcSwap::new(initial),
        }
    }

    /// Atomically replace the active certificate.
    ///
    /// Ongoing TLS handshakes that already called [`resolve`](Self::resolve)
    /// keep their snapshot; the next handshake will use `next`.
    pub fn swap(&self, next: Arc<CertifiedKey>) {
        self.current.store(next);
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

    use rustls::client::ResolvesClientCert as _;
    use rustls::sign::CertifiedKey;

    use super::AgentClientCertResolver;

    /// Install the aws-lc-rs crypto provider (idempotent, safe to call multiple times).
    fn install_crypto_provider() {
        let _ignored = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    /// Generate a throwaway [`CertifiedKey`] using rcgen + aws-lc-rs.
    fn make_dummy_certified_key() -> Arc<CertifiedKey> {
        install_crypto_provider();

        let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
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

    // ── swap_visible_on_next_resolve ─────────────────────────────────

    #[test]
    fn swap_visible_on_next_resolve() {
        let key_a = make_dummy_certified_key();
        let key_b = make_dummy_certified_key();

        let resolver = AgentClientCertResolver::new(Arc::clone(&key_a));

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

    // ── concurrent_swap_and_resolve_never_returns_none ───────────────

    #[test]
    fn concurrent_swap_and_resolve_never_returns_none() {
        let initial = make_dummy_certified_key();
        let resolver = Arc::new(AgentClientCertResolver::new(Arc::clone(&initial)));

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
