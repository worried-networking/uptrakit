use std::net::{SocketAddr, TcpListener};
use std::sync::Arc;

use axum::Router;
use axum::extract::Request;
use axum::middleware as axum_mw;
use axum::response::Json;
use axum::routing::get;
use ipnet::IpNet;
use sea_orm::{ConnectOptions, Database};
use serde::{Deserialize, Serialize};

use uptrakit_web_api::AppState;
use uptrakit_web_api::auth::device_flow::DeviceFlowStore;
use uptrakit_web_api::auth::jwt::JwtManager;
#[cfg(feature = "oidc")]
use uptrakit_web_api::auth::oidc_state::{
    AccountLinkStore, OidcFlowStore, OidcRegistrationStore, OidcTokenExchangeStore,
};
use uptrakit_web_api::auth::rate_limit::RateLimitStore;
use uptrakit_web_api::auth::registration::{RegistrationMode, RegistrationSettings};
use uptrakit_web_api::ca_snapshot::CaPublicSnapshot;
use uptrakit_web_api::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
use uptrakit_web_api::extract::ServiceIdentity;
use uptrakit_web_api::middleware;
use uptrakit_web_api::settings::Settings;

use super::pki::TestPki;

/// A lightweight test HTTPS server that runs the real `resolve_ip` and
/// `resolve_proxy_headers` middleware with minimal setup (in-memory SQLite,
/// no DB migrations, no auth).
pub struct TestServer {
    /// The local port the server is listening on.
    pub port: u16,
    handle: axum_server::Handle<SocketAddr>,
}

impl TestServer {
    /// Start a test server configured for a specific reverse proxy.
    ///
    /// - `info_header`: the forwarded client cert info header name (or `None`)
    /// - `pem_header`: the forwarded client cert PEM header name (or `None`)
    pub async fn start(pki: &TestPki, info_header: Option<&str>, pem_header: Option<&str>) -> Self {
        let state = build_state(pki, info_header, pem_header).await;
        let router = build_router(state);

        let listener = TcpListener::bind("0.0.0.0:0").expect("bind to random port");
        listener
            .set_nonblocking(true)
            .expect("set listener nonblocking");
        let port = listener.local_addr().expect("local addr").port();

        let rustls_config = build_rustls_config(pki);

        let handle = axum_server::Handle::new();
        let server_handle = handle.clone();

        tokio::spawn(async move {
            axum_server::from_tcp_rustls(listener, rustls_config)
                .expect("from_tcp_rustls")
                .handle(server_handle)
                .serve(router.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .expect("server error");
        });

        // Wait until the server is actually listening.
        handle.listening().await;

        Self { port, handle }
    }

    /// Gracefully shut down the server.
    pub fn shutdown(&self) {
        self.handle.graceful_shutdown(None);
    }
}

/// Response type for `/test/identity`.
#[derive(Serialize, Deserialize, Debug)]
pub struct IdentityResponse {
    pub agent_id: Option<String>,
    pub cert_serial: Option<String>,
}

struct NoopCertSigner;

#[async_trait::async_trait]
impl AgentCertSigner for NoopCertSigner {
    async fn sign_agent_csr(
        &self,
        _: &str,
        _: &uuid::Uuid,
        _: ::time::Duration,
    ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>> {
        Err(rootcause::Report::new(CertSignerError::Signing(
            "noop signer".to_string(),
        )))
    }

    fn active_ca_fingerprint(&self) -> String {
        "0".repeat(64)
    }
}

async fn build_state(
    pki: &TestPki,
    info_header: Option<&str>,
    pem_header: Option<&str>,
) -> Arc<AppState> {
    let snapshot_data = CaPublicSnapshot {
        active_cert_pem: pki.ca_cert_pem.clone(),
        active_fingerprint: "0".repeat(64),
        previous_cert_pem: None,
        previous_fingerprint: None,
        trusted_cas: vec![uptrakit_web_api::ca_snapshot::TrustedCaPublic {
            cert_pem: pki.ca_cert_pem.clone(),
            fingerprint: "0".repeat(64),
            not_after: ::time::OffsetDateTime::now_utc() + ::time::Duration::days(365),
        }],
        trusted_ca_cns: vec!["Test CA".to_string()],
        bundle_pem: pki.ca_cert_pem.clone(),
        bundle_hash: "0".repeat(64),
        managed: true,
        active_not_after: ::time::OffsetDateTime::now_utc() + ::time::Duration::days(365),
        pki_addr: None,
    };
    let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
    let ca_key_store: uptrakit_web_api::CaKeyStoreRef = Arc::new(tokio::sync::RwLock::new(
        uptrakit_web_api::ca_snapshot::CaKeyStore {
            active_key_pem: zeroize::Zeroizing::new(String::new()),
            previous_key_pem: None,
            trusted_ca_keys: vec![],
        },
    ));

    let rustls_cfg = {
        let key_pair =
            rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("keygen");
        let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
            .expect("cert params")
            .self_signed(&key_pair)
            .expect("self-sign");
        let server_config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der())
                    .expect("key DER"),
            )
            .expect("server config");
        axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
    };

    let opt = ConnectOptions::new("sqlite::memory:".to_owned());
    let db = Database::connect(opt).await.expect("in-memory SQLite");

    let settings = Settings::new(
        RegistrationSettings {
            mode: RegistrationMode::Open,
            token_hash: None,
            require_token_for_oidc: false,
        },
        168,
    );

    // Trust all IPs as proxy -- IP-based trust is tested elsewhere.
    let trust_all: Vec<IpNet> = vec![
        "0.0.0.0/0".parse().expect("v4 net"),
        "::/0".parse().expect("v6 net"),
    ];
    settings.set_trusted_proxies(trust_all).await;

    if let Some(h) = info_header {
        settings
            .set_forwarded_client_cert_info_header(Some(h.to_string()))
            .await;
    }
    if let Some(h) = pem_header {
        settings
            .set_forwarded_client_cert_pem_header(Some(h.to_string()))
            .await;
    }

    let service_connections =
        uptrakit_web_api::service_connections::ServiceConnectionRegistry::new();
    let controller_id = uuid::Uuid::nil();
    let notification_service = uptrakit_web_api::notification_service::NotificationService::new(
        service_connections.clone(),
        controller_id,
    );

    let channel_registry = Arc::new(
        uptrakit_notification_channels::ChannelRegistry::new()
            .expect("channel registry for test"),
    );
    let notification_dispatcher =
        uptrakit_web_api::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&channel_registry),
            "https://localhost".to_string(),
            settings.clone(),
        );

    let builder = AppState::builder()
        .ca_snapshot(ca_rx)
        .ca_key_store(ca_key_store)
        .db(db.clone())
        .settings(settings)
        .cert_signer(Arc::new(NoopCertSigner))
        .service_connections(service_connections)
        .revocation_notify(Arc::new(tokio::sync::Notify::const_new()))
        .jwt(Arc::new(JwtManager::from_secret(
            b"test-secret-reverse-proxy",
        )))
        .device_flow_store(DeviceFlowStore::new(db.clone()))
        .rate_limit_store(RateLimitStore::new(db.clone()))
        .pki_path(std::path::PathBuf::from("/tmp/test-pki-reverse-proxy"))
        .rustls_config(rustls_cfg)
        .crl_pem_cache(Arc::new(tokio::sync::RwLock::new(String::new())))
        .ca_rotation_trigger(Arc::new(tokio::sync::Notify::const_new()))
        .default_tenant_id(uuid::Uuid::nil())
        .controller_id(controller_id)
        .notification_service(notification_service)
        .channel_registry(channel_registry)
        .notification_dispatcher(notification_dispatcher)
        .token_denylist(Arc::new(
            uptrakit_web_api::auth::token_denylist::TokenDenylist::new(),
        ));

    #[cfg(feature = "oidc")]
    let builder = builder
        .oidc_flow_store(OidcFlowStore::new(db.clone()))
        .account_link_store(AccountLinkStore::new(db.clone()))
        .oidc_token_exchange_store(OidcTokenExchangeStore::new(db.clone()))
        .oidc_registration_store(OidcRegistrationStore::new(db.clone()));

    Arc::new(
        builder
            .build()
            .expect("all AppState fields set in test builder"),
    )
}

fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/test/identity", get(identity_handler))
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::resolve_proxy_headers::resolve_proxy_headers,
        ))
        .layer(axum_mw::from_fn_with_state(
            Arc::clone(&state),
            middleware::resolve_ip::resolve_ip,
        ))
        .with_state(state)
}

async fn identity_handler(req: Request) -> Json<IdentityResponse> {
    let identity = req.extensions().get::<ServiceIdentity>().cloned();
    Json(IdentityResponse {
        agent_id: identity.as_ref().map(|id| id.service_id.to_string()),
        cert_serial: identity
            .as_ref()
            .map(|id| id.cert_serial.clone())
            .filter(|s| !s.is_empty()),
    })
}

fn build_rustls_config(pki: &TestPki) -> axum_server::tls_rustls::RustlsConfig {
    let (_, cert_pem) = x509_parser::pem::parse_x509_pem(pki.server_cert_pem.as_bytes())
        .expect("parse server cert PEM");
    let cert_der = rustls::pki_types::CertificateDer::from(cert_pem.contents);

    let key_der = rustls::pki_types::PrivateKeyDer::try_from(pem_to_der(&pki.server_key_pem))
        .expect("server key DER");

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .expect("rustls server config");

    // RustlsConfig::from_config does not set ALPN automatically (unlike
    // from_pem / from_der). Without ALPN, reverse proxies cannot negotiate
    // HTTP/2 with the backend.
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

    axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
}

/// Decode a PEM-encoded block (any type) to raw DER bytes.
fn pem_to_der(pem_str: &str) -> Vec<u8> {
    use base64::Engine;
    let b64: String = pem_str
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("base64 decode PEM")
}
