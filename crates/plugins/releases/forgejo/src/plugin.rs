use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

use uptrakit_plugin_infrastructure_core::{
    Plugin, PluginCapability, PluginError, PluginType, ReleaseAsset, UpstreamRelease, Version,
};

use crate::api_types::{ForgejoApiError, ForgejoRelease};
use crate::config::ForgejoConfig;
use crate::error::{ForgejoError, Result};
use crate::tag::strip_tag_prefix;

/// Parse `"owner/repo"` from a package identifier string.
///
/// Rules:
/// - Must contain exactly one `/`.
/// - Both `owner` and `repo` parts must be non-empty.
/// - Neither part may contain `..` (path traversal guard).
pub fn parse_owner_repo(package_identifier: &str) -> Result<(&str, &str)> {
    let Some((owner, repo)) = package_identifier.split_once('/') else {
        bail!(ForgejoError::Configuration(format!(
            "package_identifier must be 'owner/repo' (got '{package_identifier}')"
        )));
    };
    if repo.contains('/') {
        bail!(ForgejoError::Configuration(format!(
            "package_identifier must be 'owner/repo' (got '{package_identifier}')"
        )));
    }
    if owner.is_empty() {
        bail!(ForgejoError::Configuration(
            "package_identifier owner must not be empty".to_string()
        ));
    }
    if repo.is_empty() {
        bail!(ForgejoError::Configuration(
            "package_identifier repo must not be empty".to_string()
        ));
    }
    if owner.contains("..") {
        bail!(ForgejoError::Configuration(format!(
            "package_identifier owner must not contain '..': '{owner}'"
        )));
    }
    if repo.contains("..") {
        bail!(ForgejoError::Configuration(format!(
            "package_identifier repo must not contain '..': '{repo}'"
        )));
    }
    Ok((owner, repo))
}

/// Forgejo Releases plugin implementation.
///
/// Fetches release metadata from the Forgejo API and converts
/// it into `UpstreamRelease` values for the controller.
///
/// The Forgejo API is nearly identical to GitHub's, so this plugin also works
/// with any self-hosted Forgejo or Gitea instance via `api_base_url`.
///
/// The `owner` and `repo` are parsed from the `package_identifier` argument
/// at call time (format: `"owner/repo"`), not stored in the plugin config.
/// A single plugin instance can therefore serve any number of tracked repositories.
pub struct ForgejoPlugin {
    client: reqwest::Client,
    config: ForgejoConfig,
    asset_filters: Vec<Regex>,
}

impl ForgejoPlugin {
    /// Compile-time capabilities for the Forgejo plugin.
    pub const CAPABILITIES: &'static [PluginCapability] =
        &[PluginCapability::ControllerSideFetchReleases];

    /// Create a new `ForgejoPlugin` from the given configuration.
    ///
    /// Validates the configuration and pre-compiles asset filter regexes.
    /// The `_executor` parameter is accepted for registry compatibility but unused
    /// (this plugin is controller-side only).
    pub async fn new(
        config: ForgejoConfig,
        _executor: std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor>,
    ) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(ForgejoError::Configuration(e.to_string())))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        if let Some(ref token) = config.auth_token {
            // Forgejo/Gitea uses "token <value>" rather than "Bearer <value>"
            let value = format!("token {}", token.expose_secret());
            let header_value = reqwest::header::HeaderValue::from_str(&value).map_err(|e| {
                report!(ForgejoError::Configuration(format!(
                    "invalid auth token header value: {e}"
                )))
            })?;
            headers.insert(reqwest::header::AUTHORIZATION, header_value);
        }

        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-releases-forgejo/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(SsrfSafeResolver::new()))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| {
                report!(ForgejoError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        let asset_filters: Vec<Regex> = config
            .asset_patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|e| {
                    report!(ForgejoError::InvalidPattern(format!(
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
    pub(crate) fn releases_url(&self, owner: &str, repo: &str) -> Result<String> {
        let base = self.config.api_base_url().ok_or_else(|| {
            report!(ForgejoError::Configuration(
                "api_base_url is required".to_string()
            ))
        })?;
        Ok(format!(
            "{base}/api/v1/repos/{owner}/{repo}/releases?limit=50"
        ))
    }

    /// Convert a Forgejo API release to an `UpstreamRelease`, applying filters.
    ///
    /// Returns `None` if the release should be skipped (draft, filtered prerelease).
    fn convert_release(&self, release: &ForgejoRelease) -> Option<UpstreamRelease> {
        // Skip drafts
        if release.draft {
            tracing::trace!(tag = %release.tag_name, "skipping draft release");
            return None;
        }

        // Skip prereleases unless configured to include them
        if release.prerelease && !self.config.include_prereleases {
            tracing::trace!(tag = %release.tag_name, "skipping prerelease");
            return None;
        }

        let version_str = strip_tag_prefix(&release.tag_name, &self.config.tag_strip_prefix);
        let version = Version::new(version_str);

        let published_at = release.published_at.as_ref().and_then(|s| {
            OffsetDateTime::parse(s, &Rfc3339)
                .inspect_err(|e| {
                    tracing::warn!(
                        tag = %release.tag_name,
                        error = %e,
                        "failed to parse published_at date"
                    );
                })
                .ok()
        });

        let assets: Vec<ReleaseAsset> = release
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
                content_type: None,
                sha256_digest: None,
            })
            .collect();

        Some({
            let mut r = UpstreamRelease::new(
                version,
                release.tag_name.clone(),
                release.prerelease,
                release.html_url.clone(),
            );
            r.release_notes = release.body.clone();
            r.published_at = published_at;
            r.assets = assets;
            r
        })
    }

    /// Check rate limit headers and log warnings if remaining requests are low.
    fn check_rate_limit(&self, headers: &reqwest::header::HeaderMap, package_identifier: &str) {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        if let Some(remaining) = remaining
            && remaining < 10
        {
            tracing::warn!(
                remaining,
                package_identifier,
                "Forgejo API rate limit is low"
            );
        }
    }
}

/// Parse the URL from a `Link: <url>; rel="next"` HTTP response header.
///
/// Returns `None` if the header is absent or contains no `rel="next"` entry.
fn parse_link_next(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get("link")?.to_str().ok()?;
    link.split(',').find_map(|part| {
        let mut it = part.trim().splitn(2, ';');
        let url = it.next()?.trim().trim_matches(|c| c == '<' || c == '>');
        let rel = it.next()?.trim();
        (rel == r#"rel="next""#).then(|| url.to_owned())
    })
}

#[async_trait]
impl Plugin for ForgejoPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::ReleasesForgejo
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
                "invalid package_identifier for Forgejo plugin: {e}"
            )))
        })?;

        let initial_url = self.releases_url(owner, repo).map_err(|e| {
            report!(PluginError::Configuration(format!(
                "Forgejo plugin configuration error: {e}"
            )))
        })?;
        tracing::debug!(url = %initial_url, "fetching Forgejo releases");

        const MAX_PAGES: usize = 10;
        let mut all_releases: Vec<ForgejoRelease> = Vec::new();
        let mut url = initial_url;

        'pages: for _ in 0..MAX_PAGES {
            let response = self.client.get(&url).send().await.map_err(|e| {
                report!(PluginError::Configuration(format!(
                    "HTTP request failed: {e}"
                )))
            })?;

            let status = response.status();
            self.check_rate_limit(response.headers(), package_identifier);

            if !status.is_success() {
                tracing::debug!(status = %status, "Forgejo API returned error status");

                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    bail!(PluginError::Configuration(
                        "Forgejo API rate limit exceeded".to_string()
                    ));
                }

                let body = response.text().await.unwrap_or_default();
                let message = serde_json::from_str::<ForgejoApiError>(&body)
                    .map(|e| e.message)
                    .unwrap_or(body);

                return Err(report!(ForgejoError::ApiError { status, message })).context_to();
            }

            let next_url = parse_link_next(response.headers());
            let page: Vec<ForgejoRelease> = response.json().await.map_err(|e| {
                report!(PluginError::Serialization(format!(
                    "failed to parse Forgejo API response: {e}"
                )))
            })?;

            if page.is_empty() {
                break 'pages;
            }
            all_releases.extend(page);
            match next_url {
                Some(next) => url = next,
                None => break 'pages,
            }
        }

        let upstream_releases: Vec<UpstreamRelease> = all_releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            total = all_releases.len(),
            "fetched Forgejo releases"
        );

        Ok(upstream_releases)
    }
}

// ── PluginBase + subtrait implementations ────────────────────────────────

uptrakit_plugin_infrastructure_core::impl_plugin_base_config!(
    ForgejoPlugin,
    ForgejoConfig,
    "releases_forgejo",
    fn capabilities(&self) -> Vec<uptrakit_plugin_infrastructure_core::PluginCapability> {
        Self::CAPABILITIES.to_vec()
    }
);

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for ForgejoPlugin {
    async fn fetch_releases(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<UpstreamRelease>> {
        Plugin::fetch_releases(self, package_identifier).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::{ForgejoAsset, ForgejoRelease};
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

    fn test_config() -> ForgejoConfig {
        ForgejoConfig {
            api_base_url: Some("https://forgejo.example.com".to_string()),
            ..ForgejoConfig::default()
        }
    }

    fn test_executor() -> std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor> {
        std::sync::Arc::new(LocalCommandExecutor)
    }

    async fn test_plugin() -> ForgejoPlugin {
        ForgejoPlugin::new(test_config(), test_executor())
            .await
            .expect("valid config")
    }

    fn make_release(tag: &str, draft: bool, prerelease: bool) -> ForgejoRelease {
        ForgejoRelease {
            tag_name: tag.to_string(),
            name: Some(format!("Release {tag}")),
            draft,
            prerelease,
            html_url: format!("https://codeberg.org/owner/repo/releases/tag/{tag}"),
            body: Some("Release notes".to_string()),
            published_at: Some("2024-01-28T12:00:00Z".to_string()),
            assets: vec![],
        }
    }

    // ── parse_owner_repo tests ────────────────────────────────────────────────

    #[test]
    fn parse_owner_repo_valid() {
        let (owner, repo) = parse_owner_repo("owner/repo").expect("valid");
        assert_eq!(owner, "owner");
        assert_eq!(repo, "repo");
    }

    #[test]
    fn parse_owner_repo_missing_slash() {
        assert!(parse_owner_repo("owner").is_err());
    }

    #[test]
    fn parse_owner_repo_two_slashes() {
        assert!(parse_owner_repo("owner/repo/extra").is_err());
    }

    #[test]
    fn parse_owner_repo_empty_owner() {
        assert!(parse_owner_repo("/repo").is_err());
    }

    #[test]
    fn parse_owner_repo_empty_repo() {
        assert!(parse_owner_repo("owner/").is_err());
    }

    #[test]
    fn parse_owner_repo_traversal_in_owner() {
        assert!(parse_owner_repo("../evil/repo").is_err());
    }

    #[test]
    fn parse_owner_repo_traversal_in_repo() {
        assert!(parse_owner_repo("owner/../evil").is_err());
    }

    // ── URL construction tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn url_construction() {
        let plugin = test_plugin().await;
        let url = plugin.releases_url("owner", "repo").expect("valid config");
        assert_eq!(
            url,
            "https://forgejo.example.com/api/v1/repos/owner/repo/releases?limit=50"
        );
    }

    #[tokio::test]
    async fn url_construction_custom_base() {
        let config = ForgejoConfig {
            api_base_url: Some("https://myforgejo.example.com".to_string()),
            ..ForgejoConfig::default()
        };
        let plugin = ForgejoPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let url = plugin.releases_url("owner", "repo").expect("valid config");
        assert_eq!(
            url,
            "https://myforgejo.example.com/api/v1/repos/owner/repo/releases?limit=50"
        );
    }

    // ── convert_release tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn convert_normal_release() {
        let plugin = test_plugin().await;
        let release = make_release("v1.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "1.0.0");
        assert_eq!(upstream.tag, "v1.0.0");
        assert!(!upstream.is_prerelease);
        assert!(upstream.published_at.is_some());
    }

    #[tokio::test]
    async fn skip_draft_release() {
        let plugin = test_plugin().await;
        let release = make_release("v1.0.0", true, false);
        assert!(plugin.convert_release(&release).is_none());
    }

    #[tokio::test]
    async fn skip_prerelease_by_default() {
        let plugin = test_plugin().await;
        let release = make_release("v1.0.0-beta.1", false, true);
        assert!(plugin.convert_release(&release).is_none());
    }

    #[tokio::test]
    async fn include_prerelease_when_configured() {
        let config = ForgejoConfig {
            include_prereleases: true,
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let release = make_release("v1.0.0-beta.1", false, true);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert!(upstream.is_prerelease);
        assert_eq!(upstream.version.as_str(), "1.0.0-beta.1");
    }

    #[tokio::test]
    async fn tag_stripping() {
        let plugin = test_plugin().await;
        let release = make_release("v2.3.4", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "2.3.4");
    }

    #[tokio::test]
    async fn tag_without_prefix() {
        let plugin = test_plugin().await;
        let release = make_release("1.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "1.0.0");
    }

    #[tokio::test]
    async fn custom_tag_prefix() {
        let config = ForgejoConfig {
            tag_strip_prefix: "release-".to_string(),
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let release = make_release("release-3.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "3.0.0");
    }

    #[tokio::test]
    async fn asset_filtering() {
        let config = ForgejoConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string()],
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_executor())
            .await
            .expect("valid config");

        let release = ForgejoRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://codeberg.org/owner/repo/releases/tag/v1.0.0".to_string(),
            body: None,
            published_at: None,
            assets: vec![
                ForgejoAsset {
                    name: "app-linux-amd64.tar.gz".to_string(),
                    browser_download_url:
                        "https://codeberg.org/owner/repo/releases/download/v1.0.0/app.tar.gz"
                            .to_string(),
                    size: 1000,
                },
                ForgejoAsset {
                    name: "app-linux-amd64.deb".to_string(),
                    browser_download_url:
                        "https://codeberg.org/owner/repo/releases/download/v1.0.0/app.deb"
                            .to_string(),
                    size: 2000,
                },
            ],
        };

        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.assets.len(), 1);
        assert_eq!(upstream.assets[0].name, "app-linux-amd64.tar.gz");
    }

    #[tokio::test]
    async fn no_asset_filter_includes_all() {
        let plugin = test_plugin().await;
        let release = ForgejoRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://codeberg.org/owner/repo/releases/tag/v1.0.0".to_string(),
            body: None,
            published_at: None,
            assets: vec![
                ForgejoAsset {
                    name: "a.tar.gz".to_string(),
                    browser_download_url:
                        "https://codeberg.org/owner/repo/releases/download/v1.0.0/a".to_string(),
                    size: 100,
                },
                ForgejoAsset {
                    name: "b.deb".to_string(),
                    browser_download_url:
                        "https://codeberg.org/owner/repo/releases/download/v1.0.0/b".to_string(),
                    size: 200,
                },
            ],
        };

        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.assets.len(), 2);
    }

    #[tokio::test]
    async fn date_parsing() {
        let plugin = test_plugin().await;
        let release = make_release("v1.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        let published = upstream.published_at.expect("should have published_at");
        assert_eq!(published.year(), 2024);
        assert_eq!(published.month() as u8, 1);
        assert_eq!(published.day(), 28);
    }

    #[tokio::test]
    async fn invalid_date_does_not_fail() {
        let plugin = test_plugin().await;
        let release = ForgejoRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            draft: false,
            prerelease: false,
            html_url: "https://codeberg.org/owner/repo/releases/tag/v1.0.0".to_string(),
            body: None,
            published_at: Some("not-a-date".to_string()),
            assets: vec![],
        };
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert!(upstream.published_at.is_none());
    }

    #[tokio::test]
    async fn plugin_creation_fails_without_api_base_url() {
        let config = ForgejoConfig::default();
        assert!(ForgejoPlugin::new(config, test_executor()).await.is_err());
    }

    #[tokio::test]
    async fn plugin_creation_succeeds_with_api_base_url() {
        let config = ForgejoConfig {
            api_base_url: Some("https://codeberg.org".to_string()),
            ..ForgejoConfig::default()
        };
        assert!(ForgejoPlugin::new(config, test_executor()).await.is_ok());
    }

    // ── parse_link_next tests ─────────────────────────────────────────────────

    #[test]
    fn parse_link_next_returns_next_url() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "link",
            r#"<https://api.example.com/releases?page=2&per_page=100>; rel="next", <https://api.example.com/releases?page=5&per_page=100>; rel="last""#
                .parse()
                .unwrap(),
        );
        let next = parse_link_next(&headers);
        assert_eq!(
            next.as_deref(),
            Some("https://api.example.com/releases?page=2&per_page=100")
        );
    }

    #[test]
    fn parse_link_next_absent_when_no_next_rel() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            "link",
            r#"<https://api.example.com/releases?page=1&per_page=100>; rel="first""#
                .parse()
                .unwrap(),
        );
        assert!(parse_link_next(&headers).is_none());
    }

    #[test]
    fn parse_link_next_absent_when_no_link_header() {
        let headers = reqwest::header::HeaderMap::new();
        assert!(parse_link_next(&headers).is_none());
    }
}
