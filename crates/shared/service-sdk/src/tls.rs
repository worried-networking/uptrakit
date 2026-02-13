//! TLS configuration builders shared by agents and MQTT services.
//!
//! Provides both `TlsConnector` (for manual TCP→TLS→WS) and raw
//! `ClientConfig` (for MQTT's `Connector::Rustls`).

use std::sync::Arc;

use rootcause::prelude::*;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, pem::PemObject};
use tokio_rustls::TlsConnector;

use crate::error::{EnrollmentError, Result};

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
        bail!(EnrollmentError::NoCertificates);
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
