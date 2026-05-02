//! TLS configuration builders shared by agents and MQTT services.
//!
//! Provides both `TlsConnector` (for manual TCP→TLS→WS) and raw
//! `ClientConfig` (for MQTT's `Connector::Rustls`).

use std::sync::Arc;

use rootcause::prelude::*;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, pem::PemObject};
use tokio_rustls::TlsConnector;

use crate::error::{EnrollmentError, Result, TlsError};

// ── TlsConnector builders (agent-style manual TCP→TLS→WS) ───────────

/// Build a TLS connector that trusts only the given CA PEM (no client auth).
pub fn build_tls_connector(ca_pem: &[u8]) -> Result<TlsConnector> {
    let config = build_pinned_ca_client_config(ca_pem)?;
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Build a TLS connector that trusts only the given CA PEM, with client cert (mTLS).
pub fn build_tls_connector_with_client_cert(
    ca_pem: &[u8],
    cert_pem: &str,
    key_pem: &str,
) -> Result<TlsConnector> {
    let config = build_mtls_client_config(ca_pem, cert_pem, key_pem)?;
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Build a TLS connector using system/webpki root certificates (no client auth).
pub fn build_system_trust_tls_connector() -> Result<TlsConnector> {
    let config = build_system_roots_client_config()?;
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Build a TLS connector using system/webpki root certs with client cert (mTLS).
pub fn build_system_trust_tls_connector_with_client_cert(
    cert_pem: &str,
    key_pem: &str,
) -> Result<TlsConnector> {
    use rustls::pki_types::PrivateKeyDer;

    let root_store = build_webpki_root_store();

    let client_certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_to::<EnrollmentError>()?;

    let client_key =
        PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).context_to::<EnrollmentError>()?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_certs, client_key)
        .context_to::<EnrollmentError>()?;

    Ok(TlsConnector::from(Arc::new(config)))
}

// ── ClientConfig builders (for MQTT's Connector::Rustls) ─────────────

/// Build a `ClientConfig` trusting only the given CA PEM (no client auth).
pub fn build_pinned_ca_client_config(ca_pem: &[u8]) -> Result<rustls::ClientConfig> {
    let root_store = build_root_store(ca_pem)?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

/// Build a `ClientConfig` for mTLS with pinned CA and client certificate.
pub fn build_mtls_client_config(
    ca_pem: &[u8],
    cert_pem: &str,
    key_pem: &str,
) -> Result<rustls::ClientConfig> {
    use rustls::pki_types::PrivateKeyDer;

    let root_store = build_root_store(ca_pem)?;

    let client_certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_to::<EnrollmentError>()?;

    let client_key =
        PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).context_to::<EnrollmentError>()?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(client_certs, client_key)
        .context_to::<EnrollmentError>()?;

    Ok(config)
}

/// Build a `ClientConfig` using system/webpki root certificates (no client auth).
pub fn build_system_roots_client_config() -> Result<rustls::ClientConfig> {
    let root_store = build_webpki_root_store();

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

/// Build a `ClientConfig` for TOFU CA fetch.
///
/// Accepts any server certificate (the CA is not known yet) but still
/// delegates TLS signature verification to the installed crypto provider,
/// preventing trivial MITM attacks that forge invalid signatures.
pub fn build_tofu_client_config() -> Result<rustls::ClientConfig> {
    #[derive(Debug)]
    struct TofuVerifier;

    impl rustls::client::danger::ServerCertVerifier for TofuVerifier {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error>
        {
            // During TOFU, we accept any cert chain since the CA is unknown.
            // Security relies on fingerprint verification after download.
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            let provider = rustls::crypto::CryptoProvider::get_default().ok_or(
                rustls::Error::General("no crypto provider installed".into()),
            )?;
            rustls::crypto::verify_tls12_signature(
                message,
                cert,
                dss,
                &provider.signature_verification_algorithms,
            )
        }

        fn verify_tls13_signature(
            &self,
            message: &[u8],
            cert: &rustls::pki_types::CertificateDer<'_>,
            dss: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            let provider = rustls::crypto::CryptoProvider::get_default().ok_or(
                rustls::Error::General("no crypto provider installed".into()),
            )?;
            rustls::crypto::verify_tls13_signature(
                message,
                cert,
                dss,
                &provider.signature_verification_algorithms,
            )
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::CryptoProvider::get_default()
                .map(|p| p.signature_verification_algorithms.supported_schemes())
                .unwrap_or_default()
        }
    }

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TofuVerifier))
        .with_no_client_auth();

    Ok(config)
}

/// Convenience wrapper: wrap a `ClientConfig` into a `TlsConnector`.
pub fn tls_connector(config: rustls::ClientConfig) -> TlsConnector {
    TlsConnector::from(Arc::new(config))
}

// ── Internal helpers ─────────────────────────────────────────────────

fn build_root_store(ca_pem: &[u8]) -> Result<RootCertStore> {
    let certs = CertificateDer::pem_slice_iter(ca_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_to::<EnrollmentError>()?;

    if certs.is_empty() {
        bail!(EnrollmentError::Tls(TlsError::NoCertificates));
    }

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store.add(cert).context_to::<EnrollmentError>()?;
    }

    Ok(root_store)
}

fn build_webpki_root_store() -> RootCertStore {
    let mut root_store = RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    root_store
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        clippy::let_underscore_must_use,
        reason = "test assertions — assert!(result.is_err()) and let _ = install_default() (idempotent) are idiomatic in tests"
    )]

    use super::*;

    /// Install the aws-lc-rs crypto provider (idempotent, safe to call multiple times).
    fn install_crypto_provider() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    }

    /// Generate a self-signed CA certificate + key pair for testing.
    fn generate_test_ca() -> (String, rcgen::KeyPair) {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test CA");
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
        let cert = params.self_signed(&key_pair).expect("self-sign");
        (cert.pem(), key_pair)
    }

    /// Generate a client certificate signed by the given CA.
    fn generate_test_client_cert(ca_pem: &str, ca_key: &rcgen::KeyPair) -> (String, String) {
        let issuer = rcgen::Issuer::from_ca_cert_pem(ca_pem, ca_key).expect("issuer");
        let client_key =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params =
            rcgen::CertificateParams::new(vec!["client.test".to_string()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test Client");
        let cert = params.signed_by(&client_key, &issuer).expect("sign");
        (cert.pem(), client_key.serialize_pem())
    }

    // ── build_root_store ─────────────────────────────────────────────

    #[test]
    fn build_root_store_valid_ca() {
        let (ca_pem, _) = generate_test_ca();
        let store = build_root_store(ca_pem.as_bytes()).expect("build");
        assert_eq!(store.len(), 1);
    }

    #[test]
    fn build_root_store_empty_pem_returns_error() {
        let result = build_root_store(b"");
        assert!(result.is_err());
    }

    #[test]
    fn build_root_store_invalid_pem_returns_error() {
        let result = build_root_store(b"not a PEM");
        assert!(result.is_err());
    }

    // ── build_pinned_ca_client_config ────────────────────────────────

    #[test]
    fn pinned_ca_client_config_succeeds() {
        install_crypto_provider();
        let (ca_pem, _) = generate_test_ca();
        let config = build_pinned_ca_client_config(ca_pem.as_bytes()).expect("config");
        // No client auth configured — has_certs is false
        assert!(!config.client_auth_cert_resolver.has_certs());
    }

    #[test]
    fn pinned_ca_client_config_invalid_ca_fails() {
        let result = build_pinned_ca_client_config(b"garbage");
        assert!(result.is_err());
    }

    // ── build_mtls_client_config ─────────────────────────────────────

    #[test]
    fn mtls_client_config_succeeds() {
        install_crypto_provider();
        let (ca_pem, ca_key) = generate_test_ca();
        let (cert_pem, key_pem) = generate_test_client_cert(&ca_pem, &ca_key);
        let config =
            build_mtls_client_config(ca_pem.as_bytes(), &cert_pem, &key_pem).expect("config");
        let _ = config;
    }

    #[test]
    fn mtls_client_config_invalid_cert_fails() {
        let (ca_pem, _) = generate_test_ca();
        let result = build_mtls_client_config(ca_pem.as_bytes(), "bad cert", "bad key");
        assert!(result.is_err());
    }

    // ── build_system_roots_client_config ─────────────────────────────

    #[test]
    fn system_roots_client_config_succeeds() {
        install_crypto_provider();
        let config = build_system_roots_client_config().expect("config");
        let _ = config;
    }

    // ── build_tofu_client_config ─────────────────────────────────────

    #[test]
    fn tofu_client_config_succeeds() {
        install_crypto_provider();
        let config = build_tofu_client_config().expect("config");
        let _ = config;
    }

    // ── build_tls_connector ──────────────────────────────────────────

    #[test]
    fn tls_connector_from_pinned_ca() {
        install_crypto_provider();
        let (ca_pem, _) = generate_test_ca();
        let connector = build_tls_connector(ca_pem.as_bytes()).expect("connector");
        let _ = connector;
    }

    #[test]
    fn tls_connector_invalid_ca_fails() {
        let result = build_tls_connector(b"not valid");
        assert!(result.is_err());
    }

    // ── build_tls_connector_with_client_cert ─────────────────────────

    #[test]
    fn tls_connector_with_client_cert_succeeds() {
        install_crypto_provider();
        let (ca_pem, ca_key) = generate_test_ca();
        let (cert_pem, key_pem) = generate_test_client_cert(&ca_pem, &ca_key);
        let connector =
            build_tls_connector_with_client_cert(ca_pem.as_bytes(), &cert_pem, &key_pem)
                .expect("connector");
        let _ = connector;
    }

    // ── build_system_trust_tls_connector ─────────────────────────────

    #[test]
    fn system_trust_tls_connector_succeeds() {
        install_crypto_provider();
        let connector = build_system_trust_tls_connector().expect("connector");
        let _ = connector;
    }

    // ── build_system_trust_tls_connector_with_client_cert ────────────

    #[test]
    fn system_trust_tls_connector_with_client_cert_succeeds() {
        install_crypto_provider();
        let (ca_pem, ca_key) = generate_test_ca();
        let (cert_pem, key_pem) = generate_test_client_cert(&ca_pem, &ca_key);
        let connector = build_system_trust_tls_connector_with_client_cert(&cert_pem, &key_pem)
            .expect("connector");
        let _ = connector;
    }

    // ── tls_connector convenience wrapper ────────────────────────────

    #[test]
    fn tls_connector_wrapper() {
        install_crypto_provider();
        let config = build_system_roots_client_config().expect("config");
        let connector = tls_connector(config);
        let _ = connector;
    }

    // ── build_webpki_root_store ──────────────────────────────────────

    #[test]
    fn webpki_root_store_is_non_empty() {
        let store = build_webpki_root_store();
        assert!(!store.is_empty());
    }

    // ── Multiple CA certs ────────────────────────────────────────────

    #[test]
    fn root_store_accepts_multiple_ca_certs() {
        let (ca1_pem, _) = generate_test_ca();
        let (ca2_pem, _) = generate_test_ca();
        let combined = format!("{ca1_pem}{ca2_pem}");
        let store = build_root_store(combined.as_bytes()).expect("build");
        assert_eq!(store.len(), 2);
    }
}
