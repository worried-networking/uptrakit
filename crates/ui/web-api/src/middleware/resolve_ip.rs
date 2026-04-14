use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::extract::{ConnectInfo, Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::HeaderMap;
use ipnet::IpNet;

use crate::AppState;
use crate::extract::{ClientIp, ProxyIp};

/// Middleware that resolves the client IP and (optionally) the proxy IP from
/// the peer address and proxy headers, then injects [`ClientIp`] and
/// [`ProxyIp`] as request extensions.
pub async fn resolve_ip(
    State(state): State<Arc<AppState>>,
    mut req: Request,
    next: Next,
) -> Response {
    let peer_ip = req
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| canonicalize(c.0.ip()));

    let network = state.settings.network();
    let (client_ip, proxy_ip) = resolve_client_ip(
        peer_ip,
        req.headers(),
        &network.trusted_proxies,
        &network.real_ip_header,
    );

    if let Some(ip) = client_ip {
        req.extensions_mut().insert(ClientIp(ip));
    }
    if let Some(ip) = proxy_ip {
        req.extensions_mut().insert(ProxyIp(ip));
    }

    let mut response = next.run(req).await;

    // Copy extensions onto the response so the outer `request_log` middleware
    // can read them.
    if let Some(ip) = client_ip {
        response.extensions_mut().insert(ClientIp(ip));
    }
    if let Some(ip) = proxy_ip {
        response.extensions_mut().insert(ProxyIp(ip));
    }

    response
}

/// Determine the client IP and optional proxy IP.
///
/// - No peer IP → `(None, None)`
/// - Peer is NOT a trusted proxy → `(Some(peer), None)` — direct client
/// - Peer IS trusted → extract from configured header
///   - Header present and parseable → `(Some(header_ip), Some(peer))`
///   - Header missing/unparseable → `(Some(peer), Some(peer))` — peer is
///     still a trusted proxy (important for cert-header trust), but the
///     client IP falls back to the peer address.
fn resolve_client_ip(
    peer_ip: Option<IpAddr>,
    headers: &HeaderMap,
    trusted_proxies: &[IpNet],
    real_ip_header: &str,
) -> (Option<IpAddr>, Option<IpAddr>) {
    let peer = match peer_ip {
        Some(ip) => ip,
        None => return (None, None),
    };

    let from_trusted_proxy = trusted_proxies.iter().any(|net| net.contains(&peer));
    if !from_trusted_proxy {
        return (Some(peer), None);
    }

    match extract_real_ip(headers, real_ip_header) {
        Some(real_ip) => (Some(canonicalize(real_ip)), Some(peer)),
        None => (Some(peer), Some(peer)),
    }
}

/// Convert an IPv4-mapped IPv6 address (`::ffff:a.b.c.d`) to its IPv4
/// equivalent. All other addresses pass through unchanged.
fn canonicalize(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(ip),
        other => other,
    }
}

/// Parse the real client IP from the configured header.
fn extract_real_ip(headers: &HeaderMap, header_name: &str) -> Option<IpAddr> {
    match header_name.to_ascii_lowercase().as_str() {
        "forwarded" => client_ip::rightmost_forwarded(headers).ok(),
        "x-forwarded-for" => client_ip::rightmost_x_forwarded_for(headers).ok(),
        "x-real-ip" => client_ip::x_real_ip(headers).ok(),
        "cf-connecting-ip" => client_ip::cf_connecting_ip(headers).ok(),
        "true-client-ip" => client_ip::true_client_ip(headers).ok(),
        custom => headers
            .get(custom)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').find_map(|s| s.trim().parse::<IpAddr>().ok())),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::middleware as axum_mw;
    use axum::routing::get;
    use http::Request;
    use ipnet::IpNet;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use tower::ServiceExt;

    use crate::AppState;
    use crate::ServiceCredentialSources;
    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::extract::{ClientIp, ProxyIp};
    use crate::settings::Settings;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn state_with(proxies: Vec<IpNet>, header: &str) -> Arc<AppState> {
        use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
        struct NoopCertSigner;
        #[async_trait::async_trait]
        impl AgentCertSigner for NoopCertSigner {
            async fn sign_agent_csr(
                &self,
                _: &str,
                _: &uuid::Uuid,
                _: time::Duration,
            ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>>
            {
                Err(rootcause::report!(CertSignerError::Signing(
                    "noop signer".to_string(),
                )))
            }
            fn active_ca_fingerprint(&self) -> String {
                "0".repeat(64)
            }
        }

        let ca_pem = "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n";
        let snapshot_data = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: ca_pem.to_string(),
            active_fingerprint: "0".repeat(64),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![crate::ca_snapshot::TrustedCaPublic {
                cert_pem: ca_pem.to_string(),
                fingerprint: "0".repeat(64),
                not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            }],
            trusted_ca_cns: Vec::new(),
            bundle_pem: ca_pem.to_string(),
            bundle_hash: "0".repeat(64),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc() + time::Duration::days(365),
            pki_addr: None,
        };
        let (_ca_tx, ca_rx) = tokio::sync::watch::channel(snapshot_data);
        let ca_key_store: crate::CaKeyStoreRef =
            Arc::new(tokio::sync::RwLock::new(crate::ca_snapshot::CaKeyStore {
                active_key_pem: zeroize::Zeroizing::new(String::new()),
                previous_key_pem: None,
                trusted_ca_keys: vec![],
            }));

        let rustls_cfg = {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).unwrap();
            let cert = rcgen::CertificateParams::new(vec!["localhost".into()])
                .unwrap()
                .self_signed(&key_pair)
                .unwrap();
            let server_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    vec![rustls::pki_types::CertificateDer::from(cert.der().to_vec())],
                    rustls::pki_types::PrivateKeyDer::try_from(key_pair.serialize_der()).unwrap(),
                )
                .unwrap();
            axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(server_config))
        };

        let db = test_db().await;
        let settings = Settings::new(
            RegistrationSettings {
                mode: RegistrationMode::Open,
                token_hash: None,
                require_token_for_oidc: false,
            },
            168,
        );
        settings.set_trusted_proxies(proxies).await;
        settings.set_real_ip_header(header.to_string()).await;

        let notification_service = crate::notification_service::NotificationService::new(
            crate::service_connections::ServiceConnectionRegistry::new(),
            uuid::Uuid::nil(),
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("default catalog should build"),
        );

        let notification_dispatcher = crate::notifications::dispatcher::NotificationDispatcher::new(
            db.clone(),
            Arc::clone(&plugin_ops),
            "https://localhost".to_string(),
        );

        Arc::new(AppState {
            db: crate::app_state::DbState::new(db.clone()),
            cert: crate::app_state::CertState {
                ca_snapshot: ca_rx,
                ca_key_store,
                revocation_notify: Arc::new(tokio::sync::Notify::const_new()),
                crl_pem_cache: Arc::new(tokio::sync::RwLock::new(String::new())),
                ca_rotation_trigger: Arc::new(tokio::sync::Notify::const_new()),
            },
            auth: crate::app_state::AuthState {
                jwt: Arc::new(crate::auth::jwt::JwtManager::from_secret(
                    b"test-secret-resolve-ip",
                )),
                device_flow_store: crate::auth::device_flow::DeviceFlowStore::new(db.clone()),
                rate_limit_store: crate::auth::rate_limit::RateLimitStore::new(db.clone()),
                token_denylist: Arc::new(crate::auth::token_denylist::TokenDenylist::new()),
            },
            notification: crate::app_state::NotificationState {
                notification_service,
                notification_dispatcher,
                event_broadcaster: crate::event_broadcaster::EventBroadcaster::new(),
            },
            broadcast: crate::app_state::BroadcastState {
                device_flow_broadcaster: crate::device_flow_broadcaster::DeviceFlowBroadcaster::new(
                ),
                update_output_broadcaster:
                    crate::update_output_broadcaster::UpdateOutputBroadcaster::new(),
                batch_progress_broadcaster:
                    crate::batch_progress_broadcaster::BatchProgressBroadcaster::new(),
            },
            #[cfg(feature = "oidc")]
            oidc: crate::app_state::OidcState {
                oidc_flow_store: crate::auth::oidc_state::OidcFlowStore::new(db.clone()),
                account_link_store: crate::auth::oidc_state::AccountLinkStore::new(db.clone()),
                oidc_token_exchange_store: crate::auth::oidc_state::OidcTokenExchangeStore::new(
                    db.clone(),
                ),
                oidc_registration_store: crate::auth::oidc_state::OidcRegistrationStore::new(
                    db.clone(),
                ),
            },
            default_tenant_id: uuid::Uuid::nil(),
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections: crate::service_connections::ServiceConnectionRegistry::new(),
            controller_id: uuid::Uuid::nil(),
            plugin_ops,
            credential_sources: ServiceCredentialSources::default(),
            shutdown_token: Default::default(),
            embedded_service_notifier: None,
            audit_log_filter: uptrakit_audit_log::AuditFilter::default(),
            audit_log_dispatcher: uptrakit_audit_log::AuditLogDispatcher::new(Arc::new(
                uptrakit_audit_log::NoopBackend,
            )),
            surface_registry: Arc::new(crate::surface_registry::SurfaceRegistry::new(
                crate::surface_registry::SurfaceRegistryConfig::default(),
            )),
            surface_proxy: Arc::new(crate::surface_proxy::SurfaceProxy::new()),
            config_test_proxy: Arc::new(crate::config_test_proxy::ConfigTestProxy::new()),
            workload_claim_registry: Arc::new(crate::workload_claims::WorkloadClaimRegistry::new()),
            pki_path: std::path::PathBuf::from("/tmp/test-pki"),
            rustls_config: rustls_cfg,
            reject_dangerous_commands: false,
            surface_runtime_rollout: crate::app_state::SurfaceRuntimeRolloutState::phase0(
                false,
                crate::app_state::default_surface_runtime_requirements(false),
                std::collections::BTreeMap::new(),
            ),
            #[cfg(feature = "interactive")]
            interactive_sessions: crate::interactive_sessions::InteractiveSessionRegistry::new(),
        })
    }

    /// Build a minimal router that applies `resolve_ip` and echoes the
    /// extensions back in the response body.
    fn app(state: Arc<AppState>) -> Router {
        Router::new()
            .route(
                "/test",
                get(|req: Request<Body>| async move {
                    let client = req.extensions().get::<ClientIp>().map(|c| c.0.to_string());
                    let proxy = req.extensions().get::<ProxyIp>().map(|p| p.0.to_string());
                    format!(
                        "client={} proxy={}",
                        client.unwrap_or("-".into()),
                        proxy.unwrap_or("-".into()),
                    )
                }),
            )
            .layer(axum_mw::from_fn_with_state(
                Arc::clone(&state),
                super::resolve_ip,
            ))
            .with_state(state)
    }

    #[tokio::test]
    async fn no_peer_ip() {
        let state = state_with(vec![], "X-Forwarded-For").await;
        let router = app(state);
        let req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "client=- proxy=-");
    }

    #[tokio::test]
    async fn direct_untrusted_client() {
        let state = state_with(vec![], "X-Forwarded-For").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 45)), 12345);
        let mut req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "client=203.0.113.45 proxy=-");
    }

    #[tokio::test]
    async fn trusted_proxy_with_x_forwarded_for() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = state_with(vec![proxy_net], "X-Forwarded-For").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        let mut req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "203.0.113.45, 10.0.0.1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        // rightmost_x_forwarded_for returns the rightmost IP
        assert!(body_str.contains("proxy=10.0.0.1"), "body: {body_str}");
    }

    #[tokio::test]
    async fn trusted_proxy_with_forwarded_rfc7239() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = state_with(vec![proxy_net], "Forwarded").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        let mut req = Request::builder()
            .uri("/test")
            .header("forwarded", "for=203.0.113.45;by=10.0.0.1, for=10.0.0.1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let body_str = String::from_utf8(body.to_vec()).unwrap();
        assert!(body_str.contains("proxy=10.0.0.1"));
    }

    #[tokio::test]
    async fn trusted_proxy_with_x_real_ip() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = state_with(vec![proxy_net], "X-Real-Ip").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        let mut req = Request::builder()
            .uri("/test")
            .header("x-real-ip", "203.0.113.45")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "client=203.0.113.45 proxy=10.0.0.1");
    }

    #[tokio::test]
    async fn trusted_proxy_without_header() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = state_with(vec![proxy_net], "X-Forwarded-For").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        let mut req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        // Client IP falls back to peer, but ProxyIp is still set because
        // the peer is in the trusted_proxies list. This ensures cert-header
        // trust works even when the proxy doesn't send X-Forwarded-For.
        assert_eq!(body, "client=10.0.0.1 proxy=10.0.0.1");
    }

    #[tokio::test]
    async fn ipv6_client_through_proxy() {
        let proxy_net: IpNet = "::1/128".parse().unwrap();
        let state = state_with(vec![proxy_net], "X-Forwarded-For").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 12345);
        let mut req = Request::builder()
            .uri("/test")
            .header("x-forwarded-for", "2001:db8::1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "client=2001:db8::1 proxy=::1");
    }

    #[tokio::test]
    async fn custom_header_name() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = state_with(vec![proxy_net], "X-Custom-Ip").await;
        let router = app(state);
        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 12345);
        let mut req = Request::builder()
            .uri("/test")
            .header("x-custom-ip", "198.51.100.7, 10.0.0.1")
            .body(Body::empty())
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        // Custom header: rightmost parseable IP from comma-separated list
        assert_eq!(body, "client=10.0.0.1 proxy=10.0.0.1");
    }

    #[tokio::test]
    async fn ipv4_mapped_ipv6_is_normalised() {
        let state = state_with(vec![], "X-Forwarded-For").await;
        let router = app(state);
        // ::ffff:203.0.113.45 is the IPv4-mapped form of 203.0.113.45
        let v4_mapped = Ipv6Addr::new(0, 0, 0, 0, 0, 0xffff, 0xcb00, 0x712d);
        let addr = SocketAddr::new(IpAddr::V6(v4_mapped), 12345);
        let mut req = Request::builder().uri("/test").body(Body::empty()).unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let resp = router.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(body, "client=203.0.113.45 proxy=-");
    }
}
