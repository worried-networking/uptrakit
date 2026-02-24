use rootcause::prelude::*;

use crate::api_types::{RegistryErrorResponse, TagListResponse};
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
    page_size: u32,
}

impl RegistryClient {
    /// Create a new registry client with optional authentication and page size.
    pub fn new(auth: Option<DockerAuth>, page_size: u32) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-provider-docker/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| {
                report!(DockerError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        Ok(Self {
            client,
            auth: RegistryAuth::new(auth),
            page_size,
        })
    }

    /// List all tags for a repository on the given registry.
    pub async fn list_tags(&self, registry: &str, repository: &str) -> Result<Vec<String>> {
        let base_url = format!("https://{registry}/v2");
        let url = format!("{base_url}/{repository}/tags/list?n={}", self.page_size);
        tracing::debug!(url = %url, "listing registry tags");

        let body = self.authenticated_get(&url).await?;
        let tag_list: TagListResponse = serde_json::from_str(&body).map_err(|e| {
            report!(DockerError::ParseResponse(format!(
                "failed to parse tag list: {e}"
            )))
        })?;

        tracing::debug!(count = tag_list.tags.len(), "fetched registry tags");
        Ok(tag_list.tags)
    }

    /// Get the manifest digest for a specific tag.
    ///
    /// Uses a HEAD request to read the `Docker-Content-Digest` header.
    pub async fn get_manifest_digest(
        &self,
        registry: &str,
        repository: &str,
        tag: &str,
    ) -> Result<String> {
        let base_url = format!("https://{registry}/v2");
        let url = format!("{base_url}/{repository}/manifests/{tag}");
        tracing::debug!(url = %url, tag = %tag, "fetching manifest digest");

        let digest = self.authenticated_head(&url).await?;
        tracing::debug!(digest = %digest, tag = %tag, "fetched manifest digest");
        Ok(digest)
    }

    /// Perform an authenticated GET request with 401 retry.
    async fn authenticated_get(&self, url: &str) -> Result<String> {
        if let Some(token) = self.auth.cached_bearer_token() {
            tracing::trace!("using cached registry auth token");
            let response = self
                .client
                .get(url)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| DockerError::Request(format!("GET failed: {e}")))?;

            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return self.handle_response(response).await;
            }
            self.auth.clear_cache();
        }

        let response = self
            .client
            .get(url)
            .send()
            .await
            .context_transform(|e| DockerError::Request(format!("GET failed: {e}")))?;

        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            let www_auth = response
                .headers()
                .get("www-authenticate")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();

            tracing::debug!("fetching new registry auth token");
            let token = self.auth.fetch_token(&self.client, &www_auth).await?;

            tracing::debug!("retrying registry request after auth refresh");
            let retry_response = self
                .client
                .get(url)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| {
                    DockerError::Request(format!("GET retry failed: {e}"))
                })?;

            return self.handle_response(retry_response).await;
        }

        self.handle_response(response).await
    }

    /// Perform an authenticated HEAD request with 401 retry.
    /// Returns the `Docker-Content-Digest` header value.
    async fn authenticated_head(&self, url: &str) -> Result<String> {
        if let Some(token) = self.auth.cached_bearer_token() {
            let response = self
                .client
                .head(url)
                .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| DockerError::Request(format!("HEAD failed: {e}")))?;

            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return self.extract_digest(response).await;
            }
            self.auth.clear_cache();
        }

        let response = self
            .client
            .head(url)
            .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
            .send()
            .await
            .context_transform(|e| DockerError::Request(format!("HEAD failed: {e}")))?;

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
                .head(url)
                .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| {
                    DockerError::Request(format!("HEAD retry failed: {e}"))
                })?;

            return self.extract_digest(retry_response).await;
        }

        self.extract_digest(response).await
    }

    /// Handle a non-401 response, checking for errors and returning the body.
    async fn handle_response(&self, response: reqwest::Response) -> Result<String> {
        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            tracing::warn!("Docker registry rate limit encountered");
            bail!(DockerError::RateLimited);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<RegistryErrorResponse>(&body)
                .ok()
                .and_then(|r| r.errors.into_iter().next())
                .map(|e| format!("{}: {}", e.code, e.message))
                .unwrap_or(body);

            bail!(DockerError::ApiError { status, message });
        }

        response.text().await.map_err(|e| {
            report!(DockerError::ParseResponse(format!(
                "failed to read response body: {e}"
            )))
        })
    }

    /// Extract the `Docker-Content-Digest` header from a HEAD response.
    async fn extract_digest(&self, response: reqwest::Response) -> Result<String> {
        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(DockerError::RateLimited);
        }

        if !status.is_success() {
            bail!(DockerError::ApiError {
                status,
                message: format!("manifest HEAD returned {status}"),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_creation_succeeds() {
        assert!(RegistryClient::new(None, 100).is_ok());
    }
}
