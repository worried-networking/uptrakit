use std::sync::Mutex;
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
        let guard = self.cached_token.lock().unwrap();
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

        // Cache the token
        let expires_at = token_resp.expires_in.map(|secs| {
            if secs > EXPIRY_SAFETY_MARGIN_SECS {
                Instant::now() + Duration::from_secs(secs - EXPIRY_SAFETY_MARGIN_SECS)
            } else {
                Instant::now()
            }
        });

        let token = token_resp.token.clone();
        {
            let mut guard = self.cached_token.lock().unwrap();
            *guard = Some(CachedToken {
                token: token_resp.token,
                expires_at,
            });
        }

        Ok(token)
    }

    /// Clear the cached token (e.g. on 401 retry).
    pub fn clear_cache(&self) {
        let mut guard = self.cached_token.lock().unwrap();
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

    #[test]
    fn cached_token_returns_none_initially() {
        let auth = RegistryAuth::new(None);
        assert!(auth.cached_bearer_token().is_none());
    }

    #[test]
    fn clear_cache_removes_token() {
        let auth = RegistryAuth::new(None);
        {
            let mut guard = auth.cached_token.lock().unwrap();
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
