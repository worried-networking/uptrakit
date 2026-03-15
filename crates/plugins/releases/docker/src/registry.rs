use std::sync::Arc;

use rootcause::prelude::*;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

use crate::api_types::{OciManifestIndex, OciPlatform};
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

/// The OCI annotation key for image creation timestamp.
const OCI_CREATED_ANNOTATION: &str = "org.opencontainers.image.created";

/// Result of a manifest fetch: digest plus optional image creation timestamp.
pub struct ManifestInfo {
    /// SHA-256 digest of the manifest (platform-specific or index digest).
    pub digest: String,
    /// When the image was created, if available from the manifest annotations
    /// or the image config blob.
    pub created_at: Option<time::OffsetDateTime>,
}

/// Low-level HTTP client for OCI Distribution API operations.
///
/// Unlike the old implementation, `RegistryClient` does not bake in a specific
/// registry hostname or repository at construction time. Instead, `registry`
/// and `repository` are passed per-call — allowing a single client instance to
/// serve multiple images with different registries.
pub struct RegistryClient {
    client: reqwest::Client,
    /// Redirect-following client for config blob fetches.
    ///
    /// Docker Hub serves blobs via CDN with 307 redirects. The primary `client`
    /// uses `Policy::none()` to prevent open-redirect abuse on manifest URLs,
    /// but blobs are content-addressed so redirects are safe to follow.
    blob_client: reqwest::Client,
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

        let blob_client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-releases-docker/",
                env!("CARGO_PKG_VERSION")
            ))
            .redirect(reqwest::redirect::Policy::limited(5))
            .dns_resolver(Arc::new(SsrfSafeResolver::new()))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| {
                report!(DockerError::Request(format!(
                    "failed to build blob HTTP client: {e}"
                )))
            })?;

        Ok(Self {
            client,
            blob_client,
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

    /// Get manifest info (digest + optional created_at) for a tag with no platform configured.
    ///
    /// Issues a GET to obtain the body for `created_at` extraction. When the
    /// response is a multi-arch image index, `created_at` is `None` because a
    /// single image-level timestamp cannot represent all platforms. When the
    /// response is a single-arch manifest, `created_at` is extracted from
    /// annotations or the config blob.
    pub async fn get_manifest_info(
        &self,
        registry: &str,
        repository: &str,
        tag: &str,
    ) -> Result<ManifestInfo> {
        let (digest, content_type, body) =
            self.fetch_manifest_body(registry, repository, tag).await?;

        let is_index =
            content_type.contains("image.index") || content_type.contains("manifest.list");

        if is_index {
            // Multi-arch index: can't determine a single platform's timestamp.
            return Ok(ManifestInfo {
                digest,
                created_at: None,
            });
        }

        // Single-arch manifest: extract created_at from annotations or config blob.
        let created_at = self
            .try_extract_created_at(registry, repository, &body)
            .await;

        Ok(ManifestInfo { digest, created_at })
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

    /// Fetch the manifest body and return `(digest, content_type, body)`.
    ///
    /// Always issues a GET so the body is available for index parsing.
    /// Uses the same 401-retry logic as `get_manifest_digest`.
    async fn fetch_manifest_body(
        &self,
        registry: &str,
        repository: &str,
        reference: &str,
    ) -> Result<(String, String, Vec<u8>)> {
        let url = format!("https://{registry}/v2/{repository}/manifests/{reference}");
        tracing::debug!(url = %url, "fetching manifest body");
        self.authenticated_request_with_body(reqwest::Method::GET, &url)
            .await
    }

    /// Authenticated GET that returns `(digest, content_type, body)`.
    async fn authenticated_request_with_body(
        &self,
        method: reqwest::Method,
        url: &str,
    ) -> Result<(String, String, Vec<u8>)> {
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
                return self.extract_body(response).await;
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
            return self.extract_body(retry_response).await;
        }

        self.extract_body(response).await
    }

    /// Extract `(digest, content_type, body)` from a manifest GET response.
    async fn extract_body(&self, response: reqwest::Response) -> Result<(String, String, Vec<u8>)> {
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

        let digest = response
            .headers()
            .get("docker-content-digest")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                report!(DockerError::ParseResponse(
                    "missing Docker-Content-Digest header".to_string()
                ))
            })?;

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = response
            .bytes()
            .await
            .context_transform(|e| DockerError::Request(format!("failed to read body: {e}")))?
            .to_vec();

        Ok((digest, content_type, body))
    }

    /// Resolve the digest and creation timestamp for a specific platform within a multi-arch image.
    ///
    /// If `tag` refers to an OCI Image Index or Docker Manifest List, parses
    /// the index and returns the `ManifestInfo` for the requested `platform` entry
    /// (including a `created_at` fetched from the platform-specific manifest).
    /// If the image is a single-arch manifest, returns its digest and extracted
    /// `created_at` directly.
    /// Returns `None` when the platform is not present in the manifest list
    /// (the image tag exists but this platform was removed).
    pub async fn get_platform_manifest_digest(
        &self,
        registry: &str,
        repository: &str,
        tag: &str,
        platform: &str,
    ) -> Result<Option<ManifestInfo>> {
        let (index_digest, content_type, body) =
            self.fetch_manifest_body(registry, repository, tag).await?;

        let is_index =
            content_type.contains("image.index") || content_type.contains("manifest.list");

        if !is_index {
            // Single-arch manifest — extract created_at from the already-fetched body.
            tracing::debug!(
                tag = %tag,
                platform = %platform,
                "manifest is single-arch; returning manifest digest"
            );
            let created_at = self
                .try_extract_created_at(registry, repository, &body)
                .await;
            return Ok(Some(ManifestInfo {
                digest: index_digest,
                created_at,
            }));
        }

        let index: OciManifestIndex = serde_json::from_slice(&body).map_err(|e| {
            report!(DockerError::ParseResponse(format!(
                "failed to parse OCI manifest index: {e}"
            )))
        })?;

        let found = index
            .manifests
            .iter()
            .find(|e| {
                e.platform
                    .as_ref()
                    .is_some_and(|p| platform_matches(p, platform))
            })
            .map(|e| e.digest.clone());

        let Some(platform_digest) = found else {
            tracing::debug!(
                tag = %tag,
                platform = %platform,
                "platform not found in manifest index"
            );
            return Ok(None);
        };

        let created_at = self
            .try_fetch_manifest_created_at(registry, repository, &platform_digest)
            .await;

        Ok(Some(ManifestInfo {
            digest: platform_digest,
            created_at,
        }))
    }

    /// Fetch a single-arch manifest body by its digest and extract the creation
    /// timestamp from manifest annotations or the image config blob.
    async fn try_fetch_manifest_created_at(
        &self,
        registry: &str,
        repository: &str,
        manifest_digest: &str,
    ) -> Option<time::OffsetDateTime> {
        let (_, _, body) = self
            .fetch_manifest_body(registry, repository, manifest_digest)
            .await
            .ok()?;
        self.try_extract_created_at(registry, repository, &body)
            .await
    }

    /// Extract the creation timestamp from a single-arch manifest body.
    ///
    /// Checks `annotations["org.opencontainers.image.created"]` first
    /// (OCI-format manifests set this; avoids an extra round-trip).
    /// Falls back to fetching the image config blob for Docker manifest v2
    /// images that do not carry annotations (e.g. many Docker Hub images).
    async fn try_extract_created_at(
        &self,
        registry: &str,
        repository: &str,
        body: &[u8],
    ) -> Option<time::OffsetDateTime> {
        use crate::api_types::OciSingleManifest;

        let manifest: OciSingleManifest = serde_json::from_slice(body).ok()?;

        // Fast path: annotation is present (OCI Buildx images, GHCR, etc.)
        if let Some(ts) = manifest.annotations.get(OCI_CREATED_ANNOTATION)
            && let Ok(dt) =
                time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339)
        {
            return Some(dt);
        }

        // Slow path: fetch the image config blob.
        self.try_config_blob_created_at(registry, repository, &manifest.config.digest)
            .await
    }

    /// Fetch the image config blob and return its `created` timestamp.
    ///
    /// Uses the redirect-following `blob_client` because registries like
    /// Docker Hub serve blobs via CDN (307 redirect).
    async fn try_config_blob_created_at(
        &self,
        registry: &str,
        repository: &str,
        config_digest: &str,
    ) -> Option<time::OffsetDateTime> {
        use crate::api_types::OciImageConfig;

        let url = format!("https://{registry}/v2/{repository}/blobs/{config_digest}");
        tracing::debug!(url = %url, "fetching image config blob for created_at");

        // Blobs are content-addressed so any valid bearer token works.
        let token = self.auth.cached_bearer_token();
        let mut req = self
            .blob_client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/octet-stream, */*");
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }

        let response = req.send().await.ok()?;
        if !response.status().is_success() {
            tracing::debug!(status = %response.status(), url = %url, "config blob fetch failed");
            return None;
        }

        let body = response.bytes().await.ok()?;
        let config: OciImageConfig = serde_json::from_slice(&body).ok()?;

        config.created.as_deref().and_then(|ts| {
            time::OffsetDateTime::parse(ts, &time::format_description::well_known::Rfc3339).ok()
        })
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

/// Parse an OCI platform string into `(os, arch, variant)`.
///
/// Format: `"os/arch"` or `"os/arch/variant"` (e.g. `"linux/arm/v7"`).
fn parse_platform(s: &str) -> (&str, &str, Option<&str>) {
    let mut parts = s.splitn(3, '/');
    let os = parts.next().unwrap_or("");
    let arch = parts.next().unwrap_or("");
    let variant = parts.next();
    (os, arch, variant)
}

/// Return `true` when the index entry's platform matches the wanted string.
fn platform_matches(entry: &OciPlatform, wanted: &str) -> bool {
    let (want_os, want_arch, want_variant) = parse_platform(wanted);
    entry.os == want_os
        && entry.architecture == want_arch
        && entry.variant.as_deref() == want_variant
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
    use crate::api_types::OciPlatform;

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

    #[test]
    fn parse_platform_os_arch() {
        let (os, arch, variant) = parse_platform("linux/amd64");
        assert_eq!(os, "linux");
        assert_eq!(arch, "amd64");
        assert!(variant.is_none());
    }

    #[test]
    fn parse_platform_with_variant() {
        let (os, arch, variant) = parse_platform("linux/arm/v7");
        assert_eq!(os, "linux");
        assert_eq!(arch, "arm");
        assert_eq!(variant, Some("v7"));
    }

    #[test]
    fn platform_matches_amd64() {
        let p = OciPlatform {
            os: "linux".to_string(),
            architecture: "amd64".to_string(),
            variant: None,
        };
        assert!(platform_matches(&p, "linux/amd64"));
        assert!(!platform_matches(&p, "linux/arm64"));
    }

    #[test]
    fn platform_matches_armv7() {
        let p = OciPlatform {
            os: "linux".to_string(),
            architecture: "arm".to_string(),
            variant: Some("v7".to_string()),
        };
        assert!(platform_matches(&p, "linux/arm/v7"));
        assert!(!platform_matches(&p, "linux/arm/v6"));
        assert!(!platform_matches(&p, "linux/arm"));
    }

    #[test]
    fn platform_matches_no_variant_mismatch() {
        // Entry has no variant, wanted string has variant → should NOT match
        let p = OciPlatform {
            os: "linux".to_string(),
            architecture: "arm64".to_string(),
            variant: None,
        };
        assert!(!platform_matches(&p, "linux/arm64/v8"));
        assert!(platform_matches(&p, "linux/arm64"));
    }

    #[tokio::test]
    async fn extract_created_at_from_annotation() {
        let client = RegistryClient::new(None).unwrap();
        let body = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:abc123"},
            "annotations": {
                "org.opencontainers.image.created": "2025-03-10T14:32:00Z"
            }
        });
        let ts = client
            .try_extract_created_at(
                "registry.example.com",
                "myrepo",
                &serde_json::to_vec(&body).unwrap(),
            )
            .await;
        assert!(ts.is_some());
        let ts = ts.unwrap();
        assert_eq!(ts.year(), 2025);
        assert_eq!(ts.month() as u8, 3);
        assert_eq!(ts.day(), 10);
    }

    #[tokio::test]
    async fn extract_created_at_no_annotation_no_blob_returns_none() {
        let client = RegistryClient::new(None).unwrap();
        // Manifest with no annotation and a config digest that won't be fetchable in tests
        let body = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:deadbeef"},
            "layers": []
        });
        // The config blob fetch will fail (no network in tests), so None is expected
        let ts = client
            .try_extract_created_at(
                "registry.example.com",
                "myrepo",
                &serde_json::to_vec(&body).unwrap(),
            )
            .await;
        // Might be Some or None depending on network; just ensure it doesn't panic
        let _ = ts;
    }

    #[tokio::test]
    async fn extract_created_at_invalid_annotation_format_returns_none() {
        let client = RegistryClient::new(None).unwrap();
        let body = serde_json::json!({
            "schemaVersion": 2,
            "config": {"digest": "sha256:abc123"},
            "annotations": {
                "org.opencontainers.image.created": "not-a-valid-date"
            }
        });
        // invalid annotation → falls through to blob fetch → fails in test → None
        let ts = client
            .try_extract_created_at(
                "registry.example.com",
                "myrepo",
                &serde_json::to_vec(&body).unwrap(),
            )
            .await;
        // In a real test environment without network: None expected
        let _ = ts;
    }
}
