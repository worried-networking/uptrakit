use std::sync::Arc;

use axum::Json;
use axum::response::{IntoResponse, Response};
use axum::{Extension, extract::State};
use http::StatusCode;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::permission::CanManageGlobalSettings;
use crate::middleware::require_auth::{
    AuthenticatedApiTokenId, AuthenticatedUser, authenticated_user_audit_actor,
};
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

impl RenewCertError {
    fn reason_code(&self) -> &'static str {
        match self {
            Self::CaKeyParse(_) => "ca_key_parse_failed",
            Self::CaIssuer(_) => "ca_issuer_build_failed",
            Self::KeyGeneration(_) => "server_key_generation_failed",
            Self::CertParams(_) => "server_certificate_params_failed",
            Self::CertSign(_) => "server_certificate_sign_failed",
            Self::CertWrite(_) => "server_certificate_write_failed",
            Self::TlsConfig(_) => "server_tls_reload_failed",
        }
    }
}

fn emit_server_cert_renew_audit(
    state: &AppState,
    user: &AuthenticatedUser,
    api_token_id: Option<AuthenticatedApiTokenId>,
    outcome: uptrakit_audit_log::AuditOutcome,
    details: serde_json::Value,
) {
    let (actor_type, actor_id) = authenticated_user_audit_actor(user, api_token_id);

    if let Ok(entry) = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::SYSTEM_SERVER_CERTIFICATE_RENEW,
    )
    .system_scope()
    .actor(actor_type, actor_id)
    .target(
        "server_certificate",
        "controller_https".to_string(),
        Some("controller_https".to_string()),
    )
    .outcome(outcome)
    .details(details)
    .build()
    {
        state.audit_emitter.emit_best_effort(entry);
    }
}

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
    CanManageGlobalSettings(user): CanManageGlobalSettings,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    match renew_server_certificate_inner(&state).await {
        Ok(resp) => {
            emit_server_cert_renew_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "san_count": state.settings.sans().len(),
                }),
            );
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "server certificate renewal failed");
            emit_server_cert_renew_audit(
                &state,
                &user,
                api_token_id,
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "reason_code": e.current_context().reason_code(),
                }),
            );
            error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

pub(crate) async fn renew_server_certificate_inner(
    state: &AppState,
) -> RenewCertResult<RenewServerCertResponse> {
    let snapshot = state.cert.ca_snapshot.borrow().clone();
    let key_store = state.cert.ca_key_store.read().await;

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
    use crate::auth::AuthMethod;
    use crate::auth::permissions::Permission;
    use crate::middleware::permission::CanManageGlobalSettings;
    use crate::middleware::require_auth::AuthenticatedUser;
    use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
    use uptrakit_shared_db::entity::system_audit_log;

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

    async fn system_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    async fn wait_for_system_audit_rows(db: &sea_orm::DatabaseConnection, expected: u64) {
        for _ in 0..50 {
            let count = system_audit_log::Entity::find()
                .count(db)
                .await
                .expect("count system audit rows");
            if count == expected {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("expected {expected} system audit rows");
    }

    #[tokio::test]
    async fn renew_server_certificate_failure_writes_failed_system_audit_event() {
        let db = crate::test_harness::setup_migrated_db().await;
        let tenant_id = crate::test_harness::insert_default_tenant(&db).await;
        let (state, _jwt) = crate::test_harness::build_test_state(db.clone(), tenant_id).await;

        let user_id = uuid::Uuid::now_v7();
        let response = renew_server_certificate(
            State(Arc::clone(&state)),
            CanManageGlobalSettings::new(AuthenticatedUser {
                user_id,
                auth_method: AuthMethod::Password,
                permissions: vec![Permission::ManageGlobalSettings],
            }),
            None,
        )
        .await;

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        wait_for_system_audit_rows(&db, 1).await;
        let row = system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SYSTEM_SERVER_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            uptrakit_audit_log::AuditActionType::SYSTEM_SERVER_CERTIFICATE_RENEW,
            row.action_type
        );
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::User.as_str()
        );
        assert_eq!(row.actor_id, Some(user_id));
        let details = row.details_json.expect("details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("ca_key_parse_failed")
        );
    }
}
