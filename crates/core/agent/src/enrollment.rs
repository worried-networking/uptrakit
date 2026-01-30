use rootcause::prelude::*;
use rustls::pki_types::ServerName;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::TlsConnector;

use crate::error::{Error, Result};

#[derive(Debug, Serialize)]
struct EnrollRequestBody {
    hostname: String,
    friendly_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    enrollment_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EnrollResponse {
    pub agent_id: String,
    pub status: String,
    pub enrollment_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct EnrollStatusResponse {
    pub agent_id: String,
    pub status: String,
}

pub async fn enroll(
    host: &str,
    port: u16,
    tls: &TlsConnector,
    hostname: &str,
    friendly_name: &str,
    token: Option<&str>,
) -> Result<EnrollResponse> {
    let body = EnrollRequestBody {
        hostname: hostname.to_string(),
        friendly_name: friendly_name.to_string(),
        enrollment_token: token.map(|s| s.to_string()),
    };
    let json = serde_json::to_string(&body).context_to::<Error>()?;

    let response = https_request(
        host,
        port,
        tls,
        "POST",
        "/api/v1/agents/enroll",
        Some(&json),
        None,
    )
    .await?;
    let (status_code, body) = parse_http_response(&response)?;

    if status_code == 201 {
        let resp: EnrollResponse = serde_json::from_str(&body).context_to::<Error>()?;
        Ok(resp)
    } else if status_code == 403 {
        Err(report!(Error::Enrollment(format!(
            "enrollment forbidden (403): {body}"
        ))))
    } else {
        Err(report!(Error::Enrollment(format!(
            "unexpected status {status_code}: {body}"
        ))))
    }
}

pub async fn poll_status(
    host: &str,
    port: u16,
    tls: &TlsConnector,
    enrollment_secret: &str,
) -> Result<EnrollStatusResponse> {
    let response = https_request(
        host,
        port,
        tls,
        "GET",
        "/api/v1/agents/enroll/status",
        None,
        Some(enrollment_secret),
    )
    .await?;
    let (status_code, body) = parse_http_response(&response)?;

    if status_code == 200 {
        let resp: EnrollStatusResponse = serde_json::from_str(&body).context_to::<Error>()?;
        Ok(resp)
    } else if status_code == 401 {
        Err(report!(Error::Enrollment(
            "invalid enrollment secret (401)".to_string()
        )))
    } else {
        Err(report!(Error::Enrollment(format!(
            "unexpected status {status_code}: {body}"
        ))))
    }
}

async fn https_request(
    host: &str,
    port: u16,
    tls: &TlsConnector,
    method: &str,
    path: &str,
    body: Option<&str>,
    bearer: Option<&str>,
) -> Result<String> {
    let tcp_stream = tokio::net::TcpStream::connect((host, port))
        .await
        .context_to::<Error>()?;

    let server_name = ServerName::try_from(host.to_string()).context_to::<Error>()?;
    let mut tls_stream = tls
        .connect(server_name, tcp_stream)
        .await
        .context_to::<Error>()?;

    let mut request =
        format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n");

    if let Some(token) = bearer {
        request.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }

    if let Some(body) = body {
        request.push_str("Content-Type: application/json\r\n");
        request.push_str(&format!("Content-Length: {}\r\n", body.len()));
        request.push_str("\r\n");
        request.push_str(body);
    } else {
        request.push_str("\r\n");
    }

    tls_stream
        .write_all(request.as_bytes())
        .await
        .context_to::<Error>()?;
    tls_stream.flush().await.context_to::<Error>()?;

    let mut response = Vec::new();
    match tls_stream.read_to_end(&mut response).await {
        Ok(_) => {}
        // Servers commonly close HTTP/1.1 Connection: close responses by
        // dropping the TCP stream without sending TLS close_notify.
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {}
        Err(e) => return Err(e).context_to::<Error>()?,
    }

    String::from_utf8(response).map_err(|_| {
        report!(Error::HttpRequest(
            "response is not valid UTF-8".to_string()
        ))
    })
}

fn parse_http_response(response: &str) -> Result<(u16, String)> {
    let header_end = response
        .find("\r\n\r\n")
        .ok_or_else(|| report!(Error::HttpRequest("invalid HTTP response".to_string())))?;

    let headers = &response[..header_end];
    let body = &response[header_end + 4..];

    // Parse status code from first line: "HTTP/1.1 200 OK"
    let status_line = headers
        .lines()
        .next()
        .ok_or_else(|| report!(Error::HttpRequest("empty HTTP response".to_string())))?;

    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| {
            report!(Error::HttpRequest(format!(
                "malformed status line: {status_line}"
            )))
        })?
        .parse()
        .map_err(|_| {
            report!(Error::HttpRequest(format!(
                "invalid status code in: {status_line}"
            )))
        })?;

    Ok((status_code, body.to_string()))
}
