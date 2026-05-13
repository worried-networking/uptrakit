//! OAuth rate-limiting helpers.
//!
//! Per MCP OAuth 2.1 spec §14.2 and §14.3, each OAuth endpoint has an
//! independent request-count limit per client IP per window.
//!
//! # Design
//!
//! [`OAuthRateLimiter`] wraps the shared [`uptrakit_web_api_auth::auth::rate_limit::RateLimitStore`]
//! and maps each [`EndpointKind`] to a composite rate-limit key of the form
//! `"{bucket_label}:{client_ip}"`.
//!
//! Route handlers call the free function [`check_rate_limit`] which returns
//! `None` on success or a pre-built 429 response that the handler can return
//! directly:
//!
//! ```ignore
//! if let Some(resp) = check_rate_limit(EndpointKind::Token, &limiter, client_ip).await {
//!     return resp;
//! }
//! ```
//!
//! Failures in the underlying store are logged at `ERROR` level and treated as
//! "allow" (fail-open) so that a transient DB error does not lock out all
//! clients.

use uptrakit_web_api_auth::auth::rate_limit::{RateLimitOutcome, RateLimitStore};

// ---------------------------------------------------------------------------
// EndpointKind
// ---------------------------------------------------------------------------

/// Identifies an OAuth endpoint for rate-limiting purposes.
///
/// Each variant maps to a distinct key prefix in the rate-limit store, a
/// default per-window limit, and a window duration.
///
/// `#[non_exhaustive]`: future OAuth endpoints may be added without a
/// breaking change.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EndpointKind {
    /// RFC 7591 Dynamic Client Registration endpoint.
    Dcr,
    /// OAuth 2.1 `/authorize` redirect endpoint.
    Authorize,
    /// OAuth 2.1 `/token` endpoint.
    Token,
    /// Consent UI POST endpoint.
    Consent,
    /// MCP authentication failure path (used to throttle failed attempts).
    McpAuthFail,
    /// Client Identifier Metadata Document (CIMD) fetch endpoint.
    CimdFetch,
}

impl EndpointKind {
    /// Settings key used to configure this endpoint's per-window limit.
    ///
    /// Reserved for future runtime configuration; the limit is currently
    /// taken from [`EndpointKind::default_per_window`].
    pub const fn settings_key(self) -> &'static str {
        match self {
            Self::Dcr => "oauth.rate.dcr_per_hour",
            Self::Authorize => "oauth.rate.authorize_per_min",
            Self::Token => "oauth.rate.token_per_min",
            Self::Consent => "oauth.rate.consent_per_min",
            Self::McpAuthFail => "oauth.rate.mcp_auth_fail_per_min",
            Self::CimdFetch => "oauth.rate.cimd_per_min",
        }
    }

    /// Default maximum requests per window, per spec §14.2.
    pub const fn default_per_window(self) -> u32 {
        match self {
            Self::Dcr => 10,         // per hour
            Self::Authorize => 60,   // per minute
            Self::Token => 60,       // per minute
            Self::Consent => 60,     // per minute
            Self::McpAuthFail => 30, // per minute
            Self::CimdFetch => 5,    // per minute
        }
    }

    /// Window duration in seconds.
    pub const fn window_secs(self) -> i64 {
        match self {
            Self::Dcr => 3600,
            _ => 60,
        }
    }

    /// Short label embedded in the composite store key.
    pub const fn bucket_label(self) -> &'static str {
        match self {
            Self::Dcr => "oauth:dcr",
            Self::Authorize => "oauth:authorize",
            Self::Token => "oauth:token",
            Self::Consent => "oauth:consent",
            Self::McpAuthFail => "oauth:mcp_auth_fail",
            Self::CimdFetch => "oauth:cimd_fetch",
        }
    }
}

// ---------------------------------------------------------------------------
// OAuthRateLimiter
// ---------------------------------------------------------------------------

/// Shared state for OAuth rate-limit checks.
///
/// Wraps [`RateLimitStore`] and provides per-endpoint key routing.
/// Designed to be stored on `AppState` (or passed via closure capture) and
/// cloned cheaply — the underlying store holds a `DatabaseConnection`
/// (connection pool) internally.
#[derive(Clone)]
pub struct OAuthRateLimiter {
    store: RateLimitStore,
}

impl OAuthRateLimiter {
    /// Create a new [`OAuthRateLimiter`] backed by `store`.
    pub fn new(store: RateLimitStore) -> Self {
        Self { store }
    }

    /// Check (and count) a request from `client_ip` against the rate limit for
    /// `endpoint`.
    ///
    /// Returns the [`RateLimitOutcome`] or propagates a store error.
    pub async fn check(
        &self,
        endpoint: EndpointKind,
        client_ip: &str,
    ) -> uptrakit_web_api_auth::auth::rate_limit::Result<RateLimitOutcome> {
        let key = format!("{}:{}", endpoint.bucket_label(), client_ip);
        self.store
            .check_rate_limit(
                &key,
                endpoint.default_per_window() as i32,
                endpoint.window_secs(),
            )
            .await
    }
}

// ---------------------------------------------------------------------------
// check_rate_limit — free function used by route handlers
// ---------------------------------------------------------------------------

/// Check whether `client_ip` has exceeded the rate limit for `endpoint`.
///
/// Returns `None` when the request is within the limit (caller continues
/// normally).  Returns `Some(Response)` with HTTP 429 and a `Retry-After`
/// header when the limit is exceeded — the caller should return that response
/// immediately.
///
/// On a store error the function logs at `ERROR` and returns `None` (fail-open)
/// so that a transient DB failure does not lock out all clients.
///
/// # Response body on 429
/// ```json
/// {"error":"invalid_request","error_description":"Too many requests"}
/// ```
pub async fn check_rate_limit(
    endpoint: EndpointKind,
    limiter: &OAuthRateLimiter,
    client_ip: &str,
) -> Option<axum::response::Response> {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;

    match limiter.check(endpoint, client_ip).await {
        Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
            let body = serde_json::json!({
                "error": "invalid_request",
                "error_description": "Too many requests"
            });
            Some(
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    [("Retry-After", retry_after_secs.to_string())],
                    axum::Json(body),
                )
                    .into_response(),
            )
        }
        Ok(RateLimitOutcome::Allowed) => None,
        Err(e) => {
            tracing::error!(error = %e, "OAuth rate limit check failed; allowing request");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    #![expect(
        clippy::expect_used,
        reason = "test assertions — panics on setup failure are acceptable in tests"
    )]

    use sea_orm::{ConnectOptions, Database, DatabaseConnection};

    use super::*;

    async fn test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    fn make_limiter(db: DatabaseConnection) -> OAuthRateLimiter {
        OAuthRateLimiter::new(RateLimitStore::new(db))
    }

    // -----------------------------------------------------------------------
    // check_returns_none_when_under_limit
    // -----------------------------------------------------------------------
    //
    // Send exactly `default_per_window` requests and verify that each one
    // returns None (allowed).
    #[tokio::test]
    async fn check_returns_none_when_under_limit() {
        let limiter = make_limiter(test_db().await);
        let max = EndpointKind::Token.default_per_window() as usize;

        for i in 0..max {
            let result = check_rate_limit(EndpointKind::Token, &limiter, "203.0.113.1").await;
            assert!(
                result.is_none(),
                "request {}/{max} should be allowed, got a response",
                i + 1
            );
        }
    }

    // -----------------------------------------------------------------------
    // check_returns_response_when_over_limit
    // -----------------------------------------------------------------------
    //
    // Exceed the limit by one and verify that:
    //   - the (N+1)-th call returns Some(Response)
    //   - the response status is 429
    //   - the Retry-After header is present and non-zero
    #[tokio::test]
    async fn check_returns_response_when_over_limit() {
        let limiter = make_limiter(test_db().await);
        let max = EndpointKind::Token.default_per_window() as usize;
        let ip = "203.0.113.2";

        // Exhaust the window.
        for _ in 0..max {
            let result = check_rate_limit(EndpointKind::Token, &limiter, ip).await;
            assert!(result.is_none(), "requests within limit must be allowed");
        }

        // The (N+1)-th request must be rate-limited.
        let response = check_rate_limit(EndpointKind::Token, &limiter, ip).await;
        assert!(
            response.is_some(),
            "request {}/{max} should be rate-limited",
            max + 1
        );

        let resp = response.unwrap();
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "expected 429 Too Many Requests"
        );

        let retry_after = resp
            .headers()
            .get("Retry-After")
            .expect("Retry-After header must be present")
            .to_str()
            .expect("Retry-After must be ASCII");
        let secs: u64 = retry_after
            .parse()
            .expect("Retry-After must be a valid integer");
        assert!(secs > 0, "Retry-After must be > 0, got {secs}");
    }

    // -----------------------------------------------------------------------
    // different_ips_have_independent_limits
    // -----------------------------------------------------------------------
    //
    // Exhaust the limit for IP A, then verify that IP B is still allowed.
    #[tokio::test]
    async fn different_ips_have_independent_limits() {
        let limiter = make_limiter(test_db().await);
        let max = EndpointKind::Token.default_per_window() as usize;
        let ip_a = "203.0.113.10";
        let ip_b = "203.0.113.11";

        // Exhaust the window for ip_a.
        for _ in 0..max {
            check_rate_limit(EndpointKind::Token, &limiter, ip_a).await;
        }

        // ip_a is now limited.
        let resp_a = check_rate_limit(EndpointKind::Token, &limiter, ip_a).await;
        assert!(
            resp_a.is_some(),
            "ip_a should be rate-limited after {max} requests"
        );

        // ip_b has never sent a request — it must still be allowed.
        let resp_b = check_rate_limit(EndpointKind::Token, &limiter, ip_b).await;
        assert!(
            resp_b.is_none(),
            "ip_b should not be affected by ip_a's rate limit"
        );
    }
}
