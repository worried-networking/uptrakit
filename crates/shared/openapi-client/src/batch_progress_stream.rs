//! Typed SSE streaming method for batch progress.
//!
//! Provides [`UptrakitClient::stream_batch_progress`] which connects to the
//! `GET /api/v1/update-batches/{id}/stream` endpoint and returns a typed
//! stream of batch progress events.

use crate::sse::{self, RawSseEvent, SseError};
use crate::{ClientError, Result, UptrakitClient};
use rootcause::prelude::*;
use serde::Deserialize;
use uuid::Uuid;

/// A typed SSE event from the batch progress stream.
#[derive(Debug, Clone)]
pub enum BatchProgressEvent {
    /// An individual update within the batch changed status.
    Update(BatchUpdateEventData),
    /// Overall batch progress summary.
    Progress(BatchProgressData),
    /// The batch has reached a terminal status.
    BatchCompleted(BatchCompletedData),
}

/// Data for an individual update event within a batch.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchUpdateEventData {
    pub event: String,
    pub update_history_id: Uuid,
    pub software_item_name: String,
    pub host_name: String,
    #[serde(default)]
    pub error: Option<String>,
}

/// Data for a batch progress summary event.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchProgressData {
    pub completed: i64,
    pub failed: i64,
    pub pending: i64,
    pub total: i32,
}

/// Data for a batch completion event.
#[derive(Debug, Clone, Deserialize)]
pub struct BatchCompletedData {
    pub status: String,
}

/// Errors specific to batch progress streaming.
#[derive(Debug, thiserror::Error)]
pub enum StreamError {
    #[error("SSE transport error: {0}")]
    Sse(#[from] SseError),

    #[error("failed to parse SSE event data: {0}")]
    Parse(#[from] serde_json::Error),
}

impl UptrakitClient {
    /// Connect to the batch progress SSE stream and return a stream of typed events.
    ///
    /// The returned stream yields [`BatchProgressEvent`] values until the batch
    /// completes (indicated by a `BatchCompleted` event) or the connection closes.
    ///
    /// This method uses no request timeout since SSE connections are long-lived.
    pub async fn stream_batch_progress(
        &self,
        id: &Uuid,
    ) -> Result<impl futures_util::Stream<Item = std::result::Result<BatchProgressEvent, StreamError>>>
    {
        let url = format!(
            "{}{}",
            self.base_url,
            crate::paths::update_batches::stream(id)
        );

        let mut req = self
            .http
            .get(&url)
            .header("Accept", "text/event-stream")
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

/// Parse a raw SSE event into a typed [`BatchProgressEvent`].
fn parse_typed_event(
    event: RawSseEvent,
) -> Option<std::result::Result<BatchProgressEvent, StreamError>> {
    match event.event_type.as_str() {
        "update" => {
            let parsed: std::result::Result<BatchUpdateEventData, _> =
                serde_json::from_str(&event.data);
            Some(parsed.map(BatchProgressEvent::Update).map_err(Into::into))
        }
        "progress" => {
            let parsed: std::result::Result<BatchProgressData, _> =
                serde_json::from_str(&event.data);
            Some(
                parsed
                    .map(BatchProgressEvent::Progress)
                    .map_err(Into::into),
            )
        }
        "batch_completed" => {
            let parsed: std::result::Result<BatchCompletedData, _> =
                serde_json::from_str(&event.data);
            Some(
                parsed
                    .map(BatchProgressEvent::BatchCompleted)
                    .map_err(Into::into),
            )
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_update_event() {
        let event = RawSseEvent {
            event_type: "update".to_string(),
            data: r#"{"event":"update_completed","update_history_id":"01234567-89ab-cdef-0123-456789abcdef","software_item_name":"nginx","host_name":"web-01"}"#.to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        assert!(matches!(typed, BatchProgressEvent::Update(ref u) if u.software_item_name == "nginx"));
    }

    #[test]
    fn parse_progress_event() {
        let event = RawSseEvent {
            event_type: "progress".to_string(),
            data: r#"{"event":"progress","completed":2,"failed":0,"pending":3,"total":5}"#
                .to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        assert!(matches!(typed, BatchProgressEvent::Progress(ref p) if p.total == 5));
    }

    #[test]
    fn parse_batch_completed_event() {
        let event = RawSseEvent {
            event_type: "batch_completed".to_string(),
            data: r#"{"event":"batch_completed","status":"completed"}"#.to_string(),
            id: None,
        };
        let result = parse_typed_event(event).expect("should produce event");
        let typed = result.expect("should parse");
        assert!(
            matches!(typed, BatchProgressEvent::BatchCompleted(ref c) if c.status == "completed")
        );
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
}
