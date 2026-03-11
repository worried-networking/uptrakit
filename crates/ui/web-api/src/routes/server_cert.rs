use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use http::StatusCode;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::pki_utils::{self, SanCollection};

#[derive(Debug, Error)]
pub(crate) enum RenewCertError {
    #[error("failed to parse CA key: {0}")]
    CaKeyParse(String),
    #[error("failed to create CA issuer: {0}")]
    CaIssuer(String),
    #[error("failed to generate key pair: {0}")]
    KeyGeneration(String),
    #[error("failed to create cert params: {0}")]
    CertParams(String),
    #[error("failed to sign server cert: {0}")]
    CertSign(String),
    #[error("failed to write server cert: {0}")]
    CertWrite(#[from] std::io::Error),
    #[error("TLS config error")]
    TlsConfig(#[from] TlsConfigError),
}

pub(crate) type RenewCertResult<T> = std::result::Result<T, Report<RenewCertError>>;

#[derive(Debug, Error)]
pub(crate) enum TlsConfigError {
    #[error("cert PEM parse: {0}")]
    CertParse(String),
    #[error("key PEM parse: {0}")]
    KeyParse(String),
    #[error("CA PEM parse: {0}")]
    CaParse(String),
    #[error("root store: {0}")]
    RootStore(String),
    #[error("verifier: {0}")]
    Verifier(String),
    #[error("server config: {0}")]
    ServerConfig(String),
}

pub(crate) type TlsConfigResult<T> = std::result::Result<T, rootcause::Report<TlsConfigError>>;

impl_report_conversion! {
    TlsConfigError => RenewCertError::TlsConfig,
    std::io::Error  => RenewCertError::CertWrite,
}

pub use uptrakit_web_api_types::server_cert::RenewServerCertResponse;

/// Renew the server TLS certificate using the current active CA.
#[utoipa::path(
    post,
    path = "/api/v1/settings/renew-server-certificate",
    tag = "Settings",
    responses(
        (status = 200, description = "Server certificate renewed", body = RenewServerCertResponse),
        (status = 403, description = "Not authorized"),
        (status = 500, description = "Renewal failed")
    ),
    extensions(("x-required-permission" = json!("manage_global_settings"))),
    security(("bearer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn renew_server_certificate(
    State(state): State<Arc<AppState>>,
    CanManageGlobalSettings(_user): CanManageGlobalSettings,
) -> Response {
    match renew_server_certificate_inner(&state).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "server certificate renewal failed");
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

pub(crate) async fn renew_server_certificate_inner(
    state: &AppState,
) -> RenewCertResult<RenewServerCertResponse> {
    let snapshot = state.ca_snapshot.borrow().clone();
    let key_store = state.ca_key_store.read().await;

    // Build CA issuer from the active snapshot and key store
    let ca_key = rcgen::KeyPair::from_pem(&key_store.active_key_pem)
        .map_err(|e| report!(RenewCertError::CaKeyParse(e.to_string())))?;
    let ca_issuer = rcgen::Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, ca_key)
        .map_err(|e| report!(RenewCertError::CaIssuer(e.to_string())))?;

    // Generate new server cert
    let san_list: Vec<String> = state.settings.sans();
    let sans: SanCollection = pki_utils::parse_san_list(&san_list);

    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|e| report!(RenewCertError::KeyGeneration(e.to_string())))?;

    let mut params = rcgen::CertificateParams::new(sans.dns_names.clone())
        .map_err(|e| report!(RenewCertError::CertParams(e.to_string())))?;
    for ip in &sans.ip_addrs {
        params
            .subject_alt_names
            .push(rcgen::SanType::IpAddress(*ip));
    }
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "Uptrakit Controller");
    params
        .distinguished_name
        .push(rcgen::DnType::OrganizationName, "Uptrakit");
    params.not_before = time::OffsetDateTime::now_utc();
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(90);

    let cert = params
        .signed_by(&key_pair, &ca_issuer)
        .map_err(|e| report!(RenewCertError::CertSign(e.to_string())))?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    // Write to disk with secure permissions (0o600) and atomic rename to
    // prevent world-readable keys and mismatched cert/key on crash.
    let cert_path = state.pki_path.join("server.crt");
    let key_path = state.pki_path.join("server.key");
    let cert_tmp = state.pki_path.join("server.crt.tmp");
    let key_tmp = state.pki_path.join("server.key.tmp");

    uptrakit_directories::write_secure_file_str(&key_tmp, &key_pem)
        .await
        .map_err(|e| {
            report!(RenewCertError::CertWrite(std::io::Error::other(
                e.to_string()
            )))
        })?;
    uptrakit_directories::write_secure_file_str(&cert_tmp, &cert_pem)
        .await
        .map_err(|e| {
            report!(RenewCertError::CertWrite(std::io::Error::other(
                e.to_string()
            )))
        })?;
    std::fs::rename(&key_tmp, &key_path).context_to::<RenewCertError>()?;
    std::fs::rename(&cert_tmp, &cert_path).context_to::<RenewCertError>()?;

    // Hot-reload TLS config
    let server_config =
        build_server_tls_config(&cert_pem, &key_pem, &snapshot.bundle_pem).context_to()?;
    state
        .rustls_config
        .reload_from_config(Arc::new(server_config));

    tracing::info!("server certificate manually renewed via API");

    Ok(RenewServerCertResponse {
        message: "Server certificate renewed successfully".to_string(),
    })
}

/// Minimal TLS config rebuild for hot-reload after cert renewal.
fn build_server_tls_config(
    cert_pem: &str,
    key_pem: &str,
    ca_bundle_pem: &str,
) -> TlsConfigResult<rustls::ServerConfig> {
    use rustls::RootCertStore;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| report!(TlsConfigError::CertParse(e.to_string())))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|e| report!(TlsConfigError::KeyParse(e.to_string())))?;
    let ca_certs: Vec<_> = CertificateDer::pem_slice_iter(ca_bundle_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| report!(TlsConfigError::CaParse(e.to_string())))?;

    let mut root_store = RootCertStore::empty();
    for ca_cert in ca_certs {
        root_store
            .add(ca_cert)
            .map_err(|e| report!(TlsConfigError::RootStore(e.to_string())))?;
    }

    let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .allow_unauthenticated()
        .build()
        .map_err(|e| report!(TlsConfigError::Verifier(e.to_string())))?;

    rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| report!(TlsConfigError::ServerConfig(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generate a self-signed CA certificate and its key pair for testing.
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

    /// Generate a server certificate signed by the given CA.
    fn generate_test_server_cert(ca_pem: &str, ca_key: &rcgen::KeyPair) -> (String, String) {
        let issuer = rcgen::Issuer::from_ca_cert_pem(ca_pem, ca_key).expect("issuer");
        let server_key =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let mut params =
            rcgen::CertificateParams::new(vec!["localhost".to_string()]).expect("params");
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "Test Server");
        let cert = params.signed_by(&server_key, &issuer).expect("sign");
        (cert.pem(), server_key.serialize_pem())
    }

    #[test]
    fn build_server_tls_config_valid_certs() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_pem, ca_key) = generate_test_ca();
        let (cert_pem, key_pem) = generate_test_server_cert(&ca_pem, &ca_key);

        let _config = build_server_tls_config(&cert_pem, &key_pem, &ca_pem)
            .expect("build_server_tls_config should succeed with valid certs");
    }

    #[test]
    fn build_server_tls_config_invalid_cert_pem() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_pem, ca_key) = generate_test_ca();
        let (_cert_pem, key_pem) = generate_test_server_cert(&ca_pem, &ca_key);

        let result = build_server_tls_config("bad cert", &key_pem, &ca_pem);

        assert!(result.is_err(), "should fail when cert PEM is invalid");
    }

    #[test]
    fn build_server_tls_config_invalid_key_pem() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_pem, ca_key) = generate_test_ca();
        let (cert_pem, _key_pem) = generate_test_server_cert(&ca_pem, &ca_key);

        let result = build_server_tls_config(&cert_pem, "bad key", &ca_pem);

        assert!(result.is_err(), "should fail when key PEM is invalid");
    }

    #[test]
    fn build_server_tls_config_invalid_ca_pem() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_pem, ca_key) = generate_test_ca();
        let (cert_pem, key_pem) = generate_test_server_cert(&ca_pem, &ca_key);

        let result = build_server_tls_config(&cert_pem, &key_pem, "bad ca");

        assert!(result.is_err(), "should fail when CA bundle PEM is invalid");
    }

    #[test]
    fn build_server_tls_config_multiple_ca_certs() {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

        let (ca_pem_1, ca_key_1) = generate_test_ca();
        let (ca_pem_2, _ca_key_2) = generate_test_ca();
        let (cert_pem, key_pem) = generate_test_server_cert(&ca_pem_1, &ca_key_1);

        // Concatenate two CA certificates into one bundle
        let ca_bundle = format!("{ca_pem_1}{ca_pem_2}");

        let _config = build_server_tls_config(&cert_pem, &key_pem, &ca_bundle)
            .expect("build_server_tls_config should succeed with multiple CA certs in bundle");
    }
}
