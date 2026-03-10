use parking_lot::Mutex;
use std::time::{Duration, Instant};

use rootcause::prelude::*;

use crate::api_types::TokenResponse;
use crate::config::DockerAuth;
use crate::error::{DockerError, Result};

/// Safety margin subtracted from token expiry to avoid using expired tokens.
const EXPIRY_SAFETY_MARGIN_SECS: u64 = 30;

/// Cached bearer token with expiry tracking.
struct CachedToken {
    token: String,
    expires_at: Option<Instant>,
}

/// Handles OCI Distribution Spec token authentication.
///
/// Flow:
/// 1. Initial request returns 401 with `WWW-Authenticate: Bearer realm="...",service="..."`
/// 2. Fetch token from the realm endpoint (with credentials if configured)
/// 3. Cache token with expiry
/// 4. Retry original request with Bearer token
pub struct RegistryAuth {
    auth_config: Option<DockerAuth>,
    cached_token: Mutex<Option<CachedToken>>,
}

impl RegistryAuth {
    /// Create a new `RegistryAuth` with optional credentials.
    pub fn new(auth_config: Option<DockerAuth>) -> Self {
        Self {
            auth_config,
            cached_token: Mutex::new(None),
        }
    }

    /// Get a valid bearer token, using the cache if available.
    /// Returns `None` if no token is cached and no challenge has been processed.
    pub fn cached_bearer_token(&self) -> Option<String> {
        let guard = self.cached_token.lock();
        guard.as_ref().and_then(|ct| {
            if let Some(expires_at) = ct.expires_at
                && Instant::now() >= expires_at
            {
                return None;
            }
            Some(ct.token.clone())
        })
    }

    /// Fetch a new bearer token from the realm endpoint after a 401 challenge.
    ///
    /// # SSRF protection
    ///
    /// The realm URL is validated to ensure it does not point to a private or
    /// internal network address. Registries legitimately use a separate auth host
    /// (e.g. Docker Hub uses `auth.docker.io` as the realm while serving manifests
    /// from `registry-1.docker.io`), so requiring the realm host to match the
    /// registry host would break Docker Hub and many other registries.
    ///
    /// Low-level DNS-rebinding protection is also provided by the `SsrfSafeResolver`
    /// configured on the shared `reqwest::Client`.
    pub async fn fetch_token(
        &self,
        client: &reqwest::Client,
        www_authenticate: &str,
    ) -> Result<String> {
        let realm = extract_quoted_param(www_authenticate, "realm").ok_or_else(|| {
            report!(DockerError::AuthFailed(
                "missing realm in WWW-Authenticate header".to_string()
            ))
        })?;

        // SSRF protection: reject realm URLs that point to private/internal addresses.
        // We intentionally do NOT require the realm host to match the registry host
        // because many registries (including Docker Hub) use a separate auth server.
        let realm_parsed = url::Url::parse(realm).map_err(|e| {
            report!(DockerError::AuthFailed(format!(
                "invalid realm URL in WWW-Authenticate header: {e}"
            )))
        })?;
        let realm_host = realm_parsed.host_str().unwrap_or("");
        if uptrakit_shared_types::network::is_private_host(realm_host) {
            bail!(DockerError::AuthFailed(format!(
                "auth realm host '{realm_host}' points to a private/internal address; \
                 possible SSRF attempt rejected",
            )));
        }

        let service = extract_quoted_param(www_authenticate, "service");
        let scope = extract_quoted_param(www_authenticate, "scope");

        let mut url = realm.to_string();
        let mut has_query = url.contains('?');

        if let Some(svc) = service {
            url.push(if has_query { '&' } else { '?' });
            has_query = true;
            url.push_str("service=");
            url.push_str(svc);
        }
        if let Some(sc) = scope {
            url.push(if has_query { '&' } else { '?' });
            url.push_str("scope=");
            url.push_str(sc);
        }

        tracing::debug!(url = %url, "fetching registry auth token");

        let mut request = client.get(&url);

        // Apply credentials
        match &self.auth_config {
            Some(DockerAuth::Basic { username, password }) => {
                request = request.basic_auth(username, Some(password.expose_secret()));
            }
            Some(DockerAuth::Bearer { token }) => {
                request = request.bearer_auth(token.expose_secret());
            }
            None => {}
        }

        let response = request.send().await.map_err(|e| {
            report!(DockerError::AuthFailed(format!(
                "token request failed: {e}"
            )))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            bail!(DockerError::AuthFailed(format!(
                "token endpoint returned {status}: {body}"
            )));
        }

        let token_resp: TokenResponse = response.json().await.map_err(|e| {
            report!(DockerError::ParseResponse(format!(
                "failed to parse token response: {e}"
            )))
        })?;

        // Prefer `token`; fall back to `access_token` (OAuth 2.0 style).
        // Docker Hub returns both keys with the same value; having separate
        // optional fields avoids the serde duplicate-key error that the
        // old `alias` approach produced.
        let token = token_resp
            .token
            .or(token_resp.access_token)
            .ok_or_else(|| {
                report!(DockerError::ParseResponse(
                    "registry token response contained neither 'token' nor 'access_token'"
                        .to_string()
                ))
            })?;

        // Cache the token
        let expires_at = token_resp.expires_in.map(|secs| {
            if secs > EXPIRY_SAFETY_MARGIN_SECS {
                Instant::now() + Duration::from_secs(secs - EXPIRY_SAFETY_MARGIN_SECS)
            } else {
                Instant::now()
            }
        });

        {
            let mut guard = self.cached_token.lock();
            *guard = Some(CachedToken {
                token: token.clone(),
                expires_at,
            });
        }

        Ok(token)
    }

    /// Clear the cached token (e.g. on 401 retry).
    pub fn clear_cache(&self) {
        let mut guard = self.cached_token.lock();
        *guard = None;
    }
}

/// Extract a quoted parameter value from a `WWW-Authenticate` header.
///
/// Parses headers like:
/// `Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull"`
fn extract_quoted_param<'a>(header: &'a str, param: &str) -> Option<&'a str> {
    let search = format!("{param}=\"");
    let start = header.find(&search)? + search.len();
    let rest = &header[start..];
    let end = rest.find('"')?;
    Some(&rest[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_realm_from_docker_hub() {
        let header = r#"Bearer realm="https://auth.docker.io/token",service="registry.docker.io",scope="repository:library/nginx:pull""#;
        assert_eq!(
            extract_quoted_param(header, "realm"),
            Some("https://auth.docker.io/token")
        );
        assert_eq!(
            extract_quoted_param(header, "service"),
            Some("registry.docker.io")
        );
        assert_eq!(
            extract_quoted_param(header, "scope"),
            Some("repository:library/nginx:pull")
        );
    }

    #[test]
    fn extract_realm_from_ghcr() {
        let header = r#"Bearer realm="https://ghcr.io/token",service="ghcr.io",scope="repository:owner/repo:pull""#;
        assert_eq!(
            extract_quoted_param(header, "realm"),
            Some("https://ghcr.io/token")
        );
        assert_eq!(extract_quoted_param(header, "service"), Some("ghcr.io"));
    }

    #[test]
    fn extract_missing_param() {
        let header = r#"Bearer realm="https://auth.docker.io/token""#;
        assert_eq!(extract_quoted_param(header, "service"), None);
    }

    #[test]
    fn extract_empty_header() {
        assert_eq!(extract_quoted_param("", "realm"), None);
    }

    #[test]
    fn extract_param_no_quotes() {
        let header = "Bearer realm=noquotes";
        assert_eq!(extract_quoted_param(header, "realm"), None);
    }

    #[test]
    fn extract_param_partial_match() {
        let header = r#"Bearer realm="https://example.com""#;
        // "real" should not match "realm"
        assert_eq!(extract_quoted_param(header, "real"), None);
    }

    // ── Realm host validation tests ───────────────────────────────────────────

    /// Helper: build a minimal WWW-Authenticate header for a given realm.
    fn make_www_auth(realm: &str) -> String {
        format!(r#"Bearer realm="{realm}",service="registry.example.com""#)
    }

    #[tokio::test]
    async fn realm_public_host_allowed() {
        // Public realm host (same or different from registry) → allowed.
        // Token fetch will fail because there is no real server, but the SSRF
        // check must pass for any non-private host (including Docker Hub's
        // `auth.docker.io` realm on `registry-1.docker.io` registries).
        let auth = RegistryAuth::new(None);
        let client = reqwest::Client::new();
        for realm in &[
            "https://registry.example.com/token",
            "https://auth.docker.io/token",
            "https://evil.registry.example.com/token",
        ] {
            let www_auth = make_www_auth(realm);
            let result = auth.fetch_token(&client, &www_auth).await;
            // The request will fail (no real server), but NOT because of SSRF rejection.
            if let Err(e) = result {
                let msg = e.to_string();
                assert!(
                    !msg.contains("private/internal address"),
                    "public realm '{realm}' should not be SSRF-rejected: {msg}"
                );
            }
        }
    }

    #[tokio::test]
    async fn realm_private_ip_rejected() {
        let auth = RegistryAuth::new(None);
        let client = reqwest::Client::new();
        // Cloud metadata endpoint — link-local, always private.
        let www_auth = make_www_auth("http://169.254.169.254/latest/meta-data/");
        let result = auth.fetch_token(&client, &www_auth).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("private/internal address"),
            "link-local realm must be rejected, got: {msg}"
        );
    }

    #[tokio::test]
    async fn realm_loopback_rejected() {
        let auth = RegistryAuth::new(None);
        let client = reqwest::Client::new();
        let www_auth = make_www_auth("http://127.0.0.1/token");
        let result = auth.fetch_token(&client, &www_auth).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("private/internal address"),
            "loopback realm must be rejected, got: {msg}"
        );
    }

    #[tokio::test]
    async fn realm_rfc1918_rejected() {
        let auth = RegistryAuth::new(None);
        let client = reqwest::Client::new();
        for realm in &[
            "http://10.0.0.1/token",
            "http://172.16.0.1/token",
            "http://192.168.1.1/token",
        ] {
            let www_auth = make_www_auth(realm);
            let result = auth.fetch_token(&client, &www_auth).await;
            assert!(
                result.is_err(),
                "RFC 1918 realm '{realm}' should be rejected"
            );
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("private/internal address"),
                "RFC 1918 realm '{realm}' must be rejected with SSRF message, got: {msg}"
            );
        }
    }

    #[tokio::test]
    async fn realm_invalid_url_rejected() {
        let auth = RegistryAuth::new(None);
        let client = reqwest::Client::new();
        let www_auth = make_www_auth("not-a-valid-url");
        let result = auth.fetch_token(&client, &www_auth).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("invalid realm URL"),
            "invalid realm URL must be rejected, got: {msg}"
        );
    }

    #[test]
    fn cached_token_returns_none_initially() {
        let auth = RegistryAuth::new(None);
        assert!(auth.cached_bearer_token().is_none());
    }

    #[test]
    fn clear_cache_removes_token() {
        let auth = RegistryAuth::new(None);
        {
            let mut guard = auth.cached_token.lock();
            *guard = Some(CachedToken {
                token: "test".to_string(),
                expires_at: None,
            });
        }
        assert!(auth.cached_bearer_token().is_some());
        auth.clear_cache();
        assert!(auth.cached_bearer_token().is_none());
    }
}
