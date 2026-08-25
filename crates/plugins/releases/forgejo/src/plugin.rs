use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, FilteredOutDiagnostic, HostRequirements, HostRuntime,
    PluginCapability, PluginError, PluginFamily, PluginHttpClientConfig, ReleaseAsset,
    UpstreamRelease, Version, build_plugin_http_client, declare_plugin, read_bytes_capped,
    rebase_to_origin,
};

use crate::api_types::{ForgejoApiError, ForgejoRelease};
use crate::config::ForgejoConfig;
use crate::error::{ForgejoError, Result};
use crate::tag::strip_tag_prefix;

/// Pagination window: at most `MAX_RELEASE_PAGES` pages of `PER_PAGE` releases.
const PER_PAGE: usize = 50;

/// Cap for one release-listing page. 100 releases with long bodies stay
/// well under this; 8 MiB bounds a hostile or misconfigured forge.
const MAX_RELEASE_PAGE_BYTES: usize = 8 * 1024 * 1024;

/// Hard cap on release-listing pages per fetch. At the crate's page size
/// this is ample history; an endless Link chain is hostile.
const MAX_RELEASE_PAGES: usize = 20;
/// Hard cap on accumulated releases per fetch (page-size ceiling is
/// 100/page x 20 pages; forgejo pages at 50).
const MAX_TOTAL_RELEASES: usize = 2000;

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
    client: parking_lot::Mutex<Option<reqwest::Client>>,
    config: ForgejoConfig,
    asset_filters: Vec<Regex>,
}

impl ForgejoPlugin {
    /// Create a new `ForgejoPlugin` from the given configuration.
    ///
    /// Pre-compiles asset filter regexes. The HTTP client is built lazily on
    /// first use because the constructor must be synchronous.
    pub fn new(
        config: ForgejoConfig,
        _runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let asset_filters: Vec<Regex> = config
            .asset_patterns
            .iter()
            .map(|p| Regex::new(p).map_err(|e| format!("invalid regex '{p}': {e}")))
            .collect::<std::result::Result<_, _>>()?;

        Ok(Self {
            client: parking_lot::Mutex::new(None),
            config,
            asset_filters,
        })
    }

    /// Get or lazily build the HTTP client.
    fn client(&self) -> Result<reqwest::Client> {
        let mut guard = self.client.lock();
        if let Some(ref c) = *guard {
            return Ok(c.clone());
        }
        let c = Self::build_client(&self.config)?;
        *guard = Some(c.clone());
        Ok(c)
    }

    /// Build the HTTP client with appropriate headers.
    fn build_client(config: &ForgejoConfig) -> Result<reqwest::Client> {
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

        build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-releases-forgejo/",
                env!("CARGO_PKG_VERSION")
            ),
            default_headers: Some(headers),
            ..Default::default()
        })
        .map_err(|e| report!(ForgejoError::Request(e.to_string())))
    }

    /// Build the releases API URL for the given owner/repo pair.
    pub(crate) fn releases_url(&self, owner: &str, repo: &str) -> Result<String> {
        let base = self.config.api_base_url().ok_or_else(|| {
            report!(ForgejoError::Configuration(
                "api_base_url is required".to_string()
            ))
        })?;
        Ok(format!(
            "{base}/api/v1/repos/{owner}/{repo}/releases?limit={PER_PAGE}"
        ))
    }

    /// Baseline release checks shared by `convert_release` and the
    /// filtered-vs-empty diagnostics in `fetch_releases`: drafts are always
    /// skipped; prereleases are skipped unless `include_prereleases` is set.
    fn passes_baseline(&self, release: &ForgejoRelease) -> bool {
        !release.draft && (!release.prerelease || self.config.include_prereleases)
    }

    /// Decide whether a fully-filtered fetch result is a configuration error.
    ///
    /// Delegates to the shared [`FilteredOutDiagnostic`] so the operator-facing
    /// wording stays identical across release-source plugins.
    fn filtered_out_error(
        &self,
        raw_count: usize,
        baseline_count: usize,
        surviving_count: usize,
        window_exhausted: bool,
    ) -> Option<String> {
        FilteredOutDiagnostic {
            raw_count,
            baseline_count,
            surviving_count,
            window_exhausted,
            max_pages: MAX_RELEASE_PAGES,
            per_page: PER_PAGE,
            tag_prefix: self.config.tag_prefix.as_deref(),
            asset_patterns: &self.config.asset_patterns,
            // Compiled filters, not the raw config strings: this check must
            // never disagree with the gating convert_release actually applies.
            asset_filters_active: !self.asset_filters.is_empty(),
        }
        .message()
    }

    /// Convert a Forgejo API release to an `UpstreamRelease`, applying filters.
    ///
    /// Returns `None` if the release should be skipped (draft, filtered prerelease).
    fn convert_release(&self, release: &ForgejoRelease) -> Option<UpstreamRelease> {
        if !self.passes_baseline(release) {
            tracing::trace!(tag = %release.tag_name, "skipping draft or filtered prerelease");
            return None;
        }

        // Series filter: when tag_prefix is set, the release must belong to
        // the series (literal prefix match) — other series in the same repo
        // are excluded entirely, not just stripped.
        if let Some(prefix) = self.config.tag_prefix.as_deref()
            && !prefix.is_empty()
            && !release.tag_name.starts_with(prefix)
        {
            tracing::trace!(tag = %release.tag_name, "skipping release outside tag_prefix series");
            return None;
        }

        // Strip order: tag_prefix first, then tag_strip_prefix, then parse —
        // so "…-standalone-" + "v" and "…-standalone-v" + "v" both yield the
        // same bare version.
        let after_series = strip_tag_prefix(
            &release.tag_name,
            self.config.tag_prefix.as_deref().unwrap_or(""),
        );
        let version_str = strip_tag_prefix(after_series, &self.config.tag_strip_prefix);
        if version_str.is_empty() {
            tracing::trace!(tag = %release.tag_name, "skipping release: empty version after prefix strip");
            return None;
        }
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

        // Asset gating: a configured asset filter that matches nothing means
        // this release has no installable artifact — drop the release instead
        // of surfacing an asset-less "update".
        if !self.asset_filters.is_empty() && assets.is_empty() {
            tracing::trace!(tag = %release.tag_name, "skipping release: no assets match asset_patterns");
            return None;
        }

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

// ── declare_plugin! ──────────────────────────────────────────────────────

declare_plugin!(ForgejoPlugin, ForgejoConfig, "releases.forgejo", {
    display_name: "Forgejo Releases",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
    config_test: [ConfigTestKind::Connectivity],
    roles: [ReleaseFetcher],
    extra_capabilities: [PluginCapability::ControllerSideFetchReleases],
});

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for ForgejoPlugin {
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

        let mut all_releases: Vec<ForgejoRelease> = Vec::new();
        let mut url = initial_url;
        let mut window_exhausted = true;
        let mut pages_fetched: usize = 0;

        'pages: loop {
            if pages_fetched >= MAX_RELEASE_PAGES {
                bail!(PluginError::Configuration(format!(
                    "release pagination exceeded {MAX_RELEASE_PAGES} pages; refusing runaway listing"
                )));
            }
            pages_fetched += 1;

            let response = self
                .client()
                .context_to()?
                .get(&url)
                .send()
                .await
                .map_err(|e| {
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

            let next_url = parse_link_next(response.headers()).and_then(|next| {
                match next.parse::<reqwest::Url>() {
                    Ok(candidate) => match url.parse::<reqwest::Url>() {
                        Ok(current_page_url) => {
                            Some(rebase_to_origin(&current_page_url, &candidate).to_string())
                        }
                        Err(e) => {
                            tracing::warn!(url, error = %e, "unparseable current page URL; stopping pagination");
                            None
                        }
                    },
                    Err(e) => {
                        tracing::warn!(next, error = %e, "unparseable pagination link; stopping pagination");
                        None
                    }
                }
            });
            let body = read_bytes_capped(response, MAX_RELEASE_PAGE_BYTES)
                .await
                .map_err(|e| {
                    report!(PluginError::Serialization(format!(
                        "failed to read Forgejo API response (cap {MAX_RELEASE_PAGE_BYTES} bytes): {e}"
                    )))
                })?;
            let page: Vec<ForgejoRelease> = serde_json::from_slice(&body).map_err(|e| {
                report!(PluginError::Serialization(format!(
                    "failed to parse Forgejo API response: {e}"
                )))
            })?;

            if page.is_empty() {
                window_exhausted = false;
                break 'pages;
            }
            let page_full = page.len() >= PER_PAGE;
            all_releases.extend(page);
            if all_releases.len() > MAX_TOTAL_RELEASES {
                bail!(PluginError::Configuration(format!(
                    "release listing exceeded {MAX_TOTAL_RELEASES} releases; refusing runaway listing"
                )));
            }
            match next_url {
                Some(next) => url = next,
                None => {
                    // No Link header after a FULL page is ambiguous — the
                    // listing may end exactly at the page boundary, or an
                    // intermediary stripped the header. Only a partial page
                    // proves the listing genuinely ended, so only then is the
                    // window known to be un-exhausted.
                    if !page_full {
                        window_exhausted = false;
                    }
                    break 'pages;
                }
            }
        }

        let raw_count = all_releases.len();
        let baseline_count = all_releases
            .iter()
            .filter(|r| self.passes_baseline(r))
            .count();

        let upstream_releases: Vec<UpstreamRelease> = all_releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            baseline = baseline_count,
            total = raw_count,
            "fetched Forgejo releases"
        );

        if let Some(msg) = self.filtered_out_error(
            raw_count,
            baseline_count,
            upstream_releases.len(),
            window_exhausted,
        ) {
            bail!(PluginError::Configuration(msg));
        }

        Ok(upstream_releases)
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use crate::api_types::{ForgejoAsset, ForgejoRelease};
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PluginHttpClientConfig, PluginMeta, ReleaseFetcher,
        SsrfMode, StandardHostRuntime, build_plugin_http_client,
    };

    fn test_config() -> ForgejoConfig {
        ForgejoConfig {
            api_base_url: Some("https://forgejo.example.com".to_string()),
            ..ForgejoConfig::default()
        }
    }

    fn test_runtime() -> Arc<dyn HostRuntime> {
        let executor = Arc::new(uptrakit_plugin_infrastructure_core::LocalCommandExecutor)
            as Arc<dyn uptrakit_plugin_infrastructure_core::command::CommandExecutor>;
        let caps = HostCapabilities::default();
        Arc::new(StandardHostRuntime::new(executor, caps))
    }

    fn test_plugin() -> ForgejoPlugin {
        ForgejoPlugin::new(test_config(), test_runtime()).expect("valid config")
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

    #[test]
    fn url_construction() {
        let plugin = test_plugin();
        let url = plugin.releases_url("owner", "repo").expect("valid config");
        assert_eq!(
            url,
            "https://forgejo.example.com/api/v1/repos/owner/repo/releases?limit=50"
        );
    }

    #[test]
    fn url_construction_custom_base() {
        let config = ForgejoConfig {
            api_base_url: Some("https://myforgejo.example.com".to_string()),
            ..ForgejoConfig::default()
        };
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");
        let url = plugin.releases_url("owner", "repo").expect("valid config");
        assert_eq!(
            url,
            "https://myforgejo.example.com/api/v1/repos/owner/repo/releases?limit=50"
        );
    }

    // ── convert_release tests ─────────────────────────────────────────────────

    #[test]
    fn convert_normal_release() {
        let plugin = test_plugin();
        let release = make_release("v1.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "1.0.0");
        assert_eq!(upstream.tag, "v1.0.0");
        assert!(!upstream.is_prerelease);
        assert!(upstream.published_at.is_some());
    }

    #[test]
    fn skip_draft_release() {
        let plugin = test_plugin();
        let release = make_release("v1.0.0", true, false);
        assert!(plugin.convert_release(&release).is_none());
    }

    #[test]
    fn skip_prerelease_by_default() {
        let plugin = test_plugin();
        let release = make_release("v1.0.0-beta.1", false, true);
        assert!(plugin.convert_release(&release).is_none());
    }

    #[test]
    fn include_prerelease_when_configured() {
        let config = ForgejoConfig {
            include_prereleases: true,
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");
        let release = make_release("v1.0.0-beta.1", false, true);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert!(upstream.is_prerelease);
        assert_eq!(upstream.version.as_str(), "1.0.0-beta.1");
    }

    #[test]
    fn tag_stripping() {
        let plugin = test_plugin();
        let release = make_release("v2.3.4", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "2.3.4");
    }

    #[test]
    fn tag_without_prefix() {
        let plugin = test_plugin();
        let release = make_release("1.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "1.0.0");
    }

    #[test]
    fn custom_tag_prefix() {
        let config = ForgejoConfig {
            tag_strip_prefix: "release-".to_string(),
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");
        let release = make_release("release-3.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        assert_eq!(upstream.version.as_str(), "3.0.0");
    }

    #[test]
    fn asset_filtering() {
        let config = ForgejoConfig {
            asset_patterns: vec![r".*\.tar\.gz$".to_string()],
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");

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

    #[test]
    fn no_asset_filter_includes_all() {
        let plugin = test_plugin();
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

    #[test]
    fn date_parsing() {
        let plugin = test_plugin();
        let release = make_release("v1.0.0", false, false);
        let upstream = plugin.convert_release(&release).expect("should convert");
        let published = upstream.published_at.expect("should have published_at");
        assert_eq!(published.year(), 2024);
        assert_eq!(published.month() as u8, 1);
        assert_eq!(published.day(), 28);
    }

    #[test]
    fn invalid_date_does_not_fail() {
        let plugin = test_plugin();
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

    // ── tag_prefix series tests ───────────────────────────────────────────────

    fn series_plugin(tag_prefix: &str) -> ForgejoPlugin {
        let config = ForgejoConfig {
            tag_prefix: Some(tag_prefix.to_string()),
            ..test_config()
        };
        ForgejoPlugin::new(config, test_runtime()).expect("valid config")
    }

    #[test]
    fn tag_prefix_filters_other_series() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        let foreign = make_release("uptrakit-agent-v1.0.0", false, false);
        assert!(plugin.convert_release(&foreign).is_none());
        let ours = make_release("uptrakit-controller-standalone-v0.0.7", false, false);
        assert!(plugin.convert_release(&ours).is_some());
    }

    #[test]
    fn strip_composition_prefix_without_v() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        let fj = make_release("uptrakit-controller-standalone-v0.0.7", false, false);
        let release = plugin.convert_release(&fj).expect("should convert");
        assert_eq!(release.version.as_str(), "0.0.7");
        assert_eq!(release.tag, "uptrakit-controller-standalone-v0.0.7");
    }

    #[test]
    fn strip_composition_prefix_including_v() {
        let plugin = series_plugin("uptrakit-controller-standalone-v");
        let fj = make_release("uptrakit-controller-standalone-v0.0.7", false, false);
        let release = plugin.convert_release(&fj).expect("should convert");
        assert_eq!(release.version.as_str(), "0.0.7");
    }

    #[test]
    fn tag_equal_to_composed_prefix_dropped() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        let fj = make_release("uptrakit-controller-standalone-v", false, false);
        assert!(plugin.convert_release(&fj).is_none());
    }

    // ── asset gating (D3) tests ───────────────────────────────────────────────

    fn release_with_asset(tag: &str, asset_name: &str) -> ForgejoRelease {
        let mut fj = make_release(tag, false, false);
        fj.assets = vec![ForgejoAsset {
            name: asset_name.to_string(),
            browser_download_url: format!("https://example.com/{asset_name}"),
            size: 42,
        }];
        fj
    }

    #[test]
    fn asset_gating_drops_release_without_matching_assets() {
        let config = ForgejoConfig {
            asset_patterns: vec![r".*\.deb$".to_string()],
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");
        let fj = release_with_asset("v1.0.0", "app.rpm");
        assert!(plugin.convert_release(&fj).is_none());
        let fj_ok = release_with_asset("v1.0.0", "app.deb");
        assert!(plugin.convert_release(&fj_ok).is_some());
    }

    #[test]
    fn no_asset_patterns_keeps_assetless_release() {
        let plugin = test_plugin();
        let fj = make_release("v1.0.0", false, false);
        assert!(plugin.convert_release(&fj).is_some());
    }

    // ── filtered-to-zero diagnostics (D4) tests ───────────────────────────────

    #[test]
    fn filtered_out_error_fires_when_series_filter_removes_all() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        let msg = plugin
            .filtered_out_error(120, 100, 0, false)
            .expect("error expected");
        assert!(msg.contains("uptrakit-controller-standalone-"));
        assert!(msg.contains("120"), "raw count must be named");
        assert!(msg.contains("100"), "baseline count must be named");
        assert!(msg.contains("tag_prefix"), "fetch recovery lever");
        assert!(
            msg.contains("version_strip_prefix"),
            "detect recovery lever"
        );
        assert!(msg.contains("tag_strip_prefix"), "full-prefix warning");
        assert!(
            msg.contains("nothing upstream matches"),
            "non-exhausted window wording"
        );
    }

    #[test]
    fn filtered_out_error_window_exhausted_reads_differently() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        // Counts deliberately distinct from the 20x50=1000 window numbers so
        // no assert can pass by matching the wrong figure.
        let msg = plugin
            .filtered_out_error(70, 60, 0, true)
            .expect("error expected");
        assert!(msg.contains("exhausted"));
        assert!(msg.contains("1000"), "window bound must be named");
        assert!(msg.contains("70"), "raw count must be named");
        assert!(msg.contains("60"), "baseline count must be named");
    }

    #[test]
    fn filtered_out_error_silent_without_active_filters() {
        // tag_strip_prefix eating whole tags is not a *new* filter: stay silent.
        let plugin = test_plugin();
        assert!(plugin.filtered_out_error(5, 5, 0, false).is_none());
    }

    #[test]
    fn filtered_out_error_silent_when_baseline_empty() {
        // All-prerelease repo with tag_prefix set: empty success, no error.
        let plugin = series_plugin("uptrakit-controller-standalone-");
        assert!(plugin.filtered_out_error(5, 0, 0, false).is_none());
    }

    #[test]
    fn filtered_out_error_silent_when_survivors_exist() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        assert!(plugin.filtered_out_error(5, 5, 1, false).is_none());
    }

    #[test]
    fn filtered_out_error_fires_for_asset_patterns_alone() {
        let config = ForgejoConfig {
            asset_patterns: vec![r".*\.deb$".to_string()],
            ..test_config()
        };
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");
        let msg = plugin
            .filtered_out_error(3, 3, 0, false)
            .expect("error expected");
        assert!(msg.contains("asset_patterns"));
    }

    // ── plugin_type_id ──────────────────────────────────────────────────

    #[test]
    fn plugin_type_id() {
        let plugin = test_plugin();
        assert_eq!(plugin.plugin_type_id().as_str(), "releases.forgejo");
    }

    // ── descriptor capabilities ─────────────────────────────────────────

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ReleaseFetching)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ControllerSideFetchReleases)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
    }

    #[test]
    fn descriptor_has_release_fetcher_role() {
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
        assert!(DESCRIPTOR.roles.update_executor.is_none());
    }

    #[test]
    fn plugin_creation_succeeds_with_api_base_url() {
        let config = ForgejoConfig {
            api_base_url: Some("https://codeberg.org".to_string()),
            ..ForgejoConfig::default()
        };
        assert!(ForgejoPlugin::new(config, test_runtime()).is_ok());
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

    // ── wired fetch_releases seam tests (httpmock) ────────────────────────────

    fn plugin_for_mock(config: ForgejoConfig) -> ForgejoPlugin {
        let plugin = ForgejoPlugin::new(config, test_runtime()).expect("valid config");
        // Same-crate test: seed the lazy client cache with a permissive-SSRF
        // client so the plugin can reach the httpmock server on 127.0.0.1
        // (idiom from the npm plugin's release tests).
        let client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: "uptrakit-plugin-releases-forgejo-test",
            ssrf_mode: SsrfMode::Permissive,
            ..PluginHttpClientConfig::default()
        })
        .expect("build test HTTP client");
        *plugin.client.lock() = Some(client);
        plugin
    }

    #[tokio::test]
    async fn fetch_releases_all_filtered_fails_with_configuration_error() {
        let server = httpmock::MockServer::start_async().await;
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/api/v1/repos/o/r/releases");
                then.status(200).json_body(serde_json::json!([{
                    "tag_name": "other-series-v1.0.0",
                    "name": null,
                    "draft": false,
                    "prerelease": false,
                    "html_url": "https://example.com/r/other-series-v1.0.0",
                    "body": null,
                    "published_at": null,
                    "assets": []
                }]));
            })
            .await;
        let plugin = plugin_for_mock(ForgejoConfig {
            api_base_url: Some(server.base_url()),
            tag_prefix: Some("uptrakit-controller-standalone-".to_string()),
            ..ForgejoConfig::default()
        });
        let err = plugin.fetch_releases("o/r").await.expect_err("must fail");
        let rendered = format!("{err:?}");
        assert!(rendered.contains("no releases survive the configured filters"));
        assert!(rendered.contains("uptrakit-controller-standalone-"));
        mock.assert_async().await;
    }

    #[tokio::test]
    async fn fetch_releases_all_prerelease_with_tag_prefix_is_empty_success() {
        let server = httpmock::MockServer::start_async().await;
        server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/api/v1/repos/o/r/releases");
                then.status(200).json_body(serde_json::json!([{
                    "tag_name": "uptrakit-controller-standalone-v0.0.7",
                    "name": null,
                    "draft": false,
                    "prerelease": true,
                    "html_url": "https://example.com/r/t",
                    "body": null,
                    "published_at": null,
                    "assets": []
                }]));
            })
            .await;
        let plugin = plugin_for_mock(ForgejoConfig {
            api_base_url: Some(server.base_url()),
            tag_prefix: Some("uptrakit-controller-standalone-".to_string()),
            ..ForgejoConfig::default()
        });
        let releases = plugin.fetch_releases("o/r").await.expect("empty success");
        assert!(releases.is_empty());
    }

    /// An over-cap release-page body is terminal (`PluginError::Serialization`)
    /// — mirrors the "malformed body" branch this cap replaces.
    #[tokio::test]
    async fn release_page_over_cap_is_rejected() {
        let server = httpmock::MockServer::start_async().await;
        let over_cap_body = "x".repeat(MAX_RELEASE_PAGE_BYTES + 1);
        let mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/api/v1/repos/o/r/releases");
                then.status(200).body(&over_cap_body);
            })
            .await;
        let plugin = plugin_for_mock(ForgejoConfig {
            api_base_url: Some(server.base_url()),
            ..ForgejoConfig::default()
        });
        let err = plugin
            .fetch_releases("o/r")
            .await
            .expect_err("over-cap release page must be rejected");
        assert!(
            matches!(err.current_context(), PluginError::Serialization(_)),
            "expected Serialization error, got: {err:?}"
        );
        mock.assert_async().await;
    }

    /// A hostile forge can serve an endless `Link: next` chain. Register one
    /// more page than `MAX_RELEASE_PAGES` allows and prove the fetch is
    /// terminal instead of silently truncating: the (`MAX_RELEASE_PAGES`+1)th
    /// page is never requested.
    #[tokio::test]
    async fn pagination_stops_at_page_cap() {
        let server = httpmock::MockServer::start_async().await;
        let total_pages = MAX_RELEASE_PAGES + 1;
        let mut mocks = Vec::with_capacity(total_pages);
        for n in 1..=total_pages {
            let path = if n == 1 {
                "/api/v1/repos/o/r/releases".to_string()
            } else {
                format!("/api/v1/repos/o/r/releases/p{n}")
            };
            let next_page = n + 1;
            let link_header = format!(
                r#"<{}/api/v1/repos/o/r/releases/p{next_page}>; rel="next""#,
                server.base_url()
            );
            let mock = server
                .mock_async(move |when, then| {
                    when.method(httpmock::Method::GET).path(path.clone());
                    then.status(200)
                        .header("link", &link_header)
                        .json_body(serde_json::json!([{
                            "tag_name": format!("v1.0.{n}"),
                            "name": null,
                            "draft": false,
                            "prerelease": false,
                            "html_url": "https://example.com/r/v1.0.0",
                            "body": null,
                            "published_at": null,
                            "assets": []
                        }]));
                })
                .await;
            mocks.push(mock);
        }

        let plugin = plugin_for_mock(ForgejoConfig {
            api_base_url: Some(server.base_url()),
            ..ForgejoConfig::default()
        });
        let err = plugin
            .fetch_releases("o/r")
            .await
            .expect_err("runaway Link chain must be rejected");
        assert!(
            matches!(err.current_context(), PluginError::Configuration(_)),
            "expected Configuration error, got: {err:?}"
        );

        mocks
            .last()
            .expect("at least one mock registered")
            .assert_calls_async(0)
            .await;
    }
}
