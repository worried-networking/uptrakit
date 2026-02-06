use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use http::StatusCode;

use rootcause::ReportConversion;
use rootcause::prelude::*;
use thiserror::Error;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::pki_utils::{self, SanCollection};

#[derive(Debug, Error)]
enum RenewCertError {
    #[error("failed to parse CA key: {0}")]
    CaKeyParse(String),
    #[error("failed to create CA issuer: {0}")]
    CaIssuer(String),
    #[error("failed to collect SANs: {0}")]
    SanCollection(String),
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

type RenewCertResult<T> = std::result::Result<T, Report<RenewCertError>>;

#[derive(Debug, Error)]
enum TlsConfigError {
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

type TlsConfigResult<T> = std::result::Result<T, rootcause::Report<TlsConfigError>>;

impl<T> ReportConversion<TlsConfigError, markers::Mutable, T> for RenewCertError
where
    RenewCertError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<TlsConfigError, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(RenewCertError::TlsConfig)
    }
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
    security(("bearer_token" = []))
)]
pub async fn renew_server_certificate(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
) -> Response {
    if !user.has_permission(Permission::ManageGlobalSettings) {
        return StatusCode::FORBIDDEN.into_response();
    }

    match renew_server_certificate_inner(&state).await {
        Ok(resp) => (StatusCode::OK, Json(resp)).into_response(),
        Err(e) => {
            tracing::error!(error = %e, "server certificate renewal failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

async fn renew_server_certificate_inner(
    state: &AppState,
) -> RenewCertResult<RenewServerCertResponse> {
    let snapshot = state.ca_snapshot.borrow().clone();

    // Build CA issuer from the active snapshot
    let ca_key = rcgen::KeyPair::from_pem(&snapshot.active_key_pem)
        .map_err(|e| report!(RenewCertError::CaKeyParse(e.to_string())))?;
    let ca_issuer = rcgen::Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, ca_key)
        .map_err(|e| report!(RenewCertError::CaIssuer(e.to_string())))?;

    // Generate new server cert
    let extra_sans: Vec<String> = state.settings.extra_sans().await;
    let sans: SanCollection = pki_utils::collect_sans(&extra_sans)
        .map_err(|e| report!(RenewCertError::SanCollection(e.to_string())))?;

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

    // Write to disk
    let cert_path = state.pki_path.join("server.crt");
    let key_path = state.pki_path.join("server.key");
    std::fs::write(&cert_path, &cert_pem).map_err(|e| report!(RenewCertError::CertWrite(e)))?;
    std::fs::write(&key_path, &key_pem).map_err(|e| report!(RenewCertError::CertWrite(e)))?;

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
