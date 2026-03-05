use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use uptrakit_plugin_infrastructure_core::{
    Plugin, PluginCapability, PluginError, PluginType, ReleaseAsset, UpstreamRelease, Version,
};

use crate::api_types::{GitLabApiError, GitLabRelease};
use crate::config::GitLabConfig;
use crate::error::{GitLabError, Result};
use crate::tag::strip_tag_prefix;

/// Parse and validate a GitLab project path from a package identifier string.
///
/// Returns the **percent-encoded** project path (all `/` replaced with `%2F`),
/// suitable for use in the GitLab Projects API URL.
///
/// Rules:
/// - Must contain at least one `/` (minimum `namespace/project`).
/// - All path components (split on `/`) must be non-empty.
/// - No component may contain `..` (path traversal guard).
pub fn parse_project_path(package_identifier: &str) -> Result<String> {
    if !package_identifier.contains('/') {
        bail!(GitLabError::Configuration(format!(
            "package_identifier must be 'namespace/project' or \
             'group/subgroup/project' (got '{package_identifier}')"
        )));
    }

    for component in package_identifier.split('/') {
        if component.is_empty() {
            bail!(GitLabError::Configuration(format!(
                "package_identifier must not contain empty path components \
                 (got '{package_identifier}')"
            )));
        }
        if component.contains("..") {
            bail!(GitLabError::Configuration(format!(
                "package_identifier must not contain '..': '{component}' in \
                 '{package_identifier}'"
            )));
        }
    }

    // Percent-encode: replace every `/` with `%2F` for the Projects API.
    Ok(package_identifier.replace('/', "%2F"))
}

/// GitLab Releases plugin implementation.
///
/// Fetches release metadata from the GitLab API and converts it into
/// `UpstreamRelease` values for the controller.
///
/// The project path is parsed from the `package_identifier` argument at call
/// time (format: `"namespace/project"` or `"group/subgroup/project"`), not
/// stored in the plugin config. A single plugin instance can therefore serve
/// any number of tracked GitLab projects.
///
/// GitLab uses `upcoming_release: true` to flag releases that are not yet
/// publicly visible (similar to GitHub's draft status). When
/// `include_prereleases` is `false`, such releases are skipped.
pub struct GitLabPlugin {
    client: reqwest::Client,
    config: GitLabConfig,
    asset_filters: Vec<Regex>,
}

impl GitLabPlugin {
    /// Compile-time capabilities for the GitLab plugin.
    pub const CAPABILITIES: &'static [PluginCapability] =
        &[PluginCapability::ControllerSideFetchReleases];

    /// Create a new `GitLabPlugin` from the given configuration.
    ///
    /// Validates the configuration and pre-compiles asset filter regexes.
    /// The `_executor` parameter is accepted for registry compatibility but unused
    /// (this plugin is controller-side only).
    pub async fn new(
        config: GitLabConfig,
        _executor: std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor>,
    ) -> Result<Self> {
        config
            .validate()
            .map_err(|e| report!(GitLabError::Configuration(e.to_string())))?;

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json"),
        );

        if let Some(ref token) = config.auth_token {
            // GitLab uses the PRIVATE-TOKEN header for personal access tokens.
            let header_value = reqwest::header::HeaderValue::from_str(token.expose_secret())
                .map_err(|e| {
                    report!(GitLabError::Configuration(format!(
                        "invalid auth token header value: {e}"
                    )))
                })?;
            headers.insert("PRIVATE-TOKEN", header_value);
        }

        let client = reqwest::Client::builder()
            .user_agent(concat!(
                "uptrakit-plugin-releases-gitlab/",
                env!("CARGO_PKG_VERSION")
            ))
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| {
                report!(GitLabError::Request(format!(
                    "failed to build HTTP client: {e}"
                )))
            })?;

        let asset_filters: Vec<Regex> = config
            .asset_patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|e| {
                    report!(GitLabError::InvalidPattern(format!(
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

    /// Build the releases API URL for the given percent-encoded project path.
    pub(crate) fn releases_url(&self, encoded_path: &str) -> String {
        format!(
            "{}/api/v4/projects/{}/releases?per_page=100",
            self.config.api_base_url(),
            encoded_path,
        )
    }

    /// Convert a GitLab API release to an `UpstreamRelease`, applying filters.
    ///
    /// Returns `None` if the release should be skipped.
    ///
    /// GitLab has no `draft` concept. `upcoming_release: true` is the closest
    /// equivalent to "not yet public". It is skipped unless `include_prereleases`
    /// is `true`.
    fn convert_release(&self, gl_release: &GitLabRelease) -> Option<UpstreamRelease> {
        // Skip upcoming (unreleased) releases unless configured to include them.
        if gl_release.upcoming_release && !self.config.include_prereleases {
            tracing::trace!(tag = %gl_release.tag_name, "skipping upcoming release");
            return None;
        }

        let version_str = strip_tag_prefix(&gl_release.tag_name, &self.config.tag_strip_prefix);
        let version = Version::new(version_str);

        let published_at = gl_release.released_at.as_ref().and_then(|s| {
            OffsetDateTime::parse(s, &Rfc3339)
                .inspect_err(|e| {
                    tracing::warn!(
                        tag = %gl_release.tag_name,
                        error = %e,
                        "failed to parse released_at date"
                    );
                })
                .ok()
        });

        // Only expose manually-uploaded asset links.
        // GitLab always provides auto-generated source archives (zip, tar.gz, etc.)
        // via `sources`, but those are not user-provided binaries; we exclude them.
        let assets: Vec<ReleaseAsset> = gl_release
            .assets
            .links
            .iter()
            .filter(|a| {
                if self.asset_filters.is_empty() {
                    return true;
                }
                self.asset_filters.iter().any(|re| re.is_match(&a.name))
            })
            .map(|a| ReleaseAsset {
                name: a.name.clone(),
                download_url: a.url.clone(),
                size: None,
                content_type: None,
                sha256_digest: None,
            })
            .collect();

        // GitLab uses `released_at` as the canonical release URL via the web UI.
        // Construct the release URL from the project path and tag.
        Some(UpstreamRelease {
            version,
            tag: gl_release.tag_name.clone(),
            is_prerelease: gl_release.upcoming_release,
            release_url: String::new(), // filled in fetch_releases from package_identifier
            release_notes: gl_release.description.clone(),
            published_at,
            assets,
            category: None,
            attestation_status: None,
        })
    }

    /// Check rate limit headers and log warnings if remaining requests are low.
    fn check_rate_limit(&self, headers: &reqwest::header::HeaderMap, package_identifier: &str) {
        let remaining = headers
            .get("ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok());

        if let Some(remaining) = remaining
            && remaining < 10
        {
            tracing::warn!(
                remaining,
                package_identifier,
                "GitLab API rate limit is low"
            );
        }
    }
}

#[async_trait]
impl Plugin for GitLabPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::ReleasesGitlab
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }

    #[tracing::instrument(skip_all)]
    async fn fetch_releases(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<UpstreamRelease>> {
        let encoded_path = parse_project_path(package_identifier).map_err(|e| {
            report!(PluginError::Configuration(format!(
                "invalid package_identifier for GitLab plugin: {e}"
            )))
        })?;

        let url = self.releases_url(&encoded_path);
        tracing::debug!(url = %url, "fetching GitLab releases");

        let response = self.client.get(&url).send().await.map_err(|e| {
            report!(PluginError::Configuration(format!(
                "HTTP request failed: {e}"
            )))
        })?;

        let status = response.status();
        self.check_rate_limit(response.headers(), package_identifier);

        if !status.is_success() {
            tracing::debug!(status = %status, "GitLab API returned error status");

            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                bail!(PluginError::Configuration(
                    "GitLab API rate limit exceeded".to_string()
                ));
            }

            let body = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<GitLabApiError>(&body)
                .map(|e| e.message)
                .unwrap_or(body);

            return Err(report!(GitLabError::ApiError { status, message })).context_to();
        }

        let releases: Vec<GitLabRelease> = response.json().await.map_err(|e| {
            report!(PluginError::Serialization(format!(
                "failed to parse GitLab API response: {e}"
            )))
        })?;

        // Build the web UI release URL base from the API base and project path.
        // Format: https://gitlab.com/{namespace}/{project}/-/releases/{tag}
        // We reconstruct the unencoded path for the web URL.
        let web_base = format!(
            "{}/{}/-/releases",
            self.config.api_base_url(),
            package_identifier
        );

        let upstream_releases: Vec<UpstreamRelease> = releases
            .iter()
            .filter_map(|r| {
                let mut upstream = self.convert_release(r)?;
                upstream.release_url = format!("{web_base}/{}", r.tag_name);
                Some(upstream)
            })
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            total = releases.len(),
            "fetched GitLab releases"
        );

        Ok(upstream_releases)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api_types::{GitLabRelease, GitLabReleaseAssets, GitLabReleaseLink};
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;

    fn test_config() -> GitLabConfig {
        GitLabConfig::default()
    }

    fn test_executor() -> std::sync::Arc<dyn uptrakit_plugin_infrastructure_core::CommandExecutor> {
        std::sync::Arc::new(LocalCommandExecutor)
    }

    async fn test_plugin() -> GitLabPlugin {
        GitLabPlugin::new(test_config(), test_executor())
            .await
            .expect("valid config")
    }

    fn make_release(tag: &str, upcoming: bool) -> GitLabRelease {
        GitLabRelease {
            tag_name: tag.to_string(),
            name: Some(format!("Release {tag}")),
            description: Some("Release notes".to_string()),
            released_at: Some("2024-01-28T12:00:00Z".to_string()),
            upcoming_release: upcoming,
            assets: GitLabReleaseAssets { links: vec![] },
        }
    }

    // ── parse_project_path tests ──────────────────────────────────────────────

    #[test]
    fn parse_project_path_simple() {
        let encoded = parse_project_path("owner/project").expect("valid");
        assert_eq!(encoded, "owner%2Fproject");
    }

    #[test]
    fn parse_project_path_nested_namespace() {
        let encoded = parse_project_path("group/subgroup/project").expect("valid");
        assert_eq!(encoded, "group%2Fsubgroup%2Fproject");
    }

    #[test]
    fn parse_project_path_no_slash_fails() {
        assert!(parse_project_path("project").is_err());
    }

    #[test]
    fn parse_project_path_empty_component_fails() {
        assert!(parse_project_path("owner//project").is_err());
        assert!(parse_project_path("/project").is_err());
        assert!(parse_project_path("owner/").is_err());
    }

    #[test]
    fn parse_project_path_traversal_fails() {
        assert!(parse_project_path("../evil/project").is_err());
        assert!(parse_project_path("owner/../evil").is_err());
        assert!(parse_project_path("group/sub../project").is_err());
    }

    // ── URL construction tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn url_construction() {
        let plugin = test_plugin().await;
        let url = plugin.releases_url("owner%2Fproject");
        assert_eq!(
            url,
            "https://gitlab.com/api/v4/projects/owner%2Fproject/releases?per_page=100"
        );
    }

    #[tokio::test]
    async fn url_construction_nested_namespace() {
        let plugin = test_plugin().await;
        let url = plugin.releases_url("group%2Fsubgroup%2Fproject");
        assert_eq!(
            url,
            "https://gitlab.com/api/v4/projects/group%2Fsubgroup%2Fproject/releases?per_page=100"
        );
    }

    #[tokio::test]
    async fn url_construction_custom_base() {
        let config = GitLabConfig {
            api_base_url: Some("https://gitlab.corp.com".to_string()),
            ..GitLabConfig::default()
        };
        let plugin = GitLabPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let url = plugin.releases_url("owner%2Fproject");
        assert_eq!(
            url,
            "https://gitlab.corp.com/api/v4/projects/owner%2Fproject/releases?per_page=100"
        );
    }

    // ── convert_release tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn convert_normal_release() {
        let plugin = test_plugin().await;
        let gl = make_release("v1.0.0", false);
        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert_eq!(upstream.version.as_str(), "1.0.0");
        assert_eq!(upstream.tag, "v1.0.0");
        assert!(!upstream.is_prerelease);
        assert!(upstream.published_at.is_some());
    }

    #[tokio::test]
    async fn skip_upcoming_release_by_default() {
        let plugin = test_plugin().await;
        let gl = make_release("v1.0.0-upcoming", true);
        assert!(plugin.convert_release(&gl).is_none());
    }

    #[tokio::test]
    async fn include_upcoming_when_configured() {
        let config = GitLabConfig {
            include_prereleases: true,
            ..GitLabConfig::default()
        };
        let plugin = GitLabPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let gl = make_release("v1.0.0-rc1", true);
        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert!(upstream.is_prerelease);
        assert_eq!(upstream.version.as_str(), "1.0.0-rc1");
    }

    #[tokio::test]
    async fn tag_stripping() {
        let plugin = test_plugin().await;
        let gl = make_release("v2.3.4", false);
        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert_eq!(upstream.version.as_str(), "2.3.4");
    }

    #[tokio::test]
    async fn tag_without_prefix() {
        let plugin = test_plugin().await;
        let gl = make_release("1.0.0", false);
        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert_eq!(upstream.version.as_str(), "1.0.0");
    }

    #[tokio::test]
    async fn custom_tag_prefix() {
        let config = GitLabConfig {
            tag_strip_prefix: "release-".to_string(),
            ..GitLabConfig::default()
        };
        let plugin = GitLabPlugin::new(config, test_executor())
            .await
            .expect("valid config");
        let gl = make_release("release-3.0.0", false);
        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert_eq!(upstream.version.as_str(), "3.0.0");
    }

    #[tokio::test]
    async fn asset_link_filtering() {
        let config = GitLabConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string()],
            ..GitLabConfig::default()
        };
        let plugin = GitLabPlugin::new(config, test_executor())
            .await
            .expect("valid config");

        let gl = GitLabRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            description: None,
            released_at: None,
            upcoming_release: false,
            assets: GitLabReleaseAssets {
                links: vec![
                    GitLabReleaseLink {
                        name: "app-linux-amd64.tar.gz".to_string(),
                        url: "https://gitlab.com/owner/project/-/releases/v1.0.0/downloads/app.tar.gz".to_string(),
                    },
                    GitLabReleaseLink {
                        name: "app-linux-amd64.deb".to_string(),
                        url: "https://gitlab.com/owner/project/-/releases/v1.0.0/downloads/app.deb".to_string(),
                    },
                ],
            },
        };

        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert_eq!(upstream.assets.len(), 1);
        assert_eq!(upstream.assets[0].name, "app-linux-amd64.tar.gz");
    }

    #[tokio::test]
    async fn no_asset_filter_includes_all_links() {
        let plugin = test_plugin().await;
        let gl = GitLabRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            description: None,
            released_at: None,
            upcoming_release: false,
            assets: GitLabReleaseAssets {
                links: vec![
                    GitLabReleaseLink {
                        name: "a.tar.gz".to_string(),
                        url: "https://gitlab.com/owner/project/-/releases/v1.0.0/downloads/a"
                            .to_string(),
                    },
                    GitLabReleaseLink {
                        name: "b.deb".to_string(),
                        url: "https://gitlab.com/owner/project/-/releases/v1.0.0/downloads/b"
                            .to_string(),
                    },
                ],
            },
        };

        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert_eq!(upstream.assets.len(), 2);
    }

    #[tokio::test]
    async fn date_parsing() {
        let plugin = test_plugin().await;
        let gl = make_release("v1.0.0", false);
        let upstream = plugin.convert_release(&gl).expect("should convert");
        let published = upstream.published_at.expect("should have published_at");
        assert_eq!(published.year(), 2024);
        assert_eq!(published.month() as u8, 1);
        assert_eq!(published.day(), 28);
    }

    #[tokio::test]
    async fn invalid_date_does_not_fail() {
        let plugin = test_plugin().await;
        let gl = GitLabRelease {
            tag_name: "v1.0.0".to_string(),
            name: None,
            description: None,
            released_at: Some("not-a-date".to_string()),
            upcoming_release: false,
            assets: GitLabReleaseAssets { links: vec![] },
        };
        let upstream = plugin.convert_release(&gl).expect("should convert");
        assert!(upstream.published_at.is_none());
    }

    #[tokio::test]
    async fn plugin_creation_succeeds_with_empty_config() {
        let config = GitLabConfig::default();
        assert!(GitLabPlugin::new(config, test_executor()).await.is_ok());
    }
}
