use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use rootcause::prelude::*;
use thiserror::Error;
use tokio::net::TcpSocket;
use tower_http::services::{ServeDir, ServeFile};
use uptrakit_shared_macros::impl_report_conversion;

use crate::mtls_acceptor::MtlsAcceptor;
use uptrakit_web_api::AppState;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Report<ServerError>>;

impl_report_conversion!(std::io::Error => ServerError::Io);

pub struct ServerOptions {
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
pub async fn run(cfg: ServerOptions) -> Result<()> {
    let mut router = uptrakit_web_api::build_router(cfg.app_state);
    if let Some(ref dir) = cfg.static_dir {
        let index = dir.join("index.html");
        let not_found = Router::new()
            .route(
                "/api/{*path}",
                axum::routing::any(uptrakit_web_api::api_not_found),
            )
            .route("/api", axum::routing::any(uptrakit_web_api::api_not_found))
            .fallback_service(ServeFile::new(index));
        router = router.fallback_service(ServeDir::new(dir).not_found_service(not_found));
    }

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
pub async fn run_pki_http(addr: SocketAddr, app_state: Arc<AppState>) -> Result<()> {
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
