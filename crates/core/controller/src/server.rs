use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use rootcause::{Report, ReportConversion, markers, prelude::*};
use thiserror::Error;
use tower_http::services::{ServeDir, ServeFile};

use crate::mtls_acceptor::MtlsAcceptor;
use uptrakit_web_api::AppState;
use uptrakit_web_api::extract::Protocol;

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, Report<ServerError>>;

impl<T> ReportConversion<std::io::Error, markers::Mutable, T> for ServerError
where
    ServerError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<std::io::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(ServerError::Io)
    }
}

pub struct ServerOptions {
    pub http_addr: SocketAddr,
    pub https_addr: SocketAddr,
    pub rustls_config: axum_server::tls_rustls::RustlsConfig,
    pub app_state: Arc<AppState>,
    pub static_dir: Option<PathBuf>,
}

/// Run both HTTP and HTTPS servers.
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
    let http_handle = tokio::spawn(run_http(cfg.http_addr, router.clone()));
    let https_handle = tokio::spawn(run_https(cfg.https_addr, router, cfg.rustls_config));

    // Wait for either server to finish (which normally means an error).
    tokio::select! {
        res = http_handle => {
            res.expect("HTTP server task panicked")?;
        }
        res = https_handle => {
            res.expect("HTTPS server task panicked")?;
        }
    }

    Ok(())
}

async fn run_http(addr: SocketAddr, router: Router) -> Result<()> {
    let router = router.layer(axum::Extension(Protocol::Plain));
    tracing::info!("HTTP server listening on {addr}");
    axum_server::bind(addr)
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context_to::<ServerError>()?;
    Ok(())
}

async fn run_https(
    addr: SocketAddr,
    router: Router,
    rustls_config: axum_server::tls_rustls::RustlsConfig,
) -> Result<()> {
    let router = router.layer(axum::Extension(Protocol::Tls));
    let rustls_acceptor = axum_server::tls_rustls::RustlsAcceptor::new(rustls_config);
    let mtls_acceptor = MtlsAcceptor::new(rustls_acceptor);

    tracing::info!("HTTPS server listening on {addr}");
    axum_server::bind(addr)
        .acceptor(mtls_acceptor)
        .serve(router.into_make_service_with_connect_info::<SocketAddr>())
        .await
        .context_to::<ServerError>()?;
    Ok(())
}
