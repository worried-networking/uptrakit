use axum::extract::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::extract::{ClientIp, ProxyIp};

/// Outermost middleware — logs every request with method, path, resolved
/// client IP, response status, and latency.
///
/// Reads [`ClientIp`] / [`ProxyIp`] from the **response** extensions
/// (propagated there by the `resolve_ip` middleware).
pub async fn request_log(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();

    let start = std::time::Instant::now();
    let response = next.run(req).await;
    let latency_ms = start.elapsed().as_millis();
    let status = response.status().as_u16();

    let client_ip = response.extensions().get::<ClientIp>().map(|c| c.0);
    let proxy_ip = response.extensions().get::<ProxyIp>().map(|p| p.0);

    let client_display = client_ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "-".into());

    if let Some(proxy) = proxy_ip {
        tracing::info!(
            %method, %path,
            client_ip = %client_display,
            proxy_ip = %proxy,
            %status, %latency_ms,
        );
    } else {
        tracing::info!(
            %method, %path,
            client_ip = %client_display,
            %status, %latency_ms,
        );
    }

    response
}
