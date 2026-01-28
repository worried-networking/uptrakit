use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use rootcause::{Report, ReportConversion, markers, prelude::*};
use rustls::ServerConfig;
use thiserror::Error;

use uptrakit_web::AppState;
use uptrakit_web::extract::Protocol;

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
    pub tls_config: ServerConfig,
    pub app_state: Arc<AppState>,
}

/// Run both HTTP and HTTPS servers.
pub async fn run(cfg: ServerOptions) -> Result<()> {
    let router = uptrakit_web::build_router(cfg.app_state);
    let http_handle = tokio::spawn(run_http(cfg.http_addr, router.clone()));
    let https_handle = tokio::spawn(run_https(cfg.https_addr, router, cfg.tls_config));

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
        .serve(router.into_make_service())
        .await
        .context_to::<ServerError>()?;
    Ok(())
}

async fn run_https(addr: SocketAddr, router: Router, tls_config: ServerConfig) -> Result<()> {
    let router = router.layer(axum::Extension(Protocol::Tls));
    let tls_config = Arc::new(tls_config);
    let rustls_config = axum_server::tls_rustls::RustlsConfig::from_config(tls_config);

    tracing::info!("HTTPS server listening on {addr}");
    axum_server::bind_rustls(addr, rustls_config)
        .serve(router.into_make_service())
        .await
        .context_to::<ServerError>()?;
    Ok(())
}
