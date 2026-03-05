use std::collections::HashMap;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use uptrakit_plugin_infrastructure_core::{
    AttestationStatus, Plugin, PluginCapability, PluginError, PluginType, ReleaseAsset,
    UpstreamRelease, Version,
};

use crate::api_types::{AttestationsApiResponse, GitHubApiError, GitHubAsset, GitHubRelease};
use crate::config::GitHubConfig;
use crate::error::{GitHubError, Result};
use crate::tag::strip_tag_prefix;

/// Parse `"owner/repo"` from a package identifier string.
///
/// Rules:
/// - Must contain exactly one `/`.
/// - Both `owner` and `repo` parts must be non-empty.
/// - Neither part may contain `..` (path traversal guard).
pub fn parse_owner_repo(package_identifier: &str) -> Result<(&str, &str)> {
    let Some((owner, repo)) = package_identifier.split_once('/') else {
        bail!(GitHubError::Configuration(format!(
            "package_identifier must be 'owner/repo' (got '{package_identifier}')"
        )));
    };
    if repo.contains('/') {
        bail!(GitHubError::Configuration(format!(
            "package_identifier must be 'owner/repo' (got '{package_identifier}')"
        )));
    }
    if owner.is_empty() {
        bail!(GitHubError::Configuration(
            "package_identifier owner must not be empty".to_string()
        ));
    }
    if repo.is_empty() {
        bail!(GitHubError::Configuration(
            "package_identifier repo must not be empty".to_string()
        ));
    }
    if owner.contains("..") {
        bail!(GitHubError::Configuration(format!(
            "package_identifier owner must not contain '..': '{owner}'"
        )));
    }
    if repo.contains("..") {
        bail!(GitHubError::Configuration(format!(
            "package_identifier repo must not contain '..': '{repo}'"
        )));
    }
    Ok((owner, repo))
}

/// GitHub Releases plugin implementation.
///
/// Fetches release metadata from the GitHub API and converts it into
/// `UpstreamRelease` values for the controller.
///
/// The `owner` and `repo` are parsed from the `package_identifier` argument
/// at call time (format: `"owner/repo"`), not stored in the plugin config.
/// A single plugin instance can therefore serve any number of GitHub repositories.
pub struct GitHubPlugin {
    client: reqwest::Client,
    config: GitHubConfig,
    asset_filters: Vec<Regex>,
}

impl GitHubPlugin {
    /// Compile-time capabilities for the GitHub plugin.
    pub const CAPABILITIES: &'static [PluginCapability] =
        &[PluginCapability::ControllerSideFetchReleases];

    /// Create a new `GitHubPlugin` from the given configuration.
    ///
    /// Validates the configuration and pre-compiles asset filter regexes.
    /// The `_executor` parameter is accepted for registry compatibility but unused
    /// (this plugin is controller-side only).
    pub async fn new(
        config: GitHubConfig,
        _executor: std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor>,
    ) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(GitHubError::Configuration(e.to_string())))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
        );
        headers.insert(
            "X-GitHub-Api-Version",
            reqwest::header::HeaderValue::from_static("2022-11-28"),
        );

        if let Some(ref token) = config.auth_token {
            let value = format!("Bearer {}", token.expose_secret());
            let header_value = reqwest::header::HeaderValue::from_str(&value).map_err(|e| {
                report!(GitHubError::Configuration(format!(
                    "invalid auth token header value: {e}"
                )))
            })?;
            headers.insert(reqwest::header::AUTHORIZATION, header_value);
        }

        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-releases-github/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| {
                report!(GitHubError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        let asset_filters: Vec<Regex> = config
            .asset_patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|e| {
                    report!(GitHubError::InvalidPattern(format!(
                        "invalid regex '{p}': {e}"
                    )))
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            client,
            config,
            asset_filters,
        })
    }

    /// Build the releases API URL for the given owner/repo pair.
    pub(crate) fn releases_url(&self, owner: &str, repo: &str) -> String {
        format!(
            "{}/repos/{}/{}/releases?per_page=100",
            self.config.api_base_url(),
            owner,
            repo,
        )
    }

    /// Convert a GitHub API release to an `UpstreamRelease`, applying filters.
    ///
    /// Returns `None` if the release should be skipped (draft, filtered prerelease).
    fn convert_release(&self, gh_release: &GitHubRelease) -> Option<UpstreamRelease> {
        // Skip drafts
        if gh_release.draft {
            tracing::trace!(tag = %gh_release.tag_name, "skipping draft release");
            return None;
        }

        // Skip prereleases unless configured to include them
        if gh_release.prerelease && !self.config.include_prereleases {
            tracing::trace!(tag = %gh_release.tag_name, "skipping prerelease");
            return None;
        }

        let version_str = strip_tag_prefix(&gh_release.tag_name, &self.config.tag_strip_prefix);
        let version = Version::new(version_str);

        let published_at = gh_release.published_at.as_ref().and_then(|s| {
            OffsetDateTime::parse(s, &Rfc3339)
                .inspect_err(|e| {
                    tracing::warn!(
                        tag = %gh_release.tag_name,
                        error = %e,
                        "failed to parse published_at date"
                    );
                })
                .ok()
        });

        let assets: Vec<ReleaseAsset> = gh_release
            .assets
            .iter()
            .filter(|a| {
                if self.asset_filters.is_empty() {
                    return true;
                }
                self.asset_filters.iter().any(|re| re.is_match(&a.name))
            })
            .map(|a| ReleaseAsset {
                name: a.name.clone(),
                download_url: a.browser_download_url.clone(),
                size: Some(a.size),
                content_type: a.content_type.clone(),
                sha256_digest: None,
            })
            .collect();

        Some(UpstreamRelease {
            version,
            tag: gh_release.tag_name.clone(),
            is_prerelease: gh_release.prerelease,
            release_url: gh_release.html_url.clone(),
            release_notes: gh_release.body.clone(),
            published_at,
            assets,
            category: None,
            attestation_status: None,
        })
    }

    /// Build the attestations API URL for the given owner/repo/digest.
    ///
    /// Produces `{api_base_url}/repos/{owner}/{repo}/attestations/sha256:{hex}`.
    pub(crate) fn attestation_url(&self, owner: &str, repo: &str, digest_hex: &str) -> String {
        format!(
            "{}/repos/{}/{}/attestations/sha256:{}",
            self.config.api_base_url(),
            owner,
            repo,
            digest_hex,
        )
    }

    /// Find the checksums asset in a list of raw GitHub assets.
    ///
    /// Returns the first asset whose name (case-insensitive) contains `"sha256"`
    /// or `"checksum"`.
    pub(crate) fn find_checksums_asset(assets: &[GitHubAsset]) -> Option<&GitHubAsset> {
        assets.iter().find(|a| {
            let lower = a.name.to_lowercase();
            lower.contains("sha256") || lower.contains("checksum")
        })
    }

    /// Parse a checksums file into a `{filename → sha256_hex}` map.
    ///
    /// Accepts lines in both `<hex>  <filename>` (text mode) and
    /// `<hex> *<filename>` (binary mode) formats.  Lines with an invalid
    /// (non-64-hex-char) digest are silently skipped.
    pub(crate) fn parse_checksums_content(content: &str) -> HashMap<String, String> {
        let mut map = HashMap::new();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Try "hex  filename" first (two spaces), then "hex *filename".
            let (hex, filename) = if let Some((h, f)) = line.split_once("  ") {
                (h, f.trim_start_matches('*'))
            } else if let Some((h, f)) = line.split_once(" *") {
                (h, f)
            } else {
                continue;
            };
            let hex = hex.trim();
            let filename = filename.trim();
            if hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
                map.insert(filename.to_string(), hex.to_string());
            }
        }
        map
    }

    /// Check the GitHub Attestations API for the first eligible release asset.
    ///
    /// 1. Finds the checksums file in `gh_assets` (raw, pre-filter GitHub assets).
    /// 2. Downloads it and parses `{filename → sha256}`.
    /// 3. Sets `sha256_digest` on each matching entry in `release_assets`.
    /// 4. Queries the attestation API for the first asset with a known digest.
    ///
    /// Returns:
    /// - `Verified` — API returned ≥1 attestation.
    /// - `NotFound` — API returned 0 attestations (404 or empty array).
    /// - `Unverified` — checksums file absent, download error, or HTTP error.
    async fn check_release_attestation(
        &self,
        owner: &str,
        repo: &str,
        gh_assets: &[GitHubAsset],
        release_assets: &mut [ReleaseAsset],
    ) -> AttestationStatus {
        // 1. Locate checksums file in raw GitHub assets.
        let checksums_asset = match Self::find_checksums_asset(gh_assets) {
            Some(a) => a,
            None => {
                tracing::debug!(owner, repo, "no checksums file found; skipping attestation");
                return AttestationStatus::Unverified;
            }
        };

        // 2. Download the checksums file.
        let checksums_text = match self
            .client
            .get(&checksums_asset.browser_download_url)
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.text().await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read checksums file body");
                    return AttestationStatus::Unverified;
                }
            },
            Ok(resp) => {
                tracing::warn!(
                    status = %resp.status(),
                    url = %checksums_asset.browser_download_url,
                    "checksums file download returned non-success status"
                );
                return AttestationStatus::Unverified;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to download checksums file");
                return AttestationStatus::Unverified;
            }
        };

        // 3. Parse checksums and set sha256_digest on matching release_assets.
        let digests = Self::parse_checksums_content(&checksums_text);
        for asset in release_assets.iter_mut() {
            if let Some(hex) = digests.get(&asset.name) {
                asset.sha256_digest = Some(hex.clone());
            }
        }

        // 4. Take first asset with a known digest and query the attestation API.
        let Some(digest_hex) = release_assets
            .iter()
            .find_map(|a| a.sha256_digest.as_deref())
        else {
            tracing::debug!(
                owner,
                repo,
                "no matching digest found in checksums; cannot verify"
            );
            return AttestationStatus::Unverified;
        };

        let url = self.attestation_url(owner, repo, digest_hex);
        tracing::debug!(%url, "checking GitHub attestation");

        let response = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(error = %e, "attestation API request failed");
                return AttestationStatus::Unverified;
            }
        };

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return AttestationStatus::NotFound;
        }
        if !status.is_success() {
            tracing::warn!(%status, "attestation API returned error");
            return AttestationStatus::Unverified;
        }

        match response.json::<AttestationsApiResponse>().await {
            Ok(body) if !body.attestations.is_empty() => AttestationStatus::Verified,
            Ok(_) => AttestationStatus::NotFound,
            Err(e) => {
                tracing::warn!(error = %e, "failed to parse attestation API response");
                AttestationStatus::Unverified
            }
        }
    }

    /// Check rate limit headers and log warnings.
    fn check_rate_limit(&self, headers: &reqwest::header::HeaderMap, package_identifier: &str) {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        if let Some(remaining) = remaining
            && remaining < 10
        {
            let reset = headers
                .get("x-ratelimit-reset")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            tracing::warn!(
                remaining,
                reset,
                package_identifier,
                "GitHub API rate limit is low"
            );
        }
    }
}

#[async_trait]
impl Plugin for GitHubPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::ReleasesGithub
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }

    #[tracing::instrument(skip_all)]
    async fn fetch_releases(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<UpstreamRelease>> {
        let (owner, repo) = parse_owner_repo(package_identifier).map_err(|e| {
            report!(PluginError::Configuration(format!(
                "invalid package_identifier for GitHub plugin: {e}"
            )))
        })?;

        let url = self.releases_url(owner, repo);
        tracing::debug!(url = %url, "fetching GitHub releases");

        let response = self.client.get(&url).send().await.map_err(|e| {
            report!(PluginError::Configuration(format!(
                "HTTP request failed: {e}"
            )))
        })?;

        let status = response.status();
        self.check_rate_limit(response.headers(), package_identifier);

        if !status.is_success() {
            tracing::debug!(status = %status, "GitHub API returned error status");
            // Check for rate limiting
            if status == reqwest::StatusCode::FORBIDDEN
                || status == reqwest::StatusCode::TOO_MANY_REQUESTS
            {
                let remaining = response
                    .headers()
                    .get("x-ratelimit-remaining")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok());

                if remaining == Some(0) {
                    let reset_at = response
                        .headers()
                        .get("x-ratelimit-reset")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("unknown")
                        .to_string();
                    bail!(PluginError::Configuration(format!(
                        "GitHub API rate limit exceeded (resets at {reset_at})"
                    )));
                }
            }

            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<GitHubApiError>(&body)
                .map(|e| e.message)
                .unwrap_or(body);

            return Err(report!(GitHubError::ApiError { status, message })).context_to();
        }

        let releases: Vec<GitHubRelease> = response.json().await.map_err(|e| {
            report!(PluginError::Serialization(format!(
                "failed to parse GitHub API response: {e}"
            )))
        })?;

        let mut upstream_releases: Vec<UpstreamRelease> = releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            total = releases.len(),
            "fetched GitHub releases"
        );

        // Attestation check for the latest (first) release.
        if self.config.verify_attestation
            && let Some(latest) = upstream_releases.first_mut()
            && let Some(gh_release) = releases.iter().find(|r| r.tag_name == latest.tag)
        {
            let status = self
                .check_release_attestation(owner, repo, &gh_release.assets, &mut latest.assets)
                .await;
            tracing::debug!(tag = %latest.tag, ?status, "attestation check complete");
            latest.attestation_status = Some(status);
        }

        Ok(upstream_releases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::{GitHubAsset, GitHubRelease};
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

    fn test_config() -> GitHubConfig {
        GitHubConfig::default()
    }

    fn test_executor() -> std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor> {
        std::sync::Arc::new(LocalCommandExecutor)
    }

    async fn test_plugin() -> GitHubPlugin {
        GitHubPlugin::new(test_config(), test_executor())
            .await
            .expect("valid config")
    }

    fn make_release(tag: &str, draft: bool, prerelease: bool) -> GitHubRelease {
        GitHubRelease {
            tag_name: tag.to_string(),
            name: Some(format!("Release {tag}")),
            draft,
            prerelease,
            html_url: format!("https://github.com/octocat/hello-world/releases/tag/{tag}"),
            body: Some("Release notes".to_string()),
            published_at: Some("2024-01-28T12:00:00Z".to_string()),
            assets: vec![],
        }
    }

    // ── parse_owner_repo tests ────────────────────────────────────────────────

    #[test]
    fn parse_owner_repo_valid() {
        let (owner, repo) = parse_owner_repo("octocat/hello-world").expect("valid");
        assert_eq!(owner, "octocat");
        assert_eq!(repo, "hello-world");
    }

    #[test]
    fn parse_owner_repo_missing_slash() {
        assert!(parse_owner_repo("octocat").is_err());
    }

    #[test]
    fn parse_owner_repo_two_slashes() {
        assert!(parse_owner_repo("octocat/hello/world").is_err());
    }

    #[test]
    fn parse_owner_repo_empty_owner() {
        assert!(parse_owner_repo("/hello-world").is_err());
    }

    #[test]
    fn parse_owner_repo_empty_repo() {
        assert!(parse_owner_repo("octocat/").is_err());
    }

    #[test]
    fn parse_owner_repo_traversal_in_owner() {
        assert!(parse_owner_repo("../evil/repo").is_err());
    }

    #[test]
    fn parse_owner_repo_traversal_in_repo() {
        assert!(parse_owner_repo("octocat/../evil").is_err());
    }

    // ── URL construction tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn url_construction() {
        let plugin = test_plugin().await;
        let url = plugin.releases_url("octocat", "hello-world");
        assert_eq!(
            url,
            "https://api.github.com/repos/octocat/hello-world/releases?per_page=100"
        );
    }

    #[tokio::test]
    async fn url_construction_custom_base() {
        let config = GitHubConfig {
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let url = plugin.releases_url("octocat", "hello-world");
        assert_eq!(
            url,
            "https://ghe.corp.com/api/v3/repos/octocat/hello-world/releases?per_page=100"
        );
    }

    // ── convert_release tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn convert_normal_release() {
        let plugin = test_plugin().await;
        let gh = make_release("v1.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "1.0.0");
        assert_eq!(release.tag, "v1.0.0");
        assert!(!release.is_prerelease);
        assert!(release.published_at.is_some());
    }

    #[tokio::test]
    async fn skip_draft_release() {
        let plugin = test_plugin().await;
        let gh = make_release("v1.0.0", true, false);
        assert!(plugin.convert_release(&gh).is_none());
    }

    #[tokio::test]
    async fn skip_prerelease_by_default() {
        let plugin = test_plugin().await;
        let gh = make_release("v1.0.0-beta.1", false, true);
        assert!(plugin.convert_release(&gh).is_none());
    }

    #[tokio::test]
    async fn include_prerelease_when_configured() {
        let config = GitHubConfig {
            include_prereleases: true,
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let gh = make_release("v1.0.0-beta.1", false, true);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert!(release.is_prerelease);
        assert_eq!(release.version.as_str(), "1.0.0-beta.1");
    }

    #[tokio::test]
    async fn tag_stripping() {
        let plugin = test_plugin().await;
        let gh = make_release("v2.3.4", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "2.3.4");
    }

    #[tokio::test]
    async fn tag_without_prefix() {
        let plugin = test_plugin().await;
        let gh = make_release("1.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "1.0.0");
    }

    #[tokio::test]
    async fn custom_tag_prefix() {
        let config = GitHubConfig {
            tag_strip_prefix: "release-".to_string(),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let gh = make_release("release-3.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "3.0.0");
    }

    #[tokio::test]
    async fn asset_filtering() {
        let config = GitHubConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string()],
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor())
            .await
            .expect("valid config");

        let gh = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://example.com".to_string(),
            body: None,
            published_at: None,
            assets: vec![
                GitHubAsset {
                    name: "app-linux-amd64.tar.gz".to_string(),
                    browser_download_url: "https://example.com/app.tar.gz".to_string(),
                    size: 1000,
                    content_type: Some("application/gzip".to_string()),
                },
                GitHubAsset {
                    name: "app-linux-amd64.deb".to_string(),
                    browser_download_url: "https://example.com/app.deb".to_string(),
                    size: 2000,
                    content_type: Some("application/vnd.debian.binary-package".to_string()),
                },
                GitHubAsset {
                    name: "checksums.txt".to_string(),
                    browser_download_url: "https://example.com/checksums.txt".to_string(),
                    size: 256,
                    content_type: Some("text/plain".to_string()),
                },
            ],
        };

        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.assets.len(), 1);
        assert_eq!(release.assets[0].name, "app-linux-amd64.tar.gz");
    }

    #[tokio::test]
    async fn no_asset_filter_includes_all() {
        let plugin = test_plugin().await;
        let gh = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://example.com".to_string(),
            body: None,
            published_at: None,
            assets: vec![
                GitHubAsset {
                    name: "a.tar.gz".to_string(),
                    browser_download_url: "https://example.com/a".to_string(),
                    size: 100,
                    content_type: None,
                },
                GitHubAsset {
                    name: "b.deb".to_string(),
                    browser_download_url: "https://example.com/b".to_string(),
                    size: 200,
                    content_type: None,
                },
            ],
        };

        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.assets.len(), 2);
    }

    #[tokio::test]
    async fn date_parsing() {
        let plugin = test_plugin().await;
        let gh = make_release("v1.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        let published = release.published_at.expect("should have published_at");
        assert_eq!(published.year(), 2024);
        assert_eq!(published.month() as u8, 1);
        assert_eq!(published.day(), 28);
    }

    #[tokio::test]
    async fn invalid_date_does_not_fail() {
        let plugin = test_plugin().await;
        let gh = GitHubRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://example.com".to_string(),
            body: None,
            published_at: Some("not-a-date".to_string()),
            assets: vec![],
        };
        let release = plugin.convert_release(&gh).expect("should convert");
        assert!(release.published_at.is_none());
    }

    #[tokio::test]
    async fn plugin_creation_succeeds_with_empty_config() {
        let config = GitHubConfig::default();
        assert!(GitHubPlugin::new(config, test_executor()).await.is_ok());
    }

    // ── find_checksums_asset tests ────────────────────────────────────────

    #[test]
    fn find_checksums_asset_finds_sha256() {
        let assets = vec![
            GitHubAsset {
                name: "app.tar.gz".to_string(),
                browser_download_url: "https://example.com/app".to_string(),
                size: 100,
                content_type: None,
            },
            GitHubAsset {
                name: "SHA256SUMS".to_string(),
                browser_download_url: "https://example.com/sums".to_string(),
                size: 128,
                content_type: None,
            },
        ];
        let found = GitHubPlugin::find_checksums_asset(&assets).expect("should find");
        assert_eq!(found.name, "SHA256SUMS");
    }

    #[test]
    fn find_checksums_asset_finds_checksum() {
        let assets = vec![GitHubAsset {
            name: "checksums.txt".to_string(),
            browser_download_url: "https://example.com/checksums.txt".to_string(),
            size: 64,
            content_type: None,
        }];
        let found = GitHubPlugin::find_checksums_asset(&assets).expect("should find");
        assert_eq!(found.name, "checksums.txt");
    }

    #[test]
    fn find_checksums_asset_returns_none_when_absent() {
        let assets = vec![GitHubAsset {
            name: "app.tar.gz".to_string(),
            browser_download_url: "https://example.com/app".to_string(),
            size: 100,
            content_type: None,
        }];
        assert!(GitHubPlugin::find_checksums_asset(&assets).is_none());
    }

    #[test]
    fn find_checksums_asset_empty_list() {
        assert!(GitHubPlugin::find_checksums_asset(&[]).is_none());
    }

    // ── parse_checksums_content tests ─────────────────────────────────────

    #[test]
    fn parse_checksums_two_space_format() {
        let content = format!("{}  app.tar.gz\n", "a".repeat(64));
        let map = GitHubPlugin::parse_checksums_content(&content);
        assert_eq!(
            map.get("app.tar.gz").map(String::as_str),
            Some("a".repeat(64).as_str())
        );
    }

    #[test]
    fn parse_checksums_star_format() {
        let content = format!("{} *app.tar.gz\n", "b".repeat(64));
        let map = GitHubPlugin::parse_checksums_content(&content);
        assert_eq!(
            map.get("app.tar.gz").map(String::as_str),
            Some("b".repeat(64).as_str())
        );
    }

    #[test]
    fn parse_checksums_multiple_entries() {
        let hex1 = "a".repeat(64);
        let hex2 = "b".repeat(64);
        let content = format!("{hex1}  file1.tar.gz\n{hex2}  file2.deb\n");
        let map = GitHubPlugin::parse_checksums_content(&content);
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("file1.tar.gz").map(String::as_str),
            Some(hex1.as_str())
        );
        assert_eq!(
            map.get("file2.deb").map(String::as_str),
            Some(hex2.as_str())
        );
    }

    #[test]
    fn parse_checksums_skips_short_hex() {
        let content = "abc123  file.tar.gz\n";
        let map = GitHubPlugin::parse_checksums_content(content);
        assert!(map.is_empty());
    }

    #[test]
    fn parse_checksums_skips_comments() {
        let hex = "c".repeat(64);
        let content = format!("# comment\n{hex}  file.tar.gz\n");
        let map = GitHubPlugin::parse_checksums_content(&content);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn parse_checksums_empty_returns_empty_map() {
        assert!(GitHubPlugin::parse_checksums_content("").is_empty());
    }

    // ── attestation_url tests ─────────────────────────────────────────────

    #[tokio::test]
    async fn attestation_url_default_base() {
        let plugin = test_plugin().await;
        let url = plugin.attestation_url("owner", "repo", "abc123");
        assert_eq!(
            url,
            "https://api.github.com/repos/owner/repo/attestations/sha256:abc123"
        );
    }

    #[tokio::test]
    async fn attestation_url_custom_base() {
        let config = GitHubConfig {
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let url = plugin.attestation_url("org", "project", "deadbeef");
        assert_eq!(
            url,
            "https://ghe.corp.com/api/v3/repos/org/project/attestations/sha256:deadbeef"
        );
    }
}
