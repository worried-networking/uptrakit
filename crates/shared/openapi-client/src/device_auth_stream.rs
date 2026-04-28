//! Typed SSE streaming method for device authorization.
//!
//! Provides [`UptrakitClient::stream_device_auth`] which connects to the
//! `GET /api/v1/auth/device/stream` endpoint and returns a typed stream of
//! device authorization events.

use crate::sse::{self, RawSseEvent, SseError};
use crate::types_impl::device_auth::DeviceAuthAuthorizedSse;
use crate::{ClientError, Result, UptrakitClient};
use rootcause::prelude::*;

/// A typed SSE event from the device auth stream.
#[derive(Debug, Clone)]
pub enum DeviceAuthSseEvent {
    /// The device flow was approved; contains the API token and token name.
    Authorized { token: String, token_name: String },
    /// The device flow expired before approval.
    Expired,
}

/// Errors specific to device auth streaming.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("SSE transport error: {0}")]
    Sse(#[from] SseError),

    #[error("failed to parse SSE event data: {0}")]
    Parse(#[from] serde_json::Error),
}

impl UptrakitClient {
    /// Connect to the device auth SSE stream and return a stream of typed events.
    ///
    /// The returned stream yields [`DeviceAuthSseEvent`] values until the device
    /// flow is authorized or expires, then the stream closes.
    ///
    /// This is an unauthenticated endpoint (same as device auth poll).
    /// Uses a 700s timeout (slightly beyond the 600s flow TTL).
    #[cfg_attr(feature = "tracing", tracing::instrument(skip_all))]
    pub async fn stream_device_auth(
        &self,
        device_code: &str,
    ) -> Result<
        impl futures_util::Stream<Item = std::result::Result<DeviceAuthSseEvent, StreamError>>,
    > {
        let url = format!("{}{}", self.base_url, crate::paths::auth::DEVICE_STREAM);

        let req = self
            .http
            .get(&url)
            .query(&[("device_code", device_code)])
            .header("Accept", "text/event-stream")
            // Override the client's default request timeout — SSE connections
            // are long-lived and should not be timed out by the HTTP client.
            .timeout(std::time::Duration::from_secs(700));

        let resp = req.send().await.context_to()?;

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            let text = resp.text().await.context_to()?;
            let message = crate::extract_error_message(&text);
            bail!(ClientError::NotFound(message));
        }
        if status.is_client_error() || status.is_server_error() {
            let text = resp.text().await.context_to()?;
            let message = crate::extract_error_message(&text);
            bail!(ClientError::Api { status, message });
        }

        let raw_stream = sse::parse_sse_stream(resp);

        let typed_stream = futures_util::StreamExt::filter_map(raw_stream, |result| async move {
            match result {
                Ok(event) => parse_typed_event(event),
                Err(e) => Some(Err(StreamError::Sse(e))),
            }
        });

        Ok(typed_stream)
    }
}

/// Parse a raw SSE event into a typed [`DeviceAuthSseEvent`].
fn parse_typed_event(
    event: RawSseEvent,
) -> Option<std::result::Result<DeviceAuthSseEvent, StreamError>> {
    match event.event_type.as_str() {
        "authorized" => {
            let parsed: std::result::Result<DeviceAuthAuthorizedSse, _> =
                serde_json::from_str(&event.data);
            Some(
                parsed
                    .map(|a| DeviceAuthSseEvent::Authorized {
                        token: a.token.expose_secret().to_string(),
                        token_name: a.token_name,
                    })
                    .map_err(Into::into),
            )
        }
        "expired" => Some(Ok(DeviceAuthSseEvent::Expired)),
        _ => {
            // Unknown event types (e.g. keep-alive comments) are silently skipped.
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::RawSseEvent;

    #[test]
    fn parse_authorized_event() {
        let event = RawSseEvent {
            event_type: "authorized".to_string(),
            data: r#"{"token":"upt_secret123","token_name":"cli-host-2026"}"#.to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        match typed {
            DeviceAuthSseEvent::Authorized { token, token_name } => {
                assert_eq!(token, "upt_secret123");
                assert_eq!(token_name, "cli-host-2026");
            }
            _ => panic!("expected Authorized"),
        }
    }

    #[test]
    fn parse_expired_event() {
        let event = RawSseEvent {
            event_type: "expired".to_string(),
            data: r#"{"message":"Device flow expired"}"#.to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        assert!(matches!(typed, DeviceAuthSseEvent::Expired));
    }

    #[test]
    fn parse_unknown_event_returns_none() {
        let event = RawSseEvent {
            event_type: "ping".to_string(),
            data: "{}".to_string(),
            id: None,
        };
        assert!(parse_typed_event(event).is_none());
    }

    #[test]
    fn parse_malformed_authorized_returns_error() {
        let event = RawSseEvent {
            event_type: "authorized".to_string(),
            data: "not json".to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        assert!(result.is_err());
    }
}
