//! WebSocket client for authenticated communication with the controller.
//!
//! Handles the authenticated mTLS connection lifecycle for the MQTT service,
//! including message send/receive. Enrollment and CA bootstrap are handled by
//! the `uptrakit-enrollment` crate.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use rootcause::prelude::*;
use thiserror::Error;
use tokio_tungstenite::{
    Connector,
    tungstenite::{self, Message},
};
use uptrakit_internal_wire::{ControllerMessage, ServiceMessage};
use uptrakit_shared_macros::impl_report_conversion;

/// Errors that can occur during controller communication.
#[derive(Debug, Error)]
pub enum ControllerError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("WebSocket error: {0}")]
    WebSocket(Box<tungstenite::Error>),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Report<ControllerError>>;

impl From<tungstenite::Error> for ControllerError {
    fn from(e: tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(e))
    }
}

impl_report_conversion!(serde_json::Error => ControllerError::Json);
impl_report_conversion!(tungstenite::Error => ControllerError, |e| ControllerError::WebSocket(Box::new(e)));

/// A connected WebSocket client to the controller.
pub struct ControllerConnection {
    sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    stream: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
}

impl ControllerConnection {
    /// Connect to the controller WebSocket endpoint using mTLS.
    ///
    /// The `client_config` should be a `rustls::ClientConfig` configured with
    /// the CA certificate and client certificate/key (mTLS).
    pub async fn connect(
        controller_url: &str,
        client_config: rustls::ClientConfig,
    ) -> Result<Self> {
        let ws_url = build_ws_url(controller_url)?;
        let connector = Connector::Rustls(Arc::new(client_config));

        let request = tungstenite::http::Request::builder()
            .uri(&ws_url)
            .header("Host", extract_host(&ws_url)?)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            );

        let request = request
            .body(())
            .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

        let (ws_stream, _response) =
            tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
                .await
                .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

        let (sink, stream) = ws_stream.split();

        Ok(Self { sink, stream })
    }

    /// Send a message to the controller.
    pub async fn send(&mut self, msg: ServiceMessage) -> Result<()> {
        let json = serde_json::to_string(&msg).context_to::<ControllerError>()?;
        self.sink
            .send(Message::Text(json.into()))
            .await
            .context_to::<ControllerError>()?;
        Ok(())
    }

    /// Receive the next message from the controller.
    pub async fn recv(&mut self) -> Result<Option<ControllerMessage>> {
        loop {
            match self.stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: ControllerMessage =
                        serde_json::from_str(&text).context_to::<ControllerError>()?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Binary(data))) => {
                    let msg: ControllerMessage =
                        serde_json::from_slice(&data).context_to::<ControllerError>()?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Ping(data))) => {
                    self.sink
                        .send(Message::Pong(data))
                        .await
                        .context_to::<ControllerError>()?;
                    continue;
                }
                Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(Message::Frame(_))) => continue,
                Some(Err(e)) => {
                    return Err(e).context_to::<ControllerError>()?;
                }
                None => return Ok(None),
            }
        }
    }
}

// --- Helper functions ---

fn build_ws_url(controller_url: &str) -> Result<String> {
    let mut url = controller_url.to_string();

    // Convert https:// to wss://
    if url.starts_with("https://") {
        url = format!("wss://{}", &url[8..]);
    } else if url.starts_with("http://") {
        url = format!("ws://{}", &url[7..]);
    }

    // Append WebSocket path if not present
    if !url.contains("/api/v1/ws/service") {
        if url.ends_with('/') {
            url.push_str("api/v1/ws/service");
        } else {
            url.push_str("/api/v1/ws/service");
        }
    }

    Ok(url)
}

fn extract_host(url: &str) -> Result<String> {
    let url: url::Url = url
        .parse()
        .map_err(|e: url::ParseError| report!(ControllerError::Connection(e.to_string())))?;

    let host = url
        .host_str()
        .ok_or_else(|| report!(ControllerError::Connection("missing host".to_string())))?;

    if let Some(port) = url.port() {
        Ok(format!("{}:{}", host, port))
    } else {
        Ok(host.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_ws_url_from_https() {
        assert_eq!(
            build_ws_url("https://controller:8443").expect("should build"),
            "wss://controller:8443/api/v1/ws/service"
        );
    }

    #[test]
    fn build_ws_url_from_wss() {
        assert_eq!(
            build_ws_url("wss://controller:8443").expect("should build"),
            "wss://controller:8443/api/v1/ws/service"
        );
    }

    #[test]
    fn build_ws_url_preserves_path() {
        assert_eq!(
            build_ws_url("wss://controller:8443/api/v1/ws/service").expect("should build"),
            "wss://controller:8443/api/v1/ws/service"
        );
    }
}
