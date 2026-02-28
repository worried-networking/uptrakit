use std::collections::HashMap;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    fail_closed: bool,
}

static RATE_LIMITS: LazyLock<HashMap<&'static str, EndpointRateLimit>> = LazyLock::new(|| {
    let map = HashMap::from([
        (
            "/api/v1/auth/login",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/register",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/refresh",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/device",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/device/poll",
            EndpointRateLimit {
                max_requests: 12,
                window_secs: 60,
                fail_closed: true,
            },
        ),
        (
            "/api/v1/auth/device/approve",
            EndpointRateLimit {
                max_requests: 5,
                window_secs: 60,
                fail_closed: true,
            },
        ),
    ]);

    #[cfg(feature = "oidc")]
    let map = {
        let mut map = map;
        map.insert(
            "/api/v1/auth/oidc/exchange",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        );
        map.insert(
            "/api/v1/auth/oidc/link",
            EndpointRateLimit {
                max_requests: 10,
                window_secs: 60,
                fail_closed: true,
            },
        );
        map.insert(
            "/api/v1/auth/oidc/complete-registration",
            EndpointRateLimit {
                max_requests: 5,
                window_secs: 60,
                fail_closed: true,
            },
        );
        map
    };

    map
});

struct LocalRateLimitEntry {
    count: u32,
    window_start: Instant,
    last_seen: Instant,
}

static FALLBACK_LIMITS: LazyLock<Mutex<HashMap<String, LocalRateLimitEntry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn check_local_fallback(key: &str, max_requests: i32, window_secs: i64) -> RateLimitOutcome {
    let now = Instant::now();
    let window = Duration::from_secs(window_secs as u64);
    let max = max_requests.max(0) as u32;
    let mut guard = FALLBACK_LIMITS.lock().unwrap();

    if let Some(entry) = guard.get_mut(key) {
        if now.duration_since(entry.window_start) >= window {
            entry.window_start = now;
            entry.count = 0;
        }
        entry.count = entry.count.saturating_add(1);
        entry.last_seen = now;
        if entry.count > max {
            let elapsed = now.duration_since(entry.window_start);
            let retry_after = window.saturating_sub(elapsed).as_secs().max(1);
            return RateLimitOutcome::Limited {
                retry_after_secs: retry_after,
            };
        }
    } else {
        guard.insert(
            key.to_string(),
            LocalRateLimitEntry {
                count: 1,
                window_start: now,
                last_seen: now,
            },
        );
    }

    let cutoff = now.checked_sub(window.saturating_mul(2)).unwrap_or(now);
    guard.retain(|_, entry| entry.last_seen >= cutoff);

    RateLimitOutcome::Allowed
}

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
            if limit.fail_closed {
                match check_local_fallback(&key, limit.max_requests, limit.window_secs) {
                    RateLimitOutcome::Allowed => next.run(req).await,
                    RateLimitOutcome::Limited { retry_after_secs } => {
                        let mut resp = error_response(
                            StatusCode::TOO_MANY_REQUESTS,
                            "Too many requests, please try again later",
                        );
                        resp.headers_mut()
                            .insert("retry-after", http::HeaderValue::from(retry_after_secs));
                        resp
                    }
                }
            } else {
                next.run(req).await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limited_paths_list() {
        let mut expected = vec![
            "/api/v1/auth/login",
            "/api/v1/auth/register",
            "/api/v1/auth/refresh",
            "/api/v1/auth/device",
            "/api/v1/auth/device/poll",
            "/api/v1/auth/device/approve",
        ];
        if cfg!(feature = "oidc") {
            expected.extend_from_slice(&[
                "/api/v1/auth/oidc/exchange",
                "/api/v1/auth/oidc/link",
                "/api/v1/auth/oidc/complete-registration",
            ]);
        }

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

    #[test]
    fn local_fallback_enforces_limits() {
        let key = "test:127.0.0.1";
        let _ = FALLBACK_LIMITS.lock().unwrap().remove(key);

        let limit = check_local_fallback(key, 2, 60);
        assert!(matches!(limit, RateLimitOutcome::Allowed));
        let limit = check_local_fallback(key, 2, 60);
        assert!(matches!(limit, RateLimitOutcome::Allowed));
        let limit = check_local_fallback(key, 2, 60);
        assert!(matches!(limit, RateLimitOutcome::Limited { .. }));
    }
}
