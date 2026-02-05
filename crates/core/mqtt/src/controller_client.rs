//! WebSocket client for communicating with the controller.
//!
//! Handles the connection lifecycle including:
//! - Initial CA certificate fetch (TOFU)
//! - Anonymous enrollment
//! - Bearer token auth (enrolled but not certified)
//! - mTLS auth (certified)
//! - Message send/receive

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use rootcause::{Report, ReportConversion, markers, report};
use thiserror::Error;
use tokio_tungstenite::{
    Connector,
    tungstenite::{self, Message},
};
use uptrakit_internal_wire::{ControllerMessage, ServiceMessage};

use crate::identity::Identity;

/// Errors that can occur during controller communication.
#[derive(Debug, Error)]
pub enum ControllerError {
    #[error("connection failed: {0}")]
    Connection(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("WebSocket error: {0}")]
    WebSocket(Box<tungstenite::Error>),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("identity error: {0}")]
    Identity(#[from] crate::identity::IdentityError),
}

pub type Result<T> = std::result::Result<T, Report<ControllerError>>;

impl From<tungstenite::Error> for ControllerError {
    fn from(e: tungstenite::Error) -> Self {
        Self::WebSocket(Box::new(e))
    }
}

impl<T> ReportConversion<serde_json::Error, markers::Mutable, T> for ControllerError
where
    ControllerError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<serde_json::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(ControllerError::Json)
    }
}

impl<T> ReportConversion<tungstenite::Error, markers::Mutable, T> for ControllerError
where
    ControllerError: markers::ObjectMarkerFor<T>,
{
    fn convert_report(
        report: Report<tungstenite::Error, markers::Mutable, T>,
    ) -> Report<Self, markers::Mutable, T> {
        report.context_transform(|e| ControllerError::WebSocket(Box::new(e)))
    }
}

/// Connection mode for the WebSocket client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionMode {
    /// No identity - for enrollment
    Anonymous,
    /// Has enrollment secret but no certificate - for CSR submission
    Enrolled,
    /// Has certificate - for authenticated operation
    Authenticated,
}

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
    /// Connect to the controller WebSocket endpoint.
    ///
    /// The connection mode determines how authentication is performed:
    /// - Anonymous: No auth headers, expects enrollment flow
    /// - Enrolled: Bearer token auth with enrollment secret
    /// - Authenticated: mTLS with client certificate
    pub async fn connect(
        controller_url: &str,
        identity: &Identity,
        mode: ConnectionMode,
        insecure: bool,
    ) -> Result<Self> {
        // Build the WebSocket URL
        let ws_url = build_ws_url(controller_url)?;

        // Build TLS config based on mode
        let connector = build_tls_connector(identity, mode, insecure)?;

        // Build request with auth headers if needed
        let mut request = tungstenite::http::Request::builder()
            .uri(&ws_url)
            .header("Host", extract_host(&ws_url)?)
            .header("Connection", "Upgrade")
            .header("Upgrade", "websocket")
            .header("Sec-WebSocket-Version", "13")
            .header(
                "Sec-WebSocket-Key",
                tungstenite::handshake::client::generate_key(),
            );

        // Add auth header for enrolled mode
        if mode == ConnectionMode::Enrolled
            && let Some(secret) = identity.enrollment_secret()
        {
            request = request.header("Authorization", format!("Bearer {}", secret));
        }

        let request = request
            .body(())
            .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

        // Connect
        let (ws_stream, _response) =
            tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
                .await
                .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

        let (sink, stream) = ws_stream.split();

        Ok(Self { sink, stream })
    }

    /// Send a message to the controller.
    pub async fn send(&mut self, msg: ServiceMessage) -> Result<()> {
        let json = serde_json::to_string(&msg).map_err(|e| report!(ControllerError::Json(e)))?;
        self.sink
            .send(Message::Text(json.into()))
            .await
            .map_err(|e| report!(ControllerError::WebSocket(Box::new(e))))?;
        Ok(())
    }

    /// Receive the next message from the controller.
    pub async fn recv(&mut self) -> Result<Option<ControllerMessage>> {
        loop {
            match self.stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: ControllerMessage = serde_json::from_str(&text)
                        .map_err(|e| report!(ControllerError::Json(e)))?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Binary(data))) => {
                    let msg: ControllerMessage = serde_json::from_slice(&data)
                        .map_err(|e| report!(ControllerError::Json(e)))?;
                    return Ok(Some(msg));
                }
                Some(Ok(Message::Ping(data))) => {
                    self.sink
                        .send(Message::Pong(data))
                        .await
                        .map_err(|e| report!(ControllerError::WebSocket(Box::new(e))))?;
                    continue;
                }
                Some(Ok(Message::Pong(_))) => continue,
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(Message::Frame(_))) => continue,
                Some(Err(e)) => {
                    return Err(report!(ControllerError::WebSocket(Box::new(e))));
                }
                None => return Ok(None),
            }
        }
    }

    /// Close the connection gracefully.
    pub async fn close(mut self) -> Result<()> {
        self.sink
            .send(Message::Close(None))
            .await
            .map_err(|e| report!(ControllerError::WebSocket(Box::new(e))))?;
        Ok(())
    }
}

/// Fetch the CA certificate from the controller.
///
/// This is used for TOFU (Trust On First Use) - the first time we connect,
/// we fetch the CA cert using system roots (or insecure mode), then pin it.
pub async fn fetch_ca_cert(controller_url: &str, insecure: bool) -> Result<String> {
    let ca_url = build_ca_url(controller_url)?;

    // Build TLS config for CA fetch
    let tls_config = if insecure {
        // Accept any certificate (DANGEROUS - only for initial setup)
        build_insecure_tls_config()?
    } else {
        // Use system root certificates
        build_system_roots_tls_config()?
    };

    let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));

    // Parse URL
    let url: url::Url = ca_url
        .parse()
        .map_err(|e: url::ParseError| report!(ControllerError::Connection(e.to_string())))?;

    let host = url
        .host_str()
        .ok_or_else(|| report!(ControllerError::Connection("missing host".to_string())))?;
    let port = url.port().unwrap_or(443);

    // Connect TCP
    let addr = format!("{}:{}", host, port);
    let stream = tokio::net::TcpStream::connect(&addr)
        .await
        .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

    // Wrap with TLS
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;
    let tls_stream = connector
        .connect(server_name, stream)
        .await
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;

    // Send HTTP request
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        url.path(),
        host
    );

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (mut reader, mut writer) = tokio::io::split(tls_stream);
    writer
        .write_all(request.as_bytes())
        .await
        .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

    // Read response
    let mut response = Vec::new();
    reader
        .read_to_end(&mut response)
        .await
        .map_err(|e| report!(ControllerError::Connection(e.to_string())))?;

    // Parse response (simple HTTP/1.1 parsing)
    let response_str = String::from_utf8_lossy(&response);
    let body_start = response_str.find("\r\n\r\n").ok_or_else(|| {
        report!(ControllerError::Protocol(
            "invalid HTTP response".to_string()
        ))
    })?;

    let body = &response_str[body_start + 4..];
    Ok(body.to_string())
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

fn build_ca_url(controller_url: &str) -> Result<String> {
    let mut url = controller_url.to_string();

    // Ensure https://
    if url.starts_with("wss://") {
        url = format!("https://{}", &url[6..]);
    } else if url.starts_with("ws://") {
        url = format!("http://{}", &url[5..]);
    }

    // Append CA path
    if !url.contains("/api/v1/pki/ca.crt") {
        if url.ends_with('/') {
            url.push_str("api/v1/pki/ca.crt");
        } else {
            url.push_str("/api/v1/pki/ca.crt");
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

fn build_tls_connector(
    identity: &Identity,
    mode: ConnectionMode,
    insecure: bool,
) -> Result<Connector> {
    let tls_config = if mode == ConnectionMode::Authenticated && identity.is_certified() {
        // mTLS with client certificate
        build_mtls_config(identity)?
    } else if insecure {
        build_insecure_tls_config()?
    } else if let Some(ca_pem) = &identity.ca_cert_pem {
        // Use pinned CA
        build_pinned_ca_config(ca_pem)?
    } else {
        // Use system roots
        build_system_roots_tls_config()?
    };

    Ok(Connector::Rustls(Arc::new(tls_config)))
}

fn build_system_roots_tls_config()
-> std::result::Result<rustls::ClientConfig, Report<ControllerError>> {
    let root_store =
        rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

fn build_pinned_ca_config(
    ca_pem: &str,
) -> std::result::Result<rustls::ClientConfig, Report<ControllerError>> {
    let mut root_store = rustls::RootCertStore::empty();

    let certs = rustls_pemfile::certs(&mut ca_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;

    for cert in certs {
        root_store
            .add(cert)
            .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    Ok(config)
}

fn build_mtls_config(
    identity: &Identity,
) -> std::result::Result<rustls::ClientConfig, Report<ControllerError>> {
    let ca_pem = identity.ca_cert_pem.as_ref().ok_or_else(|| {
        report!(ControllerError::Identity(
            crate::identity::IdentityError::NotCertified,
        ))
    })?;

    let mut root_store = rustls::RootCertStore::empty();
    let ca_certs = rustls_pemfile::certs(&mut ca_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;

    for cert in ca_certs {
        root_store
            .add(cert)
            .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;
    }

    let cert_pem = identity.certificate_pem().ok_or_else(|| {
        report!(ControllerError::Identity(
            crate::identity::IdentityError::NotCertified,
        ))
    })?;

    let key_pem = identity.private_key_pem().ok_or_else(|| {
        report!(ControllerError::Identity(
            crate::identity::IdentityError::NotCertified,
        ))
    })?;

    let certs = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;

    let key = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?
        .ok_or_else(|| report!(ControllerError::Tls("no private key found".to_string())))?;

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_client_auth_cert(certs, key)
        .map_err(|e| report!(ControllerError::Tls(e.to_string())))?;

    Ok(config)
}

fn build_insecure_tls_config() -> std::result::Result<rustls::ClientConfig, Report<ControllerError>>
{
    // DANGEROUS: Accepts any certificate
    // Only use for initial CA fetch when no CA is known yet

    #[derive(Debug)]
    struct AcceptAnyCert;

    impl rustls::client::danger::ServerCertVerifier for AcceptAnyCert {
        fn verify_server_cert(
            &self,
            _end_entity: &rustls::pki_types::CertificateDer<'_>,
            _intermediates: &[rustls::pki_types::CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error>
        {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }

        fn verify_tls12_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn verify_tls13_signature(
            &self,
            _message: &[u8],
            _cert: &rustls::pki_types::CertificateDer<'_>,
            _dss: &rustls::DigitallySignedStruct,
        ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error>
        {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }

        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            vec![
                rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
                rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
                rustls::SignatureScheme::ECDSA_NISTP521_SHA512,
                rustls::SignatureScheme::ED25519,
                rustls::SignatureScheme::RSA_PSS_SHA256,
                rustls::SignatureScheme::RSA_PSS_SHA384,
                rustls::SignatureScheme::RSA_PSS_SHA512,
                rustls::SignatureScheme::RSA_PKCS1_SHA256,
                rustls::SignatureScheme::RSA_PKCS1_SHA384,
                rustls::SignatureScheme::RSA_PKCS1_SHA512,
            ]
        }
    }

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptAnyCert))
        .with_no_client_auth();

    Ok(config)
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

    #[test]
    fn build_ca_url_from_https() {
        assert_eq!(
            build_ca_url("https://controller:8443").expect("should build"),
            "https://controller:8443/api/v1/pki/ca.crt"
        );
    }

    #[test]
    fn build_ca_url_from_wss() {
        assert_eq!(
            build_ca_url("wss://controller:8443").expect("should build"),
            "https://controller:8443/api/v1/pki/ca.crt"
        );
    }
}
