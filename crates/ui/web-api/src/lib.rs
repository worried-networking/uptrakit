pub(crate) mod actions;
pub mod api_error;
pub mod app_state;
pub mod embedded_support;
pub use uptrakit_web_api_auth::auth;
pub mod batch_progress_broadcaster;
pub mod ca_snapshot;
pub mod cert_signer;
pub mod config_test_proxy;
pub mod device_flow_broadcaster;
pub mod error_response;
pub mod event_broadcaster;
pub mod event_delivery;
pub mod extract;
pub mod middleware;
#[cfg(feature = "nats")]
pub mod nats_transport;
pub mod notification_service;
pub mod notifications;
pub mod ocsp;
#[cfg(feature = "oidc")]
pub(crate) mod oidc_http_client;
pub mod pki_utils;
pub use uptrakit_web_api_queries::notifier::ServiceNotifier;
pub use uptrakit_web_api_queries::queries;
pub mod router;
pub mod routes;
pub mod service_connections;
pub use uptrakit_web_api_auth::setting_key;
pub mod settings;
pub mod surface_proxy;
pub mod surface_registry;
pub use uptrakit_web_api_auth::settings_store;
#[cfg(feature = "interactive")]
pub mod interactive_sessions;
pub mod tenant_db;
pub mod update_output_broadcaster;
pub mod workload_claims;

#[cfg(feature = "oidc")]
pub use app_state::OidcState;
pub use app_state::{
    AppState, AppStateBuildError, AppStateBuilder, AuthState, BroadcastState, CertState,
    NotificationState, SURFACE_PROVIDER_APP_MQTT, SURFACE_PROVIDER_APP_SSH_AGENT,
    ServiceCredentialSources, SurfaceFrameworkGeneration, SurfaceProviderReport,
    SurfaceProviderRequirement, SurfaceRuntimeMode, SurfaceRuntimeRolloutState,
    default_surface_runtime_requirements,
};
pub use ca_snapshot::{CaKeyStoreRef, CaSnapshotReceiver};
pub use embedded_support::EmbeddedServiceNotifier;
pub use router::{api_not_found, build_pki_router, build_router};
pub use uptrakit_web_api_auth::SettingKey;
pub use uptrakit_web_api_types::MaskedUrl;

#[cfg(all(test, feature = "db-sqlite"))]
mod test_harness;

#[cfg(all(test, feature = "db-sqlite"))]
mod integration_tests;

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;

    use axum::body::Body;
    use http::Request;
    use http_body_util::BodyExt;
    use ipnet::IpNet;
    use sea_orm::{ConnectOptions, Database, DatabaseConnection};
    use tower::ServiceExt;

    use crate::auth::registration::{RegistrationMode, RegistrationSettings};
    use crate::cert_signer::{AgentCertSigner, CertSignerError, SignedCertBundle};
    use crate::settings::Settings;
    use crate::{AppState, ServiceCredentialSources, build_pki_router, build_router};

    struct NoopCertSigner;
    #[async_trait::async_trait]
    impl AgentCertSigner for NoopCertSigner {
        async fn sign_agent_csr(
            &self,
            _: &str,
            _: &uuid::Uuid,
            _: time::Duration,
        ) -> std::result::Result<SignedCertBundle, rootcause::Report<CertSignerError>> {
            Err(rootcause::report!(CertSignerError::Signing(
                "noop signer".to_string(),
            )))
        }

        fn active_ca_fingerprint(&self) -> String {
            "0000000000000000000000000000000000000000000000000000000000000000".to_string()
        }
    }

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:".to_owned());
        Database::connect(opt).await.expect("test db")
    }

    async fn test_state() -> Arc<AppState> {
        test_state_with_proxies(vec![]).await
    }

    async fn test_state_with_proxies(trusted_proxies: Vec<IpNet>) -> Arc<AppState> {
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

        // Create a dummy RustlsConfig — tests don't actually do TLS handshakes.
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
        if !trusted_proxies.is_empty() {
            settings.set_trusted_proxies(trusted_proxies).await;
        }

        let service_connections = crate::service_connections::ServiceConnectionRegistry::new();
        let controller_id = uuid::Uuid::nil();
        let notification_service = crate::notification_service::NotificationService::new(
            service_connections.clone(),
            controller_id,
        );

        let plugin_ops: Arc<dyn uptrakit_plugin_infrastructure_registry::PluginOps> = Arc::new(
            uptrakit_plugin_infrastructure_registry::build_catalog(
                &uptrakit_plugin_infrastructure_registry::CatalogConfig::default(),
            )
            .expect("catalog should build in tests"),
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
                    b"test-secret-lib",
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
            settings,
            cert_signer: Arc::new(NoopCertSigner),
            service_connections,
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
            default_tenant_id: uuid::Uuid::nil(),
            controller_id,
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

    #[tokio::test]
    async fn healthz_returns_ok() {
        let app = build_router(test_state().await);
        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"ok");
    }

    #[tokio::test]
    async fn ca_cert_returns_pem() {
        let app = build_router(test_state().await);
        let req = Request::builder()
            .uri("/api/v1/pki/ca.crt")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/x-pem-file"
        );
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert!(body.starts_with(b"-----BEGIN CERTIFICATE-----"));
    }

    #[tokio::test]
    async fn unknown_path_returns_404_not_https_error() {
        let app = build_router(test_state().await);

        // Test root path
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        // Should return 404 Not Found, not 403 Forbidden
        assert_eq!(resp.status(), 404);

        // Test another unknown path
        let req = Request::builder()
            .uri("/unknown/path")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 404);
    }

    /// Verify that `into_make_service_with_connect_info` properly injects
    /// `ConnectInfo<SocketAddr>` so the `resolve_ip` middleware can resolve
    /// the client IP — this is the production code path via `axum-server`.
    #[tokio::test]
    async fn make_service_with_connect_info_resolves_client_ip() {
        let router = build_router(test_state().await);
        let mut make_svc = router.into_make_service_with_connect_info::<SocketAddr>();

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 45)), 12345);
        // Simulate what axum-server does: call the make service with the peer
        // SocketAddr to obtain a per-connection service.
        let svc = <_ as tower::Service<SocketAddr>>::call(&mut make_svc, addr)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        // The resolve_ip middleware should have read ConnectInfo<SocketAddr>,
        // created ClientIp, and copied it onto the response extensions.
        let client_ip = resp.extensions().get::<crate::extract::ClientIp>();
        assert!(
            client_ip.is_some(),
            "ClientIp should be present in response extensions"
        );
        assert_eq!(
            client_ip.unwrap().0,
            IpAddr::V4(Ipv4Addr::new(203, 0, 113, 45))
        );
    }

    /// Verify the PKI router resolves client IPs the same way the main router
    /// does — through `resolve_ip` middleware and `ConnectInfo<SocketAddr>`.
    #[tokio::test]
    async fn pki_router_resolves_client_ip() {
        let router = build_pki_router(test_state().await);
        let mut make_svc = router.into_make_service_with_connect_info::<SocketAddr>();

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7)), 54321);
        let svc = <_ as tower::Service<SocketAddr>>::call(&mut make_svc, addr)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/healthz")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let client_ip = resp.extensions().get::<crate::extract::ClientIp>();
        assert!(
            client_ip.is_some(),
            "ClientIp should be present in PKI router response extensions"
        );
        assert_eq!(
            client_ip.unwrap().0,
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 7))
        );
    }

    /// Verify the PKI router honours trusted proxies when resolving client IPs.
    #[tokio::test]
    async fn pki_router_resolves_proxy_ip() {
        let proxy_net: IpNet = "10.0.0.0/8".parse().unwrap();
        let state = test_state_with_proxies(vec![proxy_net]).await;
        let router = build_pki_router(state);
        let mut make_svc = router.into_make_service_with_connect_info::<SocketAddr>();

        let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 54321);
        let svc = <_ as tower::Service<SocketAddr>>::call(&mut make_svc, addr)
            .await
            .unwrap();

        let req = Request::builder()
            .uri("/api/v1/pki/ca.crt")
            .header("x-forwarded-for", "203.0.113.45, 10.0.0.1")
            .body(Body::empty())
            .unwrap();
        let resp = svc.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), 200);

        let proxy_ip = resp.extensions().get::<crate::extract::ProxyIp>();
        assert!(
            proxy_ip.is_some(),
            "ProxyIp should be present when request comes from a trusted proxy"
        );
        assert_eq!(proxy_ip.unwrap().0, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn ca_key_store_debug_redacts_keys() {
        let store = crate::ca_snapshot::CaKeyStore {
            active_key_pem: zeroize::Zeroizing::new(
                "-----BEGIN PRIVATE KEY-----\ntest\n-----END PRIVATE KEY-----\n".to_string(),
            ),
            previous_key_pem: Some(zeroize::Zeroizing::new(
                "-----BEGIN PRIVATE KEY-----\nold\n-----END PRIVATE KEY-----\n".to_string(),
            )),
            trusted_ca_keys: vec![],
        };
        let debug_output = format!("{store:?}");
        assert!(
            !debug_output.contains("BEGIN"),
            "Debug output must not contain PEM markers"
        );
        assert!(
            debug_output.contains("REDACTED"),
            "Debug output must contain REDACTED"
        );
    }

    #[test]
    fn ca_public_snapshot_has_no_key_fields() {
        let snapshot = crate::ca_snapshot::CaPublicSnapshot {
            active_cert_pem: String::new(),
            active_fingerprint: String::new(),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![],
            trusted_ca_cns: vec![],
            bundle_pem: String::new(),
            bundle_hash: String::new(),
            managed: false,
            active_not_after: time::OffsetDateTime::now_utc(),
            pki_addr: None,
        };
        let debug_output = format!("{snapshot:?}");
        assert!(
            !debug_output.contains("key_pem"),
            "CaPublicSnapshot must not expose key material"
        );
    }
}
