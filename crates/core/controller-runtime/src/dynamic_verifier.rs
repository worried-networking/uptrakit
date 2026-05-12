use std::sync::Arc;

use parking_lot::RwLock;
use rustls::client::danger::HandshakeSignatureValid;
use rustls::pki_types::{CertificateDer, UnixTime};
use rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use rustls::{DigitallySignedStruct, DistinguishedName, SignatureScheme};

/// A [`ClientCertVerifier`] whose inner verifier can be atomically replaced at
/// runtime without stopping the server.
///
/// The inner `Arc<dyn ClientCertVerifier>` is held behind a `parking_lot::RwLock`
/// so that concurrent readers pay only a reader lock acquisition, and a caller can
/// `swap` in a fresh verifier after a CA rotation without blocking ongoing TLS
/// handshakes for more than the duration of the lock.
#[derive(Debug)]
pub(crate) struct DynamicClientVerifier {
    inner: RwLock<Arc<dyn ClientCertVerifier>>,
    empty_subjects: Vec<DistinguishedName>,
}

impl DynamicClientVerifier {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired up in a subsequent mTLS hot-swap task")
    )]
    pub(crate) fn new(initial: Arc<dyn ClientCertVerifier>) -> Self {
        Self {
            inner: RwLock::new(initial),
            empty_subjects: Vec::new(),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired up in a subsequent mTLS hot-swap task")
    )]
    pub(crate) fn swap(&self, next: Arc<dyn ClientCertVerifier>) {
        *self.inner.write() = next;
    }
}

impl ClientCertVerifier for DynamicClientVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }

    fn client_auth_mandatory(&self) -> bool {
        false
    }

    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &self.empty_subjects
    }

    fn verify_client_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        now: UnixTime,
    ) -> Result<ClientCertVerified, rustls::Error> {
        let inner = Arc::clone(&*self.inner.read());
        inner.verify_client_cert(end_entity, intermediates, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let inner = Arc::clone(&*self.inner.read());
        inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        let inner = Arc::clone(&*self.inner.read());
        inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        let inner = Arc::clone(&*self.inner.read());
        inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use rustls::server::WebPkiClientVerifier;

    use super::*;

    /// Build a [`WebPkiClientVerifier`] that trusts exactly the provided root
    /// certificates, returning it as `Arc<dyn ClientCertVerifier>` (the type
    /// produced by the builder).
    fn build_verifier_from_roots(roots: &[CertificateDer<'static>]) -> Arc<dyn ClientCertVerifier> {
        let mut root_store = rustls::RootCertStore::empty();
        for cert in roots {
            root_store.add(cert.clone()).expect("add root");
        }
        WebPkiClientVerifier::builder(Arc::new(root_store))
            .allow_unauthenticated()
            .build()
            .expect("verifier builds")
    }

    /// Returns (ca_a_der, ca_b_der, leaf_signed_by_b_der).
    fn build_two_root_fixtures() -> (
        CertificateDer<'static>,
        CertificateDer<'static>,
        CertificateDer<'static>,
    ) {
        fn make_ca(name: &str) -> (rcgen::Certificate, rcgen::KeyPair) {
            let key = rcgen::KeyPair::generate().expect("kp");
            let mut params = rcgen::CertificateParams::new(vec![]).expect("params");
            params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
            params
                .distinguished_name
                .push(rcgen::DnType::CommonName, name);
            let cert = params.self_signed(&key).expect("cert");
            (cert, key)
        }

        let (ca_a, _key_a) = make_ca("test-ca-a");
        let (ca_b, key_b) = make_ca("test-ca-b");

        let leaf_key = rcgen::KeyPair::generate().expect("leaf kp");
        let mut leaf_params =
            rcgen::CertificateParams::new(vec!["leaf.local".into()]).expect("leaf params");
        leaf_params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "leaf");
        let issuer_b =
            rcgen::Issuer::from_ca_cert_pem(&ca_b.pem(), key_b).expect("issuer from CA-B PEM");
        let leaf = leaf_params
            .signed_by(&leaf_key, &issuer_b)
            .expect("leaf cert");

        (
            CertificateDer::from(ca_a.der().to_vec()),
            CertificateDer::from(ca_b.der().to_vec()),
            CertificateDer::from(leaf.der().to_vec()),
        )
    }

    #[test]
    fn root_hint_subjects_is_empty() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_a_der, _ca_b_der, _leaf_der) = build_two_root_fixtures();
        let verifier = build_verifier_from_roots(&[ca_a_der]);
        let dynamic = DynamicClientVerifier::new(verifier);
        assert!(
            dynamic.root_hint_subjects().is_empty(),
            "root_hint_subjects should always be empty"
        );
    }

    #[test]
    fn swap_visible_on_next_verify_call() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_a_der, ca_b_der, leaf_der) = build_two_root_fixtures();

        // Verifier-A trusts only CA-A; verifier-B trusts only CA-B.
        let verifier_a = build_verifier_from_roots(&[ca_a_der]);
        let verifier_b = build_verifier_from_roots(&[ca_b_der]);

        let dynamic = DynamicClientVerifier::new(verifier_a);

        let now = UnixTime::now();

        // Before swap: leaf signed by CA-B should fail against verifier-A.
        let result_before = dynamic.verify_client_cert(&leaf_der, &[], now);
        assert!(
            result_before.is_err(),
            "leaf signed by CA-B must not verify against CA-A verifier"
        );

        // Swap to verifier-B which trusts CA-B.
        dynamic.swap(verifier_b);

        // After swap: same leaf should pass against verifier-B.
        let result_after = dynamic.verify_client_cert(&leaf_der, &[], now);
        assert!(
            result_after.is_ok(),
            "leaf signed by CA-B must verify against CA-B verifier after swap, got: {result_after:?}"
        );
    }

    #[test]
    fn concurrent_swap_and_verify_never_panics() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_a_der, ca_b_der, _leaf_der) = build_two_root_fixtures();

        let dynamic = Arc::new(DynamicClientVerifier::new(build_verifier_from_roots(&[
            ca_a_der.clone(),
        ])));

        let verifier_a = build_verifier_from_roots(&[ca_a_der]);
        let verifier_b = build_verifier_from_roots(&[ca_b_der]);

        let mut handles = Vec::new();

        // Spawn 4 reader threads, each calling supported_verify_schemes() and
        // root_hint_subjects() 200 times.
        for _ in 0..4 {
            let d = Arc::clone(&dynamic);
            handles.push(std::thread::spawn(move || {
                for _ in 0..200 {
                    let _ = d.supported_verify_schemes();
                    let _ = d.root_hint_subjects();
                }
            }));
        }

        // Swap from main thread 100 times, alternating between the two verifiers.
        for i in 0..100 {
            if i % 2 == 0 {
                dynamic.swap(verifier_b.clone());
            } else {
                dynamic.swap(verifier_a.clone());
            }
        }

        for h in handles {
            h.join().expect("reader thread must not panic");
        }
    }
}
