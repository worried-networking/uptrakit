use std::sync::Arc;

use arc_swap::ArcSwap;
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;

#[derive(Debug)]
pub(crate) struct ControllerServerCertResolver {
    current: ArcSwap<CertifiedKey>,
}

impl ControllerServerCertResolver {
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired up in a subsequent mTLS hot-swap task")
    )]
    pub(crate) fn new(initial: Arc<CertifiedKey>) -> Self {
        Self {
            current: ArcSwap::new(initial),
        }
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "wired up in a subsequent mTLS hot-swap task")
    )]
    pub(crate) fn swap(&self, next: Arc<CertifiedKey>) {
        self.current.store(next);
    }
}

impl ResolvesServerCert for ControllerServerCertResolver {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.current.load_full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_certified_key(name: &str) -> Arc<CertifiedKey> {
        let key = rcgen::KeyPair::generate().expect("kp");
        let mut params = rcgen::CertificateParams::new(vec![name.into()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, name);
        let cert = params.self_signed(&key).expect("cert");
        let cert_der = rustls::pki_types::CertificateDer::from(cert.der().to_vec());
        let key_der = rustls::pki_types::PrivateKeyDer::Pkcs8(
            rustls::pki_types::PrivatePkcs8KeyDer::from(key.serialize_der()),
        );
        let signing_key =
            rustls::crypto::aws_lc_rs::sign::any_supported_type(&key_der).expect("signing key");
        Arc::new(CertifiedKey::new(vec![cert_der], signing_key))
    }

    #[test]
    fn swap_visible_on_next_resolve() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let initial = test_certified_key("initial.local");
        let next = test_certified_key("next.local");

        let resolver = ControllerServerCertResolver::new(Arc::clone(&initial));

        // Before swap: current should be the initial key.
        let loaded = resolver.current.load_full();
        assert!(
            Arc::ptr_eq(&loaded, &initial),
            "before swap: resolver holds the initial CertifiedKey"
        );

        resolver.swap(Arc::clone(&next));

        // After swap: current should be the new key.
        let loaded = resolver.current.load_full();
        assert!(
            Arc::ptr_eq(&loaded, &next),
            "after swap: resolver holds the new CertifiedKey"
        );
        assert!(
            !Arc::ptr_eq(&loaded, &initial),
            "after swap: resolver no longer holds the initial CertifiedKey"
        );
    }
}
