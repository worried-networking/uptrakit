use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::middleware as axum_mw;
use axum::response::IntoResponse;
use rootcause::prelude::*;
use thiserror::Error;
use tower_http::services::ServeDir;
use uptrakit_shared_macros::impl_report_conversion;

use crate::mtls_acceptor::MtlsAcceptor;
use uptrakit_web_api::AppState;

#[derive(Debug, Error)]
pub(crate) enum ServerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub(crate) type Result<T> = std::result::Result<T, Report<ServerError>>;

impl_report_conversion!(std::io::Error => ServerError::Io);

pub(crate) struct ServerOptions {
    pub https_addr: SocketAddr,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub app_state: Arc<AppState>,
    pub static_dir: Option<PathBuf>,
    /// axum_server Handle for graceful shutdown.
    pub handle: axum_server::Handle<SocketAddr>,
    /// Pre-bound HTTPS listener inherited from the parent process via `LISTEN_FDS`.
    ///
    /// When `Some`, used directly instead of calling `bind(https_addr)`.
    pub inherited_listener: Option<std::net::TcpListener>,
}

/// Run the HTTPS server.
pub(crate) async fn run(cfg: ServerOptions) -> Result<()> {
    let mut router = uptrakit_web_api::build_router(Arc::clone(&cfg.app_state));
    #[cfg(feature = "mcp")]
    {
        use uptrakit_controller_core::auth::AuthStateSource;
        use uptrakit_controller_core::db::DbStateSource;

        let mcp_state = uptrakit_mcp::state::McpState::new(
            cfg.app_state.db_state(),
            cfg.app_state.auth_state(),
            cfg.app_state.settings.clone(),
            cfg.app_state.default_tenant_id,
            cfg.app_state.controller_id,
            cfg.app_state.audit_emitter.clone(),
            cfg.app_state.shutdown_token.clone(),
            Arc::clone(&cfg.app_state.update_dispatcher),
            cfg.app_state.oauth.enabled,
            if cfg.app_state.oauth.enabled {
                Some(Arc::clone(&cfg.app_state.oauth.verifier))
            } else {
                None
            },
            if cfg.app_state.oauth.enabled {
                Some(Arc::new(cfg.app_state.oauth.canonical.clone()))
            } else {
                None
            },
        );
        router = router.merge(uptrakit_mcp::build_mcp_router(mcp_state));
    }
    if let Some(ref dir) = cfg.static_dir {
        let index_for_fallback = dir.join("index.html");
        let not_found = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(uptrakit_web_api::api_not_found),
            )
            .route("/api", axum::routing::any(uptrakit_web_api::api_not_found))
            .route(
                "/_app/{*path}",
                axum::routing::any(|| async { axum::http::StatusCode::NOT_FOUND }),
            )
            .fallback(serve_spa_fallback(index_for_fallback));
        router = router.fallback_service(ServeDir::new(dir).fallback(not_found));
    } else {
        #[cfg(feature = "embedded-frontend")]
        {
            tracing::info!("serving embedded frontend");
            router = router.fallback_service(crate::embedded_frontend::router());
        }
    }

    // Apply request_log as the outermost layer so it wraps the entire Router
    // including fallback services. This ensures all requests (API and SPA) are
    // logged with method, path, status, latency, and request ID.
    router = router.layer(axum_mw::from_fn(
        uptrakit_web_api::middleware::request_log::request_log,
    ));
    // request_id generates a unique ID per request (or preserves x-request-id
    // from the client) and creates a tracing span. Listed after request_log
    // because Axum executes layers listed later first.
    router = router.layer(axum_mw::from_fn(
        uptrakit_web_api::middleware::request_id::request_id,
    ));
    // security_headers sets standard security response headers. Listed last
    // so it executes first (outermost layer), ensuring all responses carry
    // the headers regardless of which inner handler served them.
    router = router.layer(axum_mw::from_fn(
        uptrakit_web_api::middleware::security_headers::security_headers,
    ));

    let rustls_acceptor = axum_server::tls_rustls::RustlsAcceptor::new(cfg.rustls_config);
    let mtls_acceptor = MtlsAcceptor::new(rustls_acceptor);

    let listener = match cfg.inherited_listener {
        Some(fd) => {
            tracing::info!(
                "HTTPS server reusing inherited socket on {}",
                cfg.https_addr
            );
            fd
        }
        None => {
            tracing::info!("HTTPS server listening on {}", cfg.https_addr);
            let listener = std::net::TcpListener::bind(cfg.https_addr).map_err(ServerError::Io)?;
            // axum-server 0.8 does not call set_nonblocking() (upstream bug
            // #181). Without it, tokio runs the accept() syscall directly on
            // a worker thread and graceful_shutdown cannot interrupt it,
            // hanging Runtime::drop after `graceful shutdown complete`.
            listener.set_nonblocking(true).map_err(ServerError::Io)?;
            listener
        }
    };
    axum_server::from_tcp(listener)
        .context_to::<ServerError>()?
        .acceptor(mtls_acceptor)
        .handle(cfg.handle)
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context_to::<ServerError>()?;

    Ok(())
}

/// Run a plain HTTP server for PKI-only endpoints (OCSP, CRL, CA cert).
///
/// Started when `--pki-http listener` is set. Required for Nginx `ssl_ocsp_responder`
/// which only supports `http://` OCSP responder URLs.
///
/// When `inherited` is `Some`, the pre-bound socket is reused instead of calling
/// `bind(addr)` — used on the reexec path to avoid a brief port-unavailable window.
pub(crate) async fn run_pki_http(
    addr: SocketAddr,
    app_state: Arc<AppState>,
    inherited: Option<std::net::TcpListener>,
) -> Result<()> {
    let router = uptrakit_web_api::build_pki_router(app_state);
    let listener = match inherited {
        Some(fd) => {
            tracing::info!("PKI HTTP server reusing inherited socket on {addr}");
            tokio::net::TcpListener::from_std(fd).context_to::<ServerError>()?
        }
        None => {
            tracing::info!("PKI HTTP server listening on {addr}");
            tokio::net::TcpListener::bind(addr)
                .await
                .context_to::<ServerError>()?
        }
    };
    axum::serve(
        listener,
        router.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context_to::<ServerError>()?;
    Ok(())
}

/// Return a handler that serves `index.html` from the filesystem with an
/// explicit `200 OK` and `Content-Type: text/html`. Used as the SPA fallback
/// for the static-dir frontend so that client-side routing paths always
/// receive the entry page with a successful status code.
fn serve_spa_fallback(index_path: PathBuf) -> axum::routing::MethodRouter {
    axum::routing::get(move || {
        let path = index_path.clone();
        async move {
            match tokio::fs::read(&path).await {
                Ok(bytes) => (
                    axum::http::StatusCode::OK,
                    [
                        (axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8"),
                        (axum::http::header::CACHE_CONTROL, "no-cache"),
                    ],
                    bytes,
                )
                    .into_response(),
                Err(_) => axum::http::StatusCode::NOT_FOUND.into_response(),
            }
        }
    })
}
