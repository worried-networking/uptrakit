//! Lightweight Server-Sent Events (SSE) protocol parser.
//!
//! Parses an SSE byte stream (from `reqwest::Response::chunk()`) into
//! typed [`RawSseEvent`] values. Follows the [SSE specification][spec]:
//! fields are separated by `\n`, events are delimited by `\n\n`.
//!
//! [spec]: https://html.spec.whatwg.org/multipage/server-sent-events.html

/// A single parsed SSE event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSseEvent {
    /// The `event:` field (defaults to `"message"` if omitted).
    pub event_type: String,
    /// The concatenated `data:` field(s).
    pub data: String,
    /// The `id:` field, if present.
    pub id: Option<String>,
}

/// Errors that can occur while parsing an SSE stream.
#[derive(Debug, thiserror::Error)]
pub enum SseError {
    #[error("stream read error: {0}")]
    Transport(#[from] reqwest::Error),
}

/// Parse a `reqwest` streaming response into a stream of [`RawSseEvent`]s.
///
/// The returned stream yields one item per SSE event (delimited by blank
/// lines). Comment lines (starting with `:`) are silently skipped.
///
/// Uses `reqwest::Response::chunk()` for incremental reads, which does not
/// require the `stream` cargo feature on reqwest.
#[expect(
    clippy::string_slice,
    reason = "pos is sourced from find_event_boundary which searches for ASCII byte sequences; all slice boundaries are ASCII-safe"
)]
pub fn parse_sse_stream(
    response: reqwest::Response,
) -> impl futures_util::Stream<Item = Result<RawSseEvent, SseError>> {
    futures_util::stream::unfold(
        (response, String::new()),
        |(mut response, mut buffer)| async move {
            loop {
                // Check if we already have a complete event in the buffer.
                if let Some(pos) = find_event_boundary(&buffer) {
                    let event_text = buffer[..pos].to_string();
                    // Skip past the double newline.
                    let skip = if buffer[pos..].starts_with("\r\n\r\n") {
                        4
                    } else if buffer[pos..].starts_with("\n\n") {
                        2
                    } else {
                        // "\r\r" case
                        2
                    };
                    buffer = buffer[pos + skip..].to_string();

                    if let Some(event) = parse_event(&event_text) {
                        return Some((Ok(event), (response, buffer)));
                    }
                    // Empty event (e.g. just comments), continue parsing.
                    continue;
                }

                // Need more data from the stream.
                match response.chunk().await {
                    Ok(Some(bytes)) => {
                        let text = String::from_utf8_lossy(&bytes);
                        buffer.push_str(&text);
                    }
                    Ok(None) => {
                        // Stream ended. Try to parse any remaining buffered data.
                        if !buffer.trim().is_empty() {
                            let event_text = std::mem::take(&mut buffer);
                            if let Some(event) = parse_event(&event_text) {
                                return Some((Ok(event), (response, buffer)));
                            }
                        }
                        return None;
                    }
                    Err(e) => {
                        return Some((Err(SseError::Transport(e)), (response, buffer)));
                    }
                }
            }
        },
    )
}

/// Find the position of the first event boundary (`\n\n`, `\r\n\r\n`, or `\r\r`).
fn find_event_boundary(s: &str) -> Option<usize> {
    // Check for `\r\n\r\n` first (most specific).
    if let Some(pos) = s.find("\r\n\r\n") {
        // But also check for earlier `\n\n`.
        if let Some(nn_pos) = s.find("\n\n") {
            return Some(nn_pos.min(pos));
        }
        return Some(pos);
    }
    if let Some(pos) = s.find("\n\n") {
        return Some(pos);
    }
    s.find("\r\r")
}

/// Parse the text of a single event block into a [`RawSseEvent`].
/// Returns `None` if the block contains no data fields.
fn parse_event(text: &str) -> Option<RawSseEvent> {
    let mut event_type = None;
    let mut data_parts: Vec<&str> = Vec::new();
    let mut id = None;

    for line in text.lines() {
        if line.starts_with(':') {
            // Comment line — skip.
            continue;
        }

        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_parts.push(value.strip_prefix(' ').unwrap_or(value));
        } else if let Some(value) = line.strip_prefix("id:") {
            id = Some(value.trim().to_string());
        }
        // Other fields (e.g. `retry:`) are ignored.
    }

    if data_parts.is_empty() {
        return None;
    }

    Some(RawSseEvent {
        event_type: event_type.unwrap_or_else(|| "message".to_string()),
        data: data_parts.join("\n"),
        id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_single_event() {
        let text = "event: output\ndata: hello world";
        let event = parse_event(text).expect("should parse");
        assert_eq!(event.event_type, "output");
        assert_eq!(event.data, "hello world");
        assert_eq!(event.id, None);
    }

    #[test]
    fn parse_event_default_type() {
        let text = "data: just data";
        let event = parse_event(text).expect("should parse");
        assert_eq!(event.event_type, "message");
        assert_eq!(event.data, "just data");
    }

    #[test]
    fn parse_event_with_id() {
        let text = "event: completed\ndata: {\"status\":\"done\"}\nid: 42";
        let event = parse_event(text).expect("should parse");
        assert_eq!(event.event_type, "completed");
        assert_eq!(event.data, "{\"status\":\"done\"}");
        assert_eq!(event.id.as_deref(), Some("42"));
    }

    #[test]
    fn parse_event_multi_data_lines() {
        let text = "data: line1\ndata: line2\ndata: line3";
        let event = parse_event(text).expect("should parse");
        assert_eq!(event.data, "line1\nline2\nline3");
    }

    #[test]
    fn parse_event_skips_comments() {
        let text = ": this is a comment\nevent: output\ndata: payload";
        let event = parse_event(text).expect("should parse");
        assert_eq!(event.event_type, "output");
        assert_eq!(event.data, "payload");
    }

    #[test]
    fn parse_event_no_data_returns_none() {
        let text = "event: output\n: comment only";
        assert!(parse_event(text).is_none());
    }

    #[test]
    fn find_boundary_double_newline() {
        assert_eq!(find_event_boundary("data: hello\n\ndata: world"), Some(11));
    }

    #[test]
    fn find_boundary_crlf() {
        assert_eq!(
            find_event_boundary("data: hello\r\n\r\ndata: world"),
            Some(11)
        );
    }

    #[test]
    fn find_boundary_none() {
        assert_eq!(find_event_boundary("data: hello\n"), None);
    }
}
