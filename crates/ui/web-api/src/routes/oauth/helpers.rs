//! Shared helpers for OAuth route handlers.

use std::sync::Arc;

use axum::Extension;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use uptrakit_web_api_auth::auth::rate_limit::RateLimitStore;

use crate::extract::{ClientIp, ExternalBaseUrl};
use crate::middleware::require_auth::AuthenticatedUser;
use crate::oauth::rate_limit::{EndpointKind, OAuthRateLimiter, check_rate_limit};

/// Resolve the external base URL for constructing verification and endpoint URIs.
///
/// Mirrors the resolution chain used in the legacy `device_auth_start` handler:
/// 1. `ExternalBaseUrl` extension — set by the reverse-proxy middleware.
/// 2. `Origin` header.
/// 3. `Host` header with an `https://` prefix.
/// 4. Empty string as a safe fallback.
pub(super) fn resolve_external_base_url(
    ext: Option<Extension<ExternalBaseUrl>>,
    headers: &HeaderMap,
) -> String {
    if let Some(Extension(base)) = ext {
        return base.0.clone();
    }
    if let Some(origin) = headers.get("origin").and_then(|v| v.to_str().ok()) {
        return origin.to_string();
    }
    if let Some(host) = headers.get("host").and_then(|v| v.to_str().ok()) {
        return format!("https://{host}");
    }
    String::new()
}

/// Percent-encode a string for safe use in query string values.
pub(super) fn percent_encode(s: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// Build a 302 Found redirect response.
pub(super) fn redirect_302(location: &str) -> Response {
    (
        StatusCode::FOUND,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response()
}

/// Check authentication and rate limit for an OAuth endpoint.
///
/// Returns `Ok((user, ip_str))` on success, or `Err(Response)` to return
/// immediately when the request is unauthenticated or rate-limited.
pub(super) async fn require_auth_and_rate_limit(
    auth_user: Option<Extension<AuthenticatedUser>>,
    client_ip: &Option<Extension<ClientIp>>,
    state: &Arc<crate::AppState>,
    endpoint: EndpointKind,
) -> Result<(AuthenticatedUser, String), Response> {
    let auth_user = match auth_user {
        Some(Extension(u)) => u,
        None => return Err(StatusCode::UNAUTHORIZED.into_response()),
    };
    let ip_str = match client_ip {
        Some(Extension(ClientIp(ip))) => ip.to_string(),
        None => "unknown".to_string(),
    };
    let limiter = OAuthRateLimiter::new(RateLimitStore::new(state.db().clone()));
    if let Some(r) = check_rate_limit(endpoint, &limiter, &ip_str).await {
        return Err(r);
    }
    Ok((auth_user, ip_str))
}
