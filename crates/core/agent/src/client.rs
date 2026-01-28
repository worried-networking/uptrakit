use std::sync::Arc;

use http::Uri;
use rootcause::prelude::*;
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, ServerName, pem::PemObject};
use tokio_rustls::TlsConnector;
use tokio_tungstenite::tungstenite::Message;
use uptrakit_internal_wire::{AgentMessage, ControllerMessage, PingPayload, now_millis};

use crate::error::{Error, Result};

pub async fn fetch_ca_certificate(host: &str, http_port: u16) -> Result<Vec<u8>> {
    let url = format!("http://{host}:{http_port}/api/v1/ca.crt");
    tracing::info!(url = %url, "fetching CA certificate");

    let stream = tokio::net::TcpStream::connect((host, http_port))
        .await
        .context_to::<Error>()?;

    let request = format!(
        "GET /api/v1/ca.crt HTTP/1.1\r\nHost: {host}:{http_port}\r\nConnection: close\r\n\r\n"
    );

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = stream;
    stream
        .write_all(request.as_bytes())
        .await
        .context_to::<Error>()?;

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .context_to::<Error>()?;

    // Parse HTTP response - find body after \r\n\r\n
    let response_str = String::from_utf8_lossy(&response);
    let body_start = response_str
        .find("\r\n\r\n")
        .ok_or_else(|| report!(Error::FetchCaHttp("invalid HTTP response".to_string())))?
        + 4;

    let body = &response[body_start..];

    // Check for HTTP error status
    if !response_str.starts_with("HTTP/1.1 200") && !response_str.starts_with("HTTP/1.0 200") {
        let status_line = response_str.lines().next().unwrap_or("unknown");
        return Err(report!(Error::FetchCaHttp(format!(
            "HTTP error: {status_line}"
        ))));
    }

    tracing::info!(bytes = body.len(), "CA certificate fetched");
    Ok(body.to_vec())
}

pub fn build_tls_connector(ca_pem: &[u8]) -> Result<TlsConnector> {
    let certs = CertificateDer::pem_slice_iter(ca_pem)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context_to::<Error>()?;

    if certs.is_empty() {
        return Err(report!(Error::NoCertificates));
    }

    let mut root_store = RootCertStore::empty();
    for cert in certs {
        root_store.add(cert).context_to::<Error>()?;
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

pub async fn connect_and_ping(host: &str, port: u16, tls_connector: TlsConnector) -> Result<()> {
    let ws_url = format!("wss://{host}:{port}/api/v1/ws/agent");
    tracing::info!(url = %ws_url, "connecting to controller");

    // Establish TCP connection
    let tcp_stream = tokio::net::TcpStream::connect((host, port))
        .await
        .context_to::<Error>()?;

    // Perform TLS handshake
    let server_name = ServerName::try_from(host.to_string()).context_to::<Error>()?;

    let tls_stream = tls_connector
        .connect(server_name, tcp_stream)
        .await
        .context_to::<Error>()?;

    // Upgrade to WebSocket
    let uri: Uri = ws_url.parse().context_to::<Error>()?;

    let (mut ws_stream, _response) = tokio_tungstenite::client_async(uri.to_string(), tls_stream)
        .await
        .context_to::<Error>()?;

    tracing::info!("connected to controller");

    // Send ping
    let agent_ts = now_millis();
    let ping = AgentMessage::Ping(PingPayload { agent_ts });
    let ping_json = serde_json::to_string(&ping).context_to::<Error>()?;

    tracing::info!(agent_ts, "sending ping");

    use futures_util::{SinkExt, StreamExt};
    ws_stream
        .send(Message::Text(ping_json.into()))
        .await
        .context_to::<Error>()?;

    // Receive pong
    let msg = ws_stream
        .next()
        .await
        .ok_or_else(|| report!(Error::ReceiveClosed))?
        .context_to::<Error>()?;

    match msg {
        Message::Text(text) => {
            let controller_msg: ControllerMessage =
                serde_json::from_str(&text).context_to::<Error>()?;

            match controller_msg {
                ControllerMessage::Pong(pong) => {
                    let now = now_millis();
                    let rtt = now - pong.agent_ts;
                    tracing::info!(
                        agent_ts = pong.agent_ts,
                        controller_ts = pong.controller_ts,
                        rtt_ms = rtt,
                        "received pong"
                    );
                }
            }
        }
        Message::Close(_) => {
            tracing::info!("connection closed by controller");
        }
        _ => {
            return Err(report!(Error::UnexpectedMessage));
        }
    }

    // Close the connection gracefully
    ws_stream.close(None).await.context_to::<Error>()?;

    Ok(())
}
