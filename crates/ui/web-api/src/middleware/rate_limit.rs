use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::Response;
use http::StatusCode;

use crate::AppState;
use crate::auth::rate_limit::RateLimitOutcome;
use crate::error_response::error_response;
use crate::extract::ClientIp;

struct EndpointRateLimit {
    max_requests: i32,
    window_secs: i64,
}

static RATE_LIMITS: LazyLock<HashMap<&'static str, EndpointRateLimit>> = LazyLock::new(|| {
    HashMap::from([
        (
            "/api/v1/auth/login",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
            },
        ),
        (
            "/api/v1/auth/register",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
            },
        ),
        (
            "/api/v1/auth/refresh",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
            },
        ),
        (
            "/api/v1/auth/device",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
            },
        ),
        (
            "/api/v1/auth/device/poll",
            EndpointRateLimit {
                max_requests: 12,
                window_secs: 60,
            },
        ),
    ])
});

/// Middleware that enforces per-IP rate limits on public authentication
/// endpoints. Non-rate-limited paths pass through immediately.
pub async fn rate_limit_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let path = req.uri().path();

    let limit = match RATE_LIMITS.get(path) {
        Some(l) => l,
        None => return next.run(req).await,
    };

    let ip = match req.extensions().get::<ClientIp>() {
        Some(client_ip) => client_ip.0,
        None => return next.run(req).await,
    };

    let key = format!("{path}:{ip}");

    match state
        .rate_limit_store
        .check_rate_limit(&key, limit.max_requests, limit.window_secs)
        .await
    {
        Ok(RateLimitOutcome::Allowed) => next.run(req).await,
        Ok(RateLimitOutcome::Limited { retry_after_secs }) => {
            let mut resp = error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many requests, please try again later",
            );
            resp.headers_mut()
                .insert("retry-after", http::HeaderValue::from(retry_after_secs));
            resp
        }
        Err(e) => {
            tracing::error!("rate limit check failed: {e}");
            // Fail open: allow the request if the rate limiter is broken.
            next.run(req).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_paths_list() {
        let expected = [
            "/api/v1/auth/login",
            "/api/v1/auth/register",
            "/api/v1/auth/refresh",
            "/api/v1/auth/device",
            "/api/v1/auth/device/poll",
        ];

        for path in &expected {
            assert!(
                RATE_LIMITS.contains_key(path),
                "expected {path} to be rate-limited"
            );
        }

        assert_eq!(
            RATE_LIMITS.len(),
            expected.len(),
            "unexpected extra rate-limited paths"
        );
    }

    #[test]
    fn non_rate_limited_paths() {
        let paths = [
            "/api/v1/auth/logout",
            "/api/v1/auth/me",
            "/api/v1/auth/device/approve",
            "/healthz",
            "/api/v1/services",
        ];

        for path in &paths {
            assert!(
                !RATE_LIMITS.contains_key(path),
                "{path} should not be rate-limited"
            );
        }
    }

    #[test]
    fn device_poll_has_higher_limit() {
        let poll_limit = RATE_LIMITS
            .get("/api/v1/auth/device/poll")
            .expect("poll limit");
        let login_limit = RATE_LIMITS.get("/api/v1/auth/login").expect("login limit");

        assert!(
            poll_limit.max_requests > login_limit.max_requests,
            "device/poll should have a higher limit than login"
        );
    }
}
