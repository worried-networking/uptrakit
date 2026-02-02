use std::sync::Arc;

use axum::extract::State;
use axum::{Extension, Json};
use http::StatusCode;

use crate::AppState;
use crate::auth::permissions::Permission;
use crate::middleware::require_auth::AuthenticatedUser;
use crate::pki_utils::{self, SanCollection};

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
) -> Result<Json<RenewServerCertResponse>, StatusCode> {
    if !user.has_permission(Permission::ManageGlobalSettings) {
        return Err(StatusCode::FORBIDDEN);
    }

    let snapshot = state.ca_snapshot.borrow().clone();

    // Build CA issuer from the active snapshot
    let ca_key = rcgen::KeyPair::from_pem(&snapshot.active_key_pem).map_err(|e| {
        tracing::error!(error = %e, "failed to parse CA key");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let ca_issuer =
        rcgen::Issuer::from_ca_cert_pem(&snapshot.active_cert_pem, ca_key).map_err(|e| {
            tracing::error!(error = %e, "failed to create CA issuer");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Generate new server cert
    let extra_sans: Vec<String> = state.settings.extra_sans().await;
    let sans: SanCollection = pki_utils::collect_sans(&extra_sans).map_err(|e| {
        tracing::error!(error = %e, "failed to collect SANs");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|e| {
        tracing::error!(error = %e, "failed to generate key pair");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut params = rcgen::CertificateParams::new(sans.dns_names.clone()).map_err(|e| {
        tracing::error!(error = %e, "failed to create cert params");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
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

    let cert = params.signed_by(&key_pair, &ca_issuer).map_err(|e| {
        tracing::error!(error = %e, "failed to sign server cert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();

    // Write to disk
    let cert_path = state.pki_path.join("server.crt");
    let key_path = state.pki_path.join("server.key");
    std::fs::write(&cert_path, &cert_pem).map_err(|e| {
        tracing::error!(error = %e, "failed to write server cert");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    std::fs::write(&key_path, &key_pem).map_err(|e| {
        tracing::error!(error = %e, "failed to write server key");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Hot-reload TLS config
    let server_config = build_server_tls_config(&cert_pem, &key_pem, &snapshot.bundle_pem)
        .map_err(|e| {
            tracing::error!(error = %e, "failed to build TLS config");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    state
        .rustls_config
        .reload_from_config(Arc::new(server_config));

    tracing::info!("server certificate manually renewed via API");

    Ok(Json(RenewServerCertResponse {
        message: "Server certificate renewed successfully".to_string(),
    }))
}

/// Minimal TLS config rebuild for hot-reload after cert renewal.
fn build_server_tls_config(
    cert_pem: &str,
    key_pem: &str,
    ca_bundle_pem: &str,
) -> Result<rustls::ServerConfig, String> {
    use rustls::RootCertStore;
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    use rustls::server::WebPkiClientVerifier;

    let certs: Vec<_> = CertificateDer::pem_slice_iter(cert_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cert PEM parse: {e}"))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes())
        .map_err(|e| format!("key PEM parse: {e}"))?;
    let ca_certs: Vec<_> = CertificateDer::pem_slice_iter(ca_bundle_pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("CA PEM parse: {e}"))?;

    let mut root_store = RootCertStore::empty();
    for ca_cert in ca_certs {
        root_store
            .add(ca_cert)
            .map_err(|e| format!("root store: {e}"))?;
    }

    let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
        .allow_unauthenticated()
        .build()
        .map_err(|e| format!("verifier: {e}"))?;

    rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)
        .map_err(|e| format!("server config: {e}"))
}
