//! Shared helpers for OAuth route handlers.

use axum::Extension;
use axum::http::HeaderMap;

use crate::extract::ExternalBaseUrl;

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
