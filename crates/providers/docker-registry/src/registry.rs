use rootcause::prelude::*;

use crate::api_types::{RegistryErrorResponse, TagListResponse};
use crate::auth::RegistryAuth;
use crate::config::DockerRegistryConfig;
use crate::error::{DockerRegistryError, Result};

/// OCI Distribution Spec manifest media types to accept.
const MANIFEST_ACCEPT: &str = concat!(
    "application/vnd.oci.image.index.v1+json, ",
    "application/vnd.oci.image.manifest.v1+json, ",
    "application/vnd.docker.distribution.manifest.list.v2+json, ",
    "application/vnd.docker.distribution.manifest.v2+json"
);

/// Low-level HTTP client for OCI Distribution API operations.
pub struct RegistryClient {
    client: reqwest::Client,
    auth: RegistryAuth,
    base_url: String,
    repository: String,
    page_size: u32,
}

impl RegistryClient {
    /// Create a new registry client from provider configuration.
    pub fn new(config: &DockerRegistryConfig) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-provider-docker-registry/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(|e| {
                report!(DockerRegistryError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        let registry = config.resolved_registry();
        let base_url = format!("https://{registry}/v2");
        let repository = config.resolved_repository();
        let auth = RegistryAuth::new(config.auth.clone());

        Ok(Self {
            client,
            auth,
            base_url,
            repository,
            page_size: config.page_size,
        })
    }

    /// List all tags for the configured repository.
    pub async fn list_tags(&self) -> Result<Vec<String>> {
        let url = format!(
            "{}/{}/tags/list?n={}",
            self.base_url, self.repository, self.page_size
        );
        tracing::debug!(url = %url, "listing registry tags");

        let body = self.authenticated_get(&url).await?;
        let tag_list: TagListResponse = serde_json::from_str(&body).map_err(|e| {
            report!(DockerRegistryError::ParseResponse(format!(
                "failed to parse tag list: {e}"
            )))
        })?;

        tracing::debug!(count = tag_list.tags.len(), "fetched registry tags");
        Ok(tag_list.tags)
    }

    /// Get the manifest digest for a specific tag.
    ///
    /// Uses HEAD request to get the `Docker-Content-Digest` header.
    pub async fn get_manifest_digest(&self, tag: &str) -> Result<String> {
        let url = format!("{}/{}/manifests/{}", self.base_url, self.repository, tag);
        tracing::debug!(url = %url, tag = %tag, "fetching manifest digest");

        let digest = self.authenticated_head(&url).await?;
        tracing::debug!(digest = %digest, tag = %tag, "fetched manifest digest");
        Ok(digest)
    }

    /// Perform an authenticated GET request with 401 retry.
    async fn authenticated_get(&self, url: &str) -> Result<String> {
        // Try with cached token first
        if let Some(token) = self.auth.cached_bearer_token() {
            tracing::trace!("using cached registry auth token");
            let response = self
                .client
                .get(url)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| DockerRegistryError::Request(format!("GET failed: {e}")))?;

            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return self.handle_response(response).await;
            }
            // Token expired or invalid, fall through to re-auth
            self.auth.clear_cache();
        }

        // First attempt without token (or after cache clear)
        let response = self
            .client
            .get(url)
            .send()
            .await
            .context_transform(|e| DockerRegistryError::Request(format!("GET failed: {e}")))?;

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
                    DockerRegistryError::Request(format!("GET retry failed: {e}"))
                })?;

            return self.handle_response(retry_response).await;
        }

        self.handle_response(response).await
    }

    /// Perform an authenticated HEAD request with 401 retry.
    /// Returns the `Docker-Content-Digest` header value.
    async fn authenticated_head(&self, url: &str) -> Result<String> {
        // Try with cached token first
        if let Some(token) = self.auth.cached_bearer_token() {
            let response = self
                .client
                .head(url)
                .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
                .bearer_auth(&token)
                .send()
                .await
                .context_transform(|e| DockerRegistryError::Request(format!("HEAD failed: {e}")))?;

            if response.status() != reqwest::StatusCode::UNAUTHORIZED {
                return self.extract_digest(response).await;
            }
            self.auth.clear_cache();
        }

        // First attempt without token
        let response = self
            .client
            .head(url)
            .header(reqwest::header::ACCEPT, MANIFEST_ACCEPT)
            .send()
            .await
            .context_transform(|e| DockerRegistryError::Request(format!("HEAD failed: {e}")))?;

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
                    DockerRegistryError::Request(format!("HEAD retry failed: {e}"))
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
            bail!(DockerRegistryError::RateLimited);
        }

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<RegistryErrorResponse>(&body)
                .ok()
                .and_then(|r| r.errors.into_iter().next())
                .map(|e| format!("{}: {}", e.code, e.message))
                .unwrap_or(body);

            bail!(DockerRegistryError::ApiError { status, message });
        }

        response.text().await.map_err(|e| {
            report!(DockerRegistryError::ParseResponse(format!(
                "failed to read response body: {e}"
            )))
        })
    }

    /// Extract the `Docker-Content-Digest` header from a HEAD response.
    async fn extract_digest(&self, response: reqwest::Response) -> Result<String> {
        let status = response.status();

        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!(DockerRegistryError::RateLimited);
        }

        if !status.is_success() {
            bail!(DockerRegistryError::ApiError {
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
                report!(DockerRegistryError::ParseResponse(
                    "missing Docker-Content-Digest header".to_string()
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DockerRegistryConfig, TrackingMode};

    fn test_config() -> DockerRegistryConfig {
        DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 100,
            restart_command: None,
        }
    }

    #[test]
    fn client_creation_succeeds() {
        let config = test_config();
        assert!(RegistryClient::new(&config).is_ok());
    }

    #[test]
    fn client_base_url_docker_hub() {
        let config = test_config();
        let client = RegistryClient::new(&config).expect("valid config");
        assert_eq!(client.base_url, "https://registry-1.docker.io/v2");
        assert_eq!(client.repository, "library/nginx");
    }

    #[test]
    fn client_base_url_ghcr() {
        let config = DockerRegistryConfig {
            image: "ghcr.io/owner/repo".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 100,
            restart_command: None,
        };
        let client = RegistryClient::new(&config).expect("valid config");
        assert_eq!(client.base_url, "https://ghcr.io/v2");
        assert_eq!(client.repository, "owner/repo");
    }

    #[test]
    fn client_base_url_private() {
        let config = DockerRegistryConfig {
            image: "registry.example.com/myapp".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 100,
            restart_command: None,
        };
        let client = RegistryClient::new(&config).expect("valid config");
        assert_eq!(client.base_url, "https://registry.example.com/v2");
        assert_eq!(client.repository, "myapp");
    }
}
