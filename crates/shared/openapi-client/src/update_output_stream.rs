//! Typed SSE streaming method for update output.
//!
//! Provides [`UptrakitClient::stream_update_output`] which connects to the
//! `GET /api/v1/update-history/{id}/output/stream` endpoint and returns a
//! typed stream of update output events.

use crate::sse::{self, RawSseEvent, SseError};
use crate::{ClientError, Result, UptrakitClient};
use rootcause::prelude::*;
use uptrakit_web_api_types::update_history::{OutputLineSSE, UpdateCompletedSSE};

/// A typed SSE event from the update output stream.
#[derive(Debug, Clone)]
pub enum UpdateOutputEvent {
    /// A line of output from the update process.
    Output(OutputLineSSE),
    /// The update has completed (or failed).
    Completed(UpdateCompletedSSE),
}

/// Errors specific to update output streaming.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("SSE transport error: {0}")]
    Sse(#[from] SseError),

    #[error("failed to parse SSE event data: {0}")]
    Parse(#[from] serde_json::Error),
}

impl UptrakitClient {
    /// Connect to the update output SSE stream and return a stream of typed events.
    ///
    /// The returned stream yields [`UpdateOutputEvent`] values until the update
    /// completes (indicated by a `Completed` event) or the connection closes.
    ///
    /// This method uses no request timeout since SSE connections are long-lived.
    pub async fn stream_update_output(
        &self,
        id: &uuid::Uuid,
    ) -> Result<impl futures_util::Stream<Item = std::result::Result<UpdateOutputEvent, StreamError>>>
    {
        let url = format!(
            "{}{}",
            self.base_url,
            crate::paths::update_history::output_stream(id)
        );

        let mut req = self
            .http
            .get(&url)
            .header("Accept", "text/event-stream")
            // Override the client's default request timeout — SSE connections
            // are long-lived and should not be timed out by the HTTP client.
            .timeout(std::time::Duration::from_secs(86400));

        if let Some(token) = &self.token {
            req = req.bearer_auth(token);
        }

        let resp = req.send().await.context_to()?;

        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!(ClientError::NotAuthenticated);
        }
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

/// Parse a raw SSE event into a typed [`UpdateOutputEvent`].
fn parse_typed_event(
    event: RawSseEvent,
) -> Option<std::result::Result<UpdateOutputEvent, StreamError>> {
    match event.event_type.as_str() {
        "output" => {
            let parsed: std::result::Result<OutputLineSSE, _> = serde_json::from_str(&event.data);
            Some(parsed.map(UpdateOutputEvent::Output).map_err(Into::into))
        }
        "completed" => {
            let parsed: std::result::Result<UpdateCompletedSSE, _> =
                serde_json::from_str(&event.data);
            Some(parsed.map(UpdateOutputEvent::Completed).map_err(Into::into))
        }
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
    fn parse_output_event() {
        let event = RawSseEvent {
            event_type: "output".to_string(),
            data: r#"{"id":"01234567-89ab-cdef-0123-456789abcdef","text":"hello\n","stream":"stdout","timestamp":"2025-01-01T00:00:00Z","seq":0}"#.to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        assert!(matches!(typed, UpdateOutputEvent::Output(ref o) if o.text == "hello\n"));
    }

    #[test]
    fn parse_completed_event() {
        let event = RawSseEvent {
            event_type: "completed".to_string(),
            data: r#"{"status":"completed","error":null}"#.to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        assert!(matches!(typed, UpdateOutputEvent::Completed(ref c) if c.status == "completed"));
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
    fn parse_malformed_data_returns_error() {
        let event = RawSseEvent {
            event_type: "output".to_string(),
            data: "not json".to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        assert!(result.is_err());
    }
}
