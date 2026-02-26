use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use uptrakit_plugin_core::{
    Plugin, PluginCapability, PluginError, PluginType, ReleaseAsset, UpstreamRelease, Version,
};

use crate::api_types::{GitHubApiError, GitHubRelease};
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
    let slash_count = package_identifier.chars().filter(|&c| c == '/').count();
    if slash_count != 1 {
        bail!(GitHubError::Configuration(format!(
            "package_identifier must be 'owner/repo' (got '{package_identifier}')"
        )));
    }
    let slash = package_identifier.find('/').unwrap();
    let owner = &package_identifier[..slash];
    let repo = &package_identifier[slash + 1..];
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
    /// Create a new `GitHubPlugin` from the given configuration.
    ///
    /// Validates the configuration and pre-compiles asset filter regexes.
    /// The `_executor` parameter is accepted for registry compatibility but unused
    /// (this plugin is controller-side only).
    pub fn new(
        config: GitHubConfig,
        _executor: std::sync::Arc<dyn uptrakit_plugin_core::CommandExecutor>,
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
                "uptrakit-plugin-github/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
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
        })
    }

    /// Check rate limit headers and log warnings.
    fn check_rate_limit(
        &self,
        headers: &reqwest::header::HeaderMap,
        package_identifier: &str,
    ) {
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
        &[PluginCapability::ControllerSideFetchReleases]
    }

    async fn fetch_releases(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_core::Result<Vec<UpstreamRelease>> {
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

            bail!(PluginError::Configuration(format!(
                "GitHub API error: {status} {message}"
            )));
        }

        let releases: Vec<GitHubRelease> = response.json().await.map_err(|e| {
            report!(PluginError::Serialization(format!(
                "failed to parse GitHub API response: {e}"
            )))
        })?;

        let upstream_releases: Vec<UpstreamRelease> = releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            total = releases.len(),
            "fetched GitHub releases"
        );

        Ok(upstream_releases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::{GitHubAsset, GitHubRelease};
    use uptrakit_plugin_core::LocalCommandExecutor;

    fn test_config() -> GitHubConfig {
        GitHubConfig::default()
    }

    fn test_executor() -> std::sync::Arc<dyn uptrakit_plugin_core::CommandExecutor> {
        std::sync::Arc::new(LocalCommandExecutor)
    }

    fn test_plugin() -> GitHubPlugin {
        GitHubPlugin::new(test_config(), test_executor()).expect("valid config")
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

    #[test]
    fn url_construction() {
        let plugin = test_plugin();
        let url = plugin.releases_url("octocat", "hello-world");
        assert_eq!(
            url,
            "https://api.github.com/repos/octocat/hello-world/releases?per_page=100"
        );
    }

    #[test]
    fn url_construction_custom_base() {
        let config = GitHubConfig {
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor()).expect("valid config");
        let url = plugin.releases_url("octocat", "hello-world");
        assert_eq!(
            url,
            "https://ghe.corp.com/api/v3/repos/octocat/hello-world/releases?per_page=100"
        );
    }

    // ── convert_release tests ─────────────────────────────────────────────────

    #[test]
    fn convert_normal_release() {
        let plugin = test_plugin();
        let gh = make_release("v1.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "1.0.0");
        assert_eq!(release.tag, "v1.0.0");
        assert!(!release.is_prerelease);
        assert!(release.published_at.is_some());
    }

    #[test]
    fn skip_draft_release() {
        let plugin = test_plugin();
        let gh = make_release("v1.0.0", true, false);
        assert!(plugin.convert_release(&gh).is_none());
    }

    #[test]
    fn skip_prerelease_by_default() {
        let plugin = test_plugin();
        let gh = make_release("v1.0.0-beta.1", false, true);
        assert!(plugin.convert_release(&gh).is_none());
    }

    #[test]
    fn include_prerelease_when_configured() {
        let config = GitHubConfig {
            include_prereleases: true,
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor()).expect("valid config");
        let gh = make_release("v1.0.0-beta.1", false, true);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert!(release.is_prerelease);
        assert_eq!(release.version.as_str(), "1.0.0-beta.1");
    }

    #[test]
    fn tag_stripping() {
        let plugin = test_plugin();
        let gh = make_release("v2.3.4", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "2.3.4");
    }

    #[test]
    fn tag_without_prefix() {
        let plugin = test_plugin();
        let gh = make_release("1.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "1.0.0");
    }

    #[test]
    fn custom_tag_prefix() {
        let config = GitHubConfig {
            tag_strip_prefix: "release-".to_string(),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor()).expect("valid config");
        let gh = make_release("release-3.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "3.0.0");
    }

    #[test]
    fn asset_filtering() {
        let config = GitHubConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string()],
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_executor()).expect("valid config");

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

    #[test]
    fn no_asset_filter_includes_all() {
        let plugin = test_plugin();
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

    #[test]
    fn date_parsing() {
        let plugin = test_plugin();
        let gh = make_release("v1.0.0", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        let published = release.published_at.expect("should have published_at");
        assert_eq!(published.year(), 2024);
        assert_eq!(published.month() as u8, 1);
        assert_eq!(published.day(), 28);
    }

    #[test]
    fn invalid_date_does_not_fail() {
        let plugin = test_plugin();
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

    #[test]
    fn plugin_creation_succeeds_with_empty_config() {
        let config = GitHubConfig::default();
        assert!(GitHubPlugin::new(config, test_executor()).is_ok());
    }
}
