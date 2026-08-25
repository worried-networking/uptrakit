//! Capped HTTP body reads.
//!
//! Upstream servers are untrusted; an unbounded body read lets a hostile
//! or misconfigured server exhaust memory. Every plugin body read goes
//! through these helpers with an explicit cap.

use rootcause::prelude::*;

/// Errors from capped body reads.
#[derive(Debug, thiserror::Error)]
pub enum BodyReadError {
    /// The body exceeded the caller's cap.
    #[error("response body exceeded the {limit}-byte limit (saw at least {seen} bytes)")]
    TooLarge {
        /// The configured cap, in bytes.
        limit: usize,
        /// The number of bytes observed before the cap was tripped.
        seen: usize,
    },
    /// The underlying network read failed.
    #[error("failed to read response body: {0}")]
    Read(#[source] reqwest::Error),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, Report<BodyReadError>>;

/// Append `chunk` to `buf`, failing if the running total exceeds `max_bytes`.
///
/// Split out from [`read_bytes_capped`] so the mid-stream cap is unit-testable
/// directly: `reqwest::Response::content_length()` is the body size-hint (not
/// the header) and the `stream` feature is off, so every constructible
/// `Response` pre-trips the Content-Length check and this streaming branch is
/// unreachable through a `Response` in tests. At runtime it fires for chunked /
/// absent-`Content-Length` upstreams.
fn push_chunk_capped(buf: &mut Vec<u8>, chunk: &[u8], max_bytes: usize) -> Result<()> {
    if buf.len() + chunk.len() > max_bytes {
        bail!(BodyReadError::TooLarge {
            limit: max_bytes,
            seen: buf.len() + chunk.len(),
        });
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

/// Read at most `max_bytes` from `resp`; error if the body is larger.
///
/// Checks `Content-Length` first (cheap reject), then enforces the cap
/// while streaming chunks, so a lying or absent `Content-Length` cannot
/// bypass it.
///
/// # Errors
///
/// [`BodyReadError::TooLarge`] when the body exceeds `max_bytes`;
/// [`BodyReadError::Read`] when the network read fails.
pub async fn read_bytes_capped(mut resp: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    if let Some(len) = resp.content_length()
        && len > max_bytes as u64
    {
        bail!(BodyReadError::TooLarge {
            limit: max_bytes,
            seen: usize::try_from(len).unwrap_or(usize::MAX),
        });
    }
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| report!(BodyReadError::Read(e)))?
    {
        push_chunk_capped(&mut buf, &chunk, max_bytes)?;
    }
    Ok(buf)
}

/// Read at most `max_bytes` and decode as UTF-8 (lossy).
///
/// Lossy decoding is deliberate: these bodies feed version parsing and
/// JSON extraction, where a replacement character fails later parsing
/// loudly instead of aborting the read here.
///
/// # Errors
///
/// Same as [`read_bytes_capped`].
pub async fn read_text_capped(resp: reqwest::Response, max_bytes: usize) -> Result<String> {
    let bytes = read_bytes_capped(resp, max_bytes).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use httpmock::MockServer;

    use super::*;

    const CAP: usize = 64;

    #[tokio::test]
    async fn at_cap_body_is_returned() {
        let server = MockServer::start_async().await;
        let body = "x".repeat(CAP);
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/at-cap");
                then.status(200).body(&body);
            })
            .await;

        let resp = reqwest::Client::new()
            .get(server.url("/at-cap"))
            .send()
            .await
            .expect("request should succeed");

        let bytes = read_bytes_capped(resp, CAP)
            .await
            .expect("body at cap should be accepted");
        assert_eq!(bytes, body.into_bytes());
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn over_cap_body_is_rejected() {
        let server = MockServer::start_async().await;
        let body = "x".repeat(CAP + 1);
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/over-cap");
                then.status(200).body(&body);
            })
            .await;

        let resp = reqwest::Client::new()
            .get(server.url("/over-cap"))
            .send()
            .await
            .expect("request should succeed");

        let err = read_bytes_capped(resp, CAP)
            .await
            .expect_err("over-cap body must be rejected");
        assert!(matches!(
            err.current_context(),
            BodyReadError::TooLarge { limit: 64, .. }
        ));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn read_text_capped_decodes() {
        let server = MockServer::start_async().await;
        let small_mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/small");
                then.status(200).body("hello");
            })
            .await;

        let resp = reqwest::Client::new()
            .get(server.url("/small"))
            .send()
            .await
            .expect("request should succeed");

        let text = read_text_capped(resp, CAP)
            .await
            .expect("small body should decode");
        assert_eq!(text, "hello");
        small_mock.assert_async().await;

        let over_body = "x".repeat(CAP + 1);
        let over_mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/over-text");
                then.status(200).body(&over_body);
            })
            .await;

        let resp = reqwest::Client::new()
            .get(server.url("/over-text"))
            .send()
            .await
            .expect("request should succeed");

        let err = read_text_capped(resp, CAP)
            .await
            .expect_err("over-cap body must be rejected");
        assert!(matches!(
            err.current_context(),
            BodyReadError::TooLarge { limit: 64, .. }
        ));
        over_mock.assert_async().await;
    }

    #[test]
    fn streaming_accumulator_rejects_over_cap_mid_stream() {
        // The branch a Content-Length pre-check can never reach in a reqwest-based
        // test: cumulative chunk length crosses CAP mid-stream.
        let mut buf = Vec::new();
        push_chunk_capped(&mut buf, &[0u8; 40], CAP).expect("under-cap chunk must be accepted");
        let err = push_chunk_capped(&mut buf, &[0u8; 30], CAP)
            .expect_err("cumulative over-cap must reject");
        assert!(matches!(
            err.current_context(),
            BodyReadError::TooLarge { limit: 64, seen } if *seen > CAP
        ));
    }
}
