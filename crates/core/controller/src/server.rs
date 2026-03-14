use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::middleware as axum_mw;
use axum::response::IntoResponse;
use rootcause::prelude::*;
use thiserror::Error;
use tokio::net::TcpSocket;
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
    /// Enable SO_REUSEPORT for zero-downtime restarts.
    pub enable_reuseport: bool,
}

/// Create a TCP listener with optional SO_REUSEPORT.
///
/// When `reuseport` is true, enables the `SO_REUSEPORT` socket option which allows
/// multiple processes to bind to the same address. This is required for HAProxy-style
/// zero-downtime restarts where the new process starts accepting connections before
/// the old process finishes draining.
async fn create_listener(
    addr: SocketAddr,
    reuseport: bool,
) -> std::io::Result<std::net::TcpListener> {
    let socket = if addr.is_ipv6() {
        TcpSocket::new_v6()?
    } else {
        TcpSocket::new_v4()?
    };

    if reuseport {
        socket.set_reuseport(true)?;
        tracing::info!("SO_REUSEPORT enabled on {addr}");
    }

    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(1024)?.into_std()
}

/// Run the HTTPS server.
pub(crate) async fn run(cfg: ServerOptions) -> Result<()> {
    let mut router = uptrakit_web_api::build_router(cfg.app_state);
    if let Some(ref dir) = cfg.static_dir {
        let index_for_fallback = dir.join("index.html");
        let not_found = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(uptrakit_web_api::api_not_found),
            )
            .route("/api", axum::routing::any(uptrakit_web_api::api_not_found))
            .fallback(serve_spa_fallback(index_for_fallback));
        router = router.fallback_service(ServeDir::new(dir).not_found_service(not_found));
    } else {
        #[cfg(feature = "embed-frontend")]
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

    let listener = create_listener(cfg.https_addr, cfg.enable_reuseport)
        .await
        .context_to::<ServerError>()?;

    tracing::info!("HTTPS server listening on {}", cfg.https_addr);
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
pub(crate) async fn run_pki_http(addr: SocketAddr, app_state: Arc<AppState>) -> Result<()> {
    let router = uptrakit_web_api::build_pki_router(app_state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context_to::<ServerError>()?;
    tracing::info!("PKI HTTP server listening on {addr}");
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
                    [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
                    bytes,
                )
                    .into_response(),
                Err(_) => axum::http::StatusCode::NOT_FOUND.into_response(),
            }
        }
    })
}
