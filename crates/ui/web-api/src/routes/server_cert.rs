use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use http::StatusCode;
use serde::Serialize;
use utoipa::ToSchema;

use crate::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct RenewServerCertResponse {
    pub message: String,
}

/// Renew the server TLS certificate using the current active CA.
#[utoipa::path(
    post,
    path = "/api/v1/settings/renew-server-certificate",
    tag = "Settings",
    responses(
        (status = 200, description = "Server certificate renewed", body = RenewServerCertResponse),
        (status = 500, description = "Renewal failed")
    ),
    security(("bearer_token" = []))
)]
pub async fn renew_server_certificate(
    State(state): State<Arc<AppState>>,
) -> Result<Json<RenewServerCertResponse>, StatusCode> {
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
    let extra_sans: Vec<String> = state.extra_sans.to_vec();
    let sans = collect_sans(&extra_sans).map_err(|e| {
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

struct SanCollection {
    dns_names: Vec<String>,
    ip_addrs: Vec<std::net::IpAddr>,
}

fn collect_sans(extra: &[String]) -> Result<SanCollection, String> {
    let mut dns_names = Vec::new();
    let mut ip_addrs = Vec::new();

    let hostname = hostname::get()
        .map_err(|e| format!("hostname: {e}"))?
        .to_string_lossy()
        .to_string();

    if !hostname.is_empty() {
        dns_names.push(hostname.clone());
    }

    if let Some(dot_pos) = hostname.find('.') {
        let short = &hostname[..dot_pos];
        if !short.is_empty() && short != hostname {
            dns_names.push(short.to_string());
        }
    }

    if !dns_names.iter().any(|n| n == "localhost") {
        dns_names.push("localhost".to_string());
    }

    for san in extra {
        if let Ok(ip) = san.parse::<std::net::IpAddr>() {
            if !ip_addrs.contains(&ip) {
                ip_addrs.push(ip);
            }
        } else if !dns_names.iter().any(|n| n == san) {
            dns_names.push(san.clone());
        }
    }

    dns_names.sort();
    dns_names.dedup();

    Ok(SanCollection {
        dns_names,
        ip_addrs,
    })
}
