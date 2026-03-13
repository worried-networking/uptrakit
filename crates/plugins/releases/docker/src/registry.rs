use std::sync::Arc;

use rootcause::prelude::*;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

use crate::auth::RegistryAuth;
use crate::config::DockerAuth;
use crate::error::{DockerError, Result};

/// OCI Distribution Spec manifest media types to accept.
const MANIFEST_ACCEPT: &str = concat!(
    "application/vnd.oci.image.index.v1+json, ",
    "application/vnd.oci.image.manifest.v1+json, ",
    "application/vnd.docker.distribution.manifest.list.v2+json, ",
    "application/vnd.docker.distribution.manifest.v2+json"
);

/// Low-level HTTP client for OCI Distribution API operations.
///
/// Unlike the old implementation, `RegistryClient` does not bake in a specific
/// registry hostname or repository at construction time. Instead, `registry`
/// and `repository` are passed per-call — allowing a single client instance to
/// serve multiple images with different registries.
pub struct RegistryClient {
    client: reqwest::Client,
    auth: RegistryAuth,
}

impl RegistryClient {
    /// Create a new registry client with optional authentication.
    pub fn new(auth: Option<DockerAuth>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-releases-docker/",
                env!("CARGO_PKG_VERSION")
            ))
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(SsrfSafeResolver::new()))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| {
                report!(DockerError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        Ok(Self {
            client,
            auth: RegistryAuth::new(auth),
        })
    }

    /// Get the manifest digest for a specific tag.
    ///
    /// Attempts HEAD first (no body transfer). Falls back to GET if HEAD returns
    /// 403, because some registries (notably GHCR for public packages) reject HEAD
    /// but accept GET with the same authentication token.
    pub async fn get_manifest_digest(
        &self,
        registry: &str,
        repository: &str,
        tag: &str,
    ) -> Result<String> {
        let url = format!("https://{registry}/v2/{repository}/manifests/{tag}");
        tracing::debug!(url = %url, tag = %tag, "fetching manifest digest");

        match self
            .authenticated_request(reqwest::Method::HEAD, &url)
            .await
        {
            Ok(digest) => {
                tracing::debug!(digest = %digest, tag = %tag, "fetched manifest digest via HEAD");
                Ok(digest)
            }
            Err(err) if is_forbidden_response(&err) => {
                // Some registries (e.g. GHCR for public packages) return 403 on
                // HEAD requests but accept GET with the same authentication token.
                // Fall back to GET transparently so version checks succeed.
                tracing::debug!(
                    url = %url,
                    tag = %tag,
                    "HEAD returned 403; retrying with GET"
                );
                let digest = self
                    .authenticated_request(reqwest::Method::GET, &url)
                    .await?;
                tracing::debug!(digest = %digest, tag = %tag, "fetched manifest digest via GET");
                Ok(digest)
            }
            Err(err) => Err(err),
        }
    }

    /// Perform an authenticated request (HEAD or GET) with 401 retry.
    /// Returns the `Docker-Content-Digest` header value.
    async fn authenticated_request(&self, method: reqwest::Method, url: &str) -> Result<String> {
        if let Some(token) = self.auth.cached_bearer_token() {
            let response = self
                .client
                .request(method.clone(), url)
                .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| DockerError::Request(format!("{method} failed: {e}")))?;

            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return self.extract_digest(response).await;
            }
            self.auth.clear_cache();
        }

        let response = self
            .client
            .request(method.clone(), url)
            .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
            .send()
            .await
            .context_transform(|e| DockerError::Request(format!("{method} failed: {e}")))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let www_auth = response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            let token = self.auth.fetch_token(&self.client, &www_auth).await?;

            let retry_response = self
                .client
                .request(method.clone(), url)
                .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| {
                    DockerError::Request(format!("{method} retry failed: {e}"))
                })?;

            return self.extract_digest(retry_response).await;
        }

        self.extract_digest(response).await
    }

    /// Extract the `Docker-Content-Digest` header from a manifest response.
    ///
    /// Works for both HEAD and GET responses; the header is present in both
    /// per the OCI Distribution Specification.
    async fn extract_digest(&self, response: reqwest::Response) -> Result<String> {
        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(DockerError::RateLimited);
        }

        if !status.is_success() {
            bail!(DockerError::ApiError {
                status,
                message: format!("manifest request returned {status}"),
            });
        }

        response
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                report!(DockerError::ParseResponse(
                    "missing Docker-Content-Digest header".to_string()
                ))
            })
    }
}

/// Returns `true` if the error represents a 403 Forbidden API response.
fn is_forbidden_response(err: &Report<DockerError>) -> bool {
    matches!(
        err.current_context(),
        DockerError::ApiError { status, .. } if *status == reqwest::StatusCode::FORBIDDEN
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation_succeeds() {
        assert!(RegistryClient::new(None).is_ok());
    }

    #[test]
    fn is_forbidden_response_true_for_403() {
        let err = report!(DockerError::ApiError {
            status: reqwest::StatusCode::FORBIDDEN,
            message: "manifest request returned 403 Forbidden".to_string(),
        });
        assert!(is_forbidden_response(&err));
    }

    #[test]
    fn is_forbidden_response_false_for_404() {
        let err = report!(DockerError::ApiError {
            status: reqwest::StatusCode::NOT_FOUND,
            message: "not found".to_string(),
        });
        assert!(!is_forbidden_response(&err));
    }

    #[test]
    fn is_forbidden_response_false_for_non_api_error() {
        let err = report!(DockerError::Request("connection failed".to_string()));
        assert!(!is_forbidden_response(&err));
    }
}
