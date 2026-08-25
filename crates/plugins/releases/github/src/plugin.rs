#![expect(
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "array and slice indices are bounded by construction or derived from known-valid positions; string slices use byte positions derived from ASCII-only content or fixed-length pattern matching; UTF-8 boundary safety is guaranteed by construction"
)]
use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::io::AsyncWriteExt;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec, send_output};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    AttestationStatus, ConfigModel, ConfigTestKind, ExecuteUpdateResult, FilteredOutDiagnostic,
    HostRequirements, HostRuntime, OutputStreamType, PluginCapability, PluginError, PluginFamily,
    ReleaseAsset, ReleaseInfo, SudoCommandEntry, UpdateOutputLine, UpstreamRelease, Version,
    declare_plugin,
};
use uptrakit_plugin_infrastructure_core::{
    PluginHttpClientConfig, RedirectMode, build_plugin_http_client, read_bytes_capped,
    rebase_to_origin,
};

use crate::api_types::{AttestationsApiResponse, GitHubApiError, GitHubAsset, GitHubRelease};
use crate::config::GitHubConfig;
use crate::error::{GitHubError, Result};
use crate::tag::strip_tag_prefix;

/// Pagination window: at most `MAX_RELEASE_PAGES` pages of `PER_PAGE` releases.
const PER_PAGE: usize = 100;

/// Cap for one release-listing page. 100 releases with long bodies stay
/// well under this; 8 MiB bounds a hostile or misconfigured forge.
const MAX_RELEASE_PAGE_BYTES: usize = 8 * 1024 * 1024;

/// Hard cap on release-listing pages per fetch. At the crate's page size
/// this is ample history; an endless Link chain is hostile.
const MAX_RELEASE_PAGES: usize = 20;
/// Hard cap on accumulated releases per fetch (page-size ceiling is
/// 100/page x 20 pages; forgejo pages at 50).
const MAX_TOTAL_RELEASES: usize = 2000;

/// Cap for a single release-asset download. Sized generously: real-world
/// release assets are far smaller, but the read must still be bounded.
const MAX_ASSET_DOWNLOAD_BYTES: usize = 512 * 1024 * 1024;

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
/// `UpstreamRelease` values for the controller. When `install_path` is
/// configured, also supports `execute_update` on the agent side: downloads
/// the matching release asset, verifies its SHA-256 checksum, and installs
/// it at the target path.
///
/// The `owner` and `repo` are parsed from the `package_identifier` argument
/// at call time (format: `"owner/repo"`), not stored in the plugin config.
/// A single plugin instance can therefore serve any number of GitHub repositories.
pub struct GitHubPlugin {
    client: parking_lot::Mutex<Option<reqwest::Client>>,
    config: GitHubConfig,
    asset_filters: Vec<Regex>,
    /// Command executor for agent-side `execute_update`. `None` when instantiated
    /// on the controller (where only `ReleaseFetcher` runs).
    executor: Option<Arc<dyn CommandExecutor>>,
}

impl GitHubPlugin {
    /// Create a new `GitHubPlugin` from the given configuration.
    ///
    /// Pre-compiles asset filter regexes. The HTTP client is built lazily.
    /// The POSIX executor is obtained from the runtime if available (agent-side);
    /// on the controller side it will be `None`.
    pub fn new(
        config: GitHubConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let asset_filters: Vec<Regex> = config
            .asset_patterns
            .iter()
            .map(|p| Regex::new(p).map_err(|e| format!("invalid regex '{p}': {e}")))
            .collect::<std::result::Result<_, _>>()?;

        // Try to get POSIX executor; None is fine for controller-side usage.
        let executor = Some(runtime.executor());

        Ok(Self {
            client: parking_lot::Mutex::new(None),
            config,
            asset_filters,
            executor,
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
    fn build_client(config: &GitHubConfig) -> Result<reqwest::Client> {
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

        // The checksums fetch reuses this client. It must NEVER follow
        // redirects — keep the default RedirectMode::None (spec
        // 2026-08-13-plugin-http-redirect-security; the checksums path is
        // deleted by the digest-attestation spec).
        build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-releases-github/",
                env!("CARGO_PKG_VERSION")
            ),
            default_headers: Some(headers),
            ..Default::default()
        })
        .map_err(|e| report!(GitHubError::Request(e.to_string())))
    }

    /// Return the sudo commands required by the GitHub plugin.
    ///
    /// This is a static function taking the serialized config (not `&self`)
    /// because the descriptor stores it as a function pointer.
    pub fn required_sudo_commands(_config: &serde_json::Value) -> Vec<SudoCommandEntry> {
        // `install` — copies release assets to the target path.
        vec![SudoCommandEntry::new(
            "install",
            "Install downloaded GitHub release assets to the target path",
        )]
    }

    /// Build the releases API URL for the given owner/repo pair.
    pub(crate) fn releases_url(&self, owner: &str, repo: &str) -> String {
        format!(
            "{}/repos/{}/{}/releases?per_page={PER_PAGE}",
            self.config.api_base_url(),
            owner,
            repo,
        )
    }

    /// Baseline release checks shared by `convert_release` and the
    /// filtered-vs-empty diagnostics in `fetch_releases`: drafts are always
    /// skipped; prereleases are skipped unless `include_prereleases` is set.
    fn passes_baseline(&self, release: &GitHubRelease) -> bool {
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

    /// Convert a GitHub API release to an `UpstreamRelease`, applying filters.
    ///
    /// Returns `None` if the release should be skipped (draft, filtered
    /// prerelease, outside the configured tag series, empty version after
    /// prefix stripping, or no assets survive `asset_patterns`).
    fn convert_release(&self, gh_release: &GitHubRelease) -> Option<UpstreamRelease> {
        if !self.passes_baseline(gh_release) {
            tracing::trace!(tag = %gh_release.tag_name, "skipping draft or filtered prerelease");
            return None;
        }

        // Series filter: when tag_prefix is set, the release must belong to
        // the series (literal prefix match) — other series in the same repo
        // are excluded entirely, not just stripped.
        if let Some(prefix) = self.config.tag_prefix.as_deref()
            && !prefix.is_empty()
            && !gh_release.tag_name.starts_with(prefix)
        {
            tracing::trace!(tag = %gh_release.tag_name, "skipping release outside tag_prefix series");
            return None;
        }

        // Strip order: tag_prefix first, then tag_strip_prefix, then parse —
        // so "…-standalone-" + "v" and "…-standalone-v" + "v" both yield the
        // same bare version.
        let after_series = strip_tag_prefix(
            &gh_release.tag_name,
            self.config.tag_prefix.as_deref().unwrap_or(""),
        );
        let version_str = strip_tag_prefix(after_series, &self.config.tag_strip_prefix);
        if version_str.is_empty() {
            tracing::trace!(tag = %gh_release.tag_name, "skipping release: empty version after prefix strip");
            return None;
        }
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

        // Asset gating: a configured asset filter that matches nothing means
        // this release has no installable artifact — drop the release instead
        // of surfacing an asset-less "update".
        if !self.asset_filters.is_empty() && assets.is_empty() {
            tracing::trace!(tag = %gh_release.tag_name, "skipping release: no assets match asset_patterns");
            return None;
        }

        Some({
            let mut r = UpstreamRelease::new(
                version,
                gh_release.tag_name.clone(),
                gh_release.prerelease,
                gh_release.html_url.clone(),
            );
            r.release_notes = gh_release.body.clone();
            r.published_at = published_at;
            r.assets = assets;
            r
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
        let client = match self.client() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "failed to build HTTP client for attestation check");
                return AttestationStatus::Unverified;
            }
        };
        let checksums_text = match client
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

        let response = match client.get(&url).send().await {
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

declare_plugin!(GitHubPlugin, GitHubConfig, "releases.github", {
    display_name: "GitHub Releases",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::CONTROLLER_ONLY,
    config_test: [ConfigTestKind::Connectivity],
    roles: [ReleaseFetcher, UpdateExecutor { host_requirements: HostRequirements::POSIX }],
    extra_capabilities: [PluginCapability::ControllerSideFetchReleases],
    sudo: GitHubPlugin::required_sudo_commands,
});

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for GitHubPlugin {
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

        let initial_url = self.releases_url(owner, repo);
        tracing::debug!(url = %initial_url, "fetching GitHub releases");

        let mut all_releases: Vec<GitHubRelease> = Vec::new();
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
                tracing::debug!(status = %status, "GitHub API returned error status");
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
                        "failed to read GitHub API response (cap {MAX_RELEASE_PAGE_BYTES} bytes): {e}"
                    )))
                })?;
            let page: Vec<GitHubRelease> = serde_json::from_slice(&body).map_err(|e| {
                report!(PluginError::Serialization(format!(
                    "failed to parse GitHub API response: {e}"
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

        let mut upstream_releases: Vec<UpstreamRelease> = all_releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        // count vs baseline exposes how many releases the series/asset
        // filters dropped, even when survivors remain.
        tracing::debug!(
            count = upstream_releases.len(),
            baseline = baseline_count,
            total = raw_count,
            "fetched GitHub releases"
        );

        if let Some(msg) = self.filtered_out_error(
            raw_count,
            baseline_count,
            upstream_releases.len(),
            window_exhausted,
        ) {
            bail!(PluginError::Configuration(msg));
        }

        // Attestation check for the latest (first) release.
        if self.config.verify_attestation
            && let Some(latest) = upstream_releases.first_mut()
            && let Some(gh_release) = all_releases.iter().find(|r| r.tag_name == latest.tag)
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

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for GitHubPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        _package_identifier: &str,
        _to_version: &str,
        release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_plugin_infrastructure_core::Result<ExecuteUpdateResult> {
        let install_path = self.config.install_path.as_deref().ok_or_else(|| {
            report!(PluginError::Configuration(
                "execute_update requires install_path to be configured".to_string()
            ))
        })?;

        let release = release_info.ok_or_else(|| report!(PluginError::MissingReleaseInfo))?;

        // ── 1. Select the matching asset ───────────────────────────────

        let matching_assets: Vec<&ReleaseAsset> = if self.asset_filters.is_empty() {
            release.assets.iter().collect()
        } else {
            release
                .assets
                .iter()
                .filter(|a| self.asset_filters.iter().any(|re| re.is_match(&a.name)))
                .collect()
        };

        if matching_assets.is_empty() {
            let available: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
            bail!(PluginError::InstallFailed(format!(
                "no asset matched the configured asset_patterns; available assets: [{}]",
                available.join(", ")
            )));
        }

        if matching_assets.len() > 1 {
            let names: Vec<&str> = matching_assets.iter().map(|a| a.name.as_str()).collect();
            bail!(PluginError::InstallFailed(format!(
                "asset_patterns matched {} assets (expected exactly 1): [{}]. \
                 Narrow the patterns or use a per-host config_override.",
                matching_assets.len(),
                names.join(", ")
            )));
        }

        let asset = matching_assets[0];

        if let Some(size) = asset.size
            && size > MAX_ASSET_DOWNLOAD_BYTES as u64
        {
            bail!(PluginError::InstallFailed(format!(
                "asset {name} declares {size} bytes, over the {MAX_ASSET_DOWNLOAD_BYTES}-byte download cap",
                name = asset.name
            )));
        }

        send_output(
            output_tx,
            &format!(
                "Selected asset: {} ({})",
                asset.name,
                format_size(asset.size)
            ),
            OutputStreamType::Stdout,
        )
        .await;

        // ── 2. Download asset into memory ──────────────────────────────

        send_output(
            output_tx,
            &format!("Downloading {} ...", asset.download_url),
            OutputStreamType::Stdout,
        )
        .await;

        // Build a download-specific client that follows redirects (GitHub
        // asset URLs redirect to a CDN).
        let download_resp = {
            let mut download_headers = reqwest::header::HeaderMap::new();
            download_headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/octet-stream"),
            );
            if let Some(ref token) = self.config.auth_token {
                let value = format!("Bearer {}", token.expose_secret());
                if let Ok(hv) = reqwest::header::HeaderValue::from_str(&value) {
                    download_headers.insert(reqwest::header::AUTHORIZATION, hv);
                }
            }

            let download_client = build_plugin_http_client(PluginHttpClientConfig {
                user_agent: concat!(
                    "uptrakit-plugin-releases-github/",
                    env!("CARGO_PKG_VERSION")
                ),
                default_headers: Some(download_headers),
                request_timeout_secs: 600,
                redirect: RedirectMode::Limited { hops: 10 },
                ..Default::default()
            })
            .map_err(|e| report!(PluginError::InstallFailed(e.to_string())))?;

            download_client
                .get(&asset.download_url)
                .send()
                .await
                .map_err(|e| {
                    report!(PluginError::InstallFailed(format!(
                        "asset download request failed: {e}"
                    )))
                })?
        };

        if !download_resp.status().is_success() {
            bail!(PluginError::InstallFailed(format!(
                "asset download returned HTTP {}",
                download_resp.status()
            )));
        }

        let body_bytes = read_bytes_capped(download_resp, MAX_ASSET_DOWNLOAD_BYTES)
            .await
            .map_err(|e| {
                report!(PluginError::InstallFailed(format!(
                    "failed to read asset body (cap {MAX_ASSET_DOWNLOAD_BYTES} bytes): {e}"
                )))
            })?;

        send_output(
            output_tx,
            &format!("Downloaded {} bytes", body_bytes.len()),
            OutputStreamType::Stdout,
        )
        .await;

        // ── 3. Verify SHA-256 checksum ─────────────────────────────────

        // Always compute SHA-256: used for checksum verification and to derive
        // a unique remote temp-file name.
        let mut hasher = Sha256::new();
        hasher.update(&body_bytes);
        let digest = hasher.finalize();
        let digest_hex = uptrakit_shared_types::hex::encode(digest);

        if let Some(ref expected_hex) = asset.sha256_digest {
            if digest_hex != *expected_hex {
                bail!(PluginError::InstallFailed(format!(
                    "SHA-256 checksum mismatch: expected {expected_hex}, got {digest_hex}"
                )));
            }
            send_output(
                output_tx,
                &format!("SHA-256 verified: {expected_hex}"),
                OutputStreamType::Stdout,
            )
            .await;
        } else {
            send_output(
                output_tx,
                "No SHA-256 checksum available; skipping verification",
                OutputStreamType::Stdout,
            )
            .await;
        }

        // ── 4. Upload asset to remote temp file ────────────────────────

        // Use the first 8 bytes of the SHA-256 as a unique suffix so that
        // concurrent uploads of different assets never collide.
        let remote_temp = format!("/tmp/.uptrakit-{}", &digest_hex[..16]);

        send_output(
            output_tx,
            &format!("Uploading to remote temp file {remote_temp} ..."),
            OutputStreamType::Stdout,
        )
        .await;

        // Stream the asset bytes directly into the remote `cat` process via
        // the SSH stdio tunnel.  This works for any executor that supports
        // `open_stdio_tunnel` — in practice all SSH-backed executors.
        let executor = self.executor.as_ref().ok_or_else(|| {
            report!(PluginError::Configuration(
                "execute_update requires a POSIX executor (not available on controller)"
                    .to_string()
            ))
        })?;

        {
            let tunnel_cmd = format!("cat > {remote_temp}");
            let mut tunnel = executor.open_stdio_tunnel(&tunnel_cmd).await.map_err(|e| {
                report!(PluginError::InstallFailed(format!(
                    "failed to open upload channel: {e}"
                )))
            })?;
            tunnel.write_all(&body_bytes).await.map_err(|e| {
                report!(PluginError::InstallFailed(format!(
                    "failed to upload asset: {e}"
                )))
            })?;
            tunnel.shutdown().await.map_err(|e| {
                report!(PluginError::InstallFailed(format!(
                    "failed to finalise upload: {e}"
                )))
            })?;
        }

        // ── 5. Install the file at the target path ─────────────────────

        let mode = if self.config.make_executable {
            "755"
        } else {
            "644"
        };

        send_output(
            output_tx,
            &format!("Installing to {install_path} (mode {mode})"),
            OutputStreamType::Stdout,
        )
        .await;

        let install_spec = CommandSpec::exec(
            "install",
            [
                "-m".to_string(),
                mode.to_string(),
                remote_temp.clone(),
                install_path.to_string(),
            ],
        )
        .privileged();

        let install_result = executor.execute(&install_spec, output_tx).await;

        // Best-effort cleanup of the remote temp file regardless of install outcome.
        let rm_spec = CommandSpec::exec("rm", ["-f".to_string(), remote_temp]);
        if let Err(e) = executor.execute_quiet(&rm_spec).await {
            tracing::warn!(error = %e, "failed to remove remote temp file (best-effort)");
        }

        install_result.map_err(|e| {
            report!(PluginError::InstallFailed(format!(
                "install command failed: {e}"
            )))
        })?;

        let summary = format!("Installed {} to {install_path}", asset.name);
        send_output(output_tx, &summary, OutputStreamType::Stdout).await;

        Ok(ExecuteUpdateResult::new(summary, false))
    }
}

/// Format an optional byte size for display.
fn format_size(size: Option<u64>) -> String {
    match size {
        Some(s) if s >= 1_048_576 => format!("{:.1} MB", s as f64 / 1_048_576.0),
        Some(s) if s >= 1024 => format!("{:.1} KB", s as f64 / 1024.0),
        Some(s) => format!("{s} bytes"),
        None => "unknown size".to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions use assert!(result.is_ok()) pattern"
    )]
    use super::*;
    use crate::api_types::{GitHubAsset, GitHubRelease};
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PluginHttpClientConfig, ReleaseFetcher, SsrfMode,
        StandardHostRuntime, UpdateExecutor as _, build_plugin_http_client,
    };

    fn test_config() -> GitHubConfig {
        GitHubConfig::default()
    }

    fn test_runtime() -> Arc<dyn HostRuntime> {
        let executor = Arc::new(uptrakit_plugin_infrastructure_core::LocalCommandExecutor)
            as Arc<dyn uptrakit_plugin_infrastructure_core::command::CommandExecutor>;
        let caps = HostCapabilities::default();
        Arc::new(StandardHostRuntime::new(executor, caps))
    }

    fn test_plugin() -> GitHubPlugin {
        GitHubPlugin::new(test_config(), test_runtime()).expect("valid config")
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
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
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
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
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
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
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
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");

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

    // ── tag_prefix series tests ───────────────────────────────────────────────

    fn series_plugin(tag_prefix: &str) -> GitHubPlugin {
        let config = GitHubConfig {
            tag_prefix: Some(tag_prefix.to_string()),
            ..GitHubConfig::default()
        };
        GitHubPlugin::new(config, test_runtime()).expect("valid config")
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
        let gh = make_release("uptrakit-controller-standalone-v0.0.7", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "0.0.7");
        assert_eq!(release.tag, "uptrakit-controller-standalone-v0.0.7");
    }

    #[test]
    fn strip_composition_prefix_including_v() {
        let plugin = series_plugin("uptrakit-controller-standalone-v");
        let gh = make_release("uptrakit-controller-standalone-v0.0.7", false, false);
        let release = plugin.convert_release(&gh).expect("should convert");
        assert_eq!(release.version.as_str(), "0.0.7");
    }

    #[test]
    fn tag_equal_to_composed_prefix_dropped() {
        let plugin = series_plugin("uptrakit-controller-standalone-");
        let gh = make_release("uptrakit-controller-standalone-v", false, false);
        assert!(plugin.convert_release(&gh).is_none());
    }

    // ── asset gating (D3) tests ───────────────────────────────────────────────

    fn release_with_asset(tag: &str, asset_name: &str) -> GitHubRelease {
        let mut gh = make_release(tag, false, false);
        gh.assets = vec![GitHubAsset {
            name: asset_name.to_string(),
            browser_download_url: format!("https://example.com/{asset_name}"),
            size: 42,
            content_type: None,
        }];
        gh
    }

    #[test]
    fn asset_gating_drops_release_without_matching_assets() {
        let config = GitHubConfig {
            asset_patterns: vec![r".*\.deb$".to_string()],
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let gh = release_with_asset("v1.0.0", "app.rpm");
        assert!(plugin.convert_release(&gh).is_none());
        let gh_ok = release_with_asset("v1.0.0", "app.deb");
        assert!(plugin.convert_release(&gh_ok).is_some());
    }

    #[test]
    fn no_asset_patterns_keeps_assetless_release() {
        let plugin = test_plugin();
        let gh = make_release("v1.0.0", false, false);
        assert!(plugin.convert_release(&gh).is_some());
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
        // Counts deliberately distinct from the 20x100=2000 window numbers so
        // no assert can pass by matching the wrong figure.
        let msg = plugin
            .filtered_out_error(170, 150, 0, true)
            .expect("error expected");
        assert!(msg.contains("exhausted"));
        assert!(msg.contains("2000"), "window bound must be named");
        assert!(msg.contains("170"), "raw count must be named");
        assert!(msg.contains("150"), "baseline count must be named");
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
        let config = GitHubConfig {
            asset_patterns: vec![r".*\.deb$".to_string()],
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let msg = plugin
            .filtered_out_error(3, 3, 0, false)
            .expect("error expected");
        assert!(msg.contains("asset_patterns"));
    }

    #[test]
    fn plugin_creation_succeeds_with_empty_config() {
        let config = GitHubConfig::default();
        assert!(GitHubPlugin::new(config, test_runtime()).is_ok());
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

    #[test]
    fn attestation_url_default_base() {
        let plugin = test_plugin();
        let url = plugin.attestation_url("owner", "repo", "abc123");
        assert_eq!(
            url,
            "https://api.github.com/repos/owner/repo/attestations/sha256:abc123"
        );
    }

    #[test]
    fn attestation_url_custom_base() {
        let config = GitHubConfig {
            api_base_url: Some("https://ghe.corp.com/api/v3".to_string()),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let url = plugin.attestation_url("org", "project", "deadbeef");
        assert_eq!(
            url,
            "https://ghe.corp.com/api/v3/repos/org/project/attestations/sha256:deadbeef"
        );
    }

    // ── execute_update tests ─────────────────────────────────────────────

    fn make_release_info(assets: Vec<ReleaseAsset>) -> ReleaseInfo {
        ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://github.com/owner/repo/releases/tag/v1.0.0".to_string(),
            assets,
            attestation_status: None,
            require_attestation: false,
        }
    }

    fn make_release_asset(name: &str, url: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            download_url: url.to_string(),
            size: Some(1024),
            content_type: None,
            sha256_digest: None,
        }
    }

    #[tokio::test]
    async fn execute_update_fails_without_install_path() {
        let plugin = test_plugin();
        let (tx, _rx) = mpsc::channel(100);
        let release = make_release_info(vec![make_release_asset("app", "https://example.com/app")]);
        let result = plugin
            .execute_update("owner/repo", "1.0.0", Some(&release), &tx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("install_path"));
    }

    #[tokio::test]
    async fn execute_update_fails_without_release_info() {
        let config = GitHubConfig {
            install_path: Some("/usr/local/bin/app".to_string()),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let (tx, _rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("owner/repo", "1.0.0", None, &tx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("release info"));
    }

    #[tokio::test]
    async fn execute_update_fails_no_matching_asset() {
        let config = GitHubConfig {
            install_path: Some("/usr/local/bin/app".to_string()),
            asset_patterns: vec![r".*windows.*".to_string()],
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let (tx, _rx) = mpsc::channel(100);
        let release = make_release_info(vec![
            make_release_asset("app-linux-amd64", "https://example.com/linux"),
            make_release_asset("app-linux-arm64", "https://example.com/arm"),
        ]);
        let result = plugin
            .execute_update("owner/repo", "1.0.0", Some(&release), &tx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("no asset matched"), "error: {err}");
    }

    #[tokio::test]
    async fn execute_update_fails_ambiguous_assets() {
        let config = GitHubConfig {
            install_path: Some("/usr/local/bin/app".to_string()),
            asset_patterns: vec![r".*linux.*".to_string()],
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let (tx, _rx) = mpsc::channel(100);
        let release = make_release_info(vec![
            make_release_asset("app-linux-amd64", "https://example.com/amd64"),
            make_release_asset("app-linux-arm64", "https://example.com/arm64"),
        ]);
        let result = plugin
            .execute_update("owner/repo", "1.0.0", Some(&release), &tx)
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("matched 2 assets"), "error: {err}");
    }

    /// Declared asset size over the download cap must be rejected before any
    /// network request is made — the download mock must receive zero hits.
    ///
    /// Correctness of the enforced read cap itself at 512 MiB is NOT
    /// integration-tested here (no 512 MiB fixtures); that is covered by
    /// `read_bytes_capped`'s own unit tests in
    /// `uptrakit-plugin-infrastructure-core`.
    #[tokio::test]
    async fn oversized_asset_is_rejected_before_download() {
        let server = httpmock::MockServer::start_async().await;
        let download_mock = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET).path("/app");
                then.status(200).body("irrelevant");
            })
            .await;

        let config = GitHubConfig {
            install_path: Some("/usr/local/bin/app".to_string()),
            ..GitHubConfig::default()
        };
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        let (tx, _rx) = mpsc::channel(100);

        let mut asset = make_release_asset("app", &format!("{}/app", server.base_url()));
        asset.size = Some(549_755_813_888); // 512 GiB
        let release = make_release_info(vec![asset]);

        let result = plugin
            .execute_update("owner/repo", "1.0.0", Some(&release), &tx)
            .await;

        let err = result.expect_err("oversized asset must be rejected");
        assert!(
            matches!(err.current_context(), PluginError::InstallFailed(_)),
            "expected InstallFailed, got: {err:?}"
        );
        download_mock.assert_calls_async(0).await;
    }

    // ── required_sudo_commands tests ─────────────────────────────────────

    #[test]
    fn required_sudo_commands_returns_install_only() {
        let cmds = GitHubPlugin::required_sudo_commands(&serde_json::json!({}));
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].command, "install");
    }

    // ── format_size tests ────────────────────────────────────────────────

    #[test]
    fn format_size_bytes() {
        assert_eq!(format_size(Some(500)), "500 bytes");
    }

    #[test]
    fn format_size_kilobytes() {
        assert_eq!(format_size(Some(2048)), "2.0 KB");
    }

    #[test]
    fn format_size_megabytes() {
        assert_eq!(format_size(Some(5_242_880)), "5.0 MB");
    }

    #[test]
    fn format_size_none() {
        assert_eq!(format_size(None), "unknown size");
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

    // ── descriptor tests ────────────────────────────────────────────────────

    #[test]
    fn descriptor_plugin_type_id() {
        assert_eq!(DESCRIPTOR.type_id, "releases.github");
    }

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
                .contains(&PluginCapability::UpdateExecution)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ControllerSideFetchReleases)
        );
    }

    #[test]
    fn descriptor_roles() {
        assert!(DESCRIPTOR.roles.release_fetcher.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.version_detector.is_none());
    }

    // ── wired fetch_releases seam tests (httpmock) ────────────────────────────

    fn plugin_for_mock(config: GitHubConfig) -> GitHubPlugin {
        let plugin = GitHubPlugin::new(config, test_runtime()).expect("valid config");
        // Same-crate test: seed the lazy client cache with a permissive-SSRF
        // client so the plugin can reach the httpmock server on 127.0.0.1
        // (idiom from the npm plugin's release tests).
        let client = build_plugin_http_client(PluginHttpClientConfig {
            user_agent: "uptrakit-plugin-releases-github-test",
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
                    .path("/repos/o/r/releases");
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
        let plugin = plugin_for_mock(GitHubConfig {
            api_base_url: Some(server.base_url()),
            tag_prefix: Some("uptrakit-controller-standalone-".to_string()),
            ..GitHubConfig::default()
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
                    .path("/repos/o/r/releases");
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
        let plugin = plugin_for_mock(GitHubConfig {
            api_base_url: Some(server.base_url()),
            tag_prefix: Some("uptrakit-controller-standalone-".to_string()),
            ..GitHubConfig::default()
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
                    .path("/repos/o/r/releases");
                then.status(200).body(&over_cap_body);
            })
            .await;
        let plugin = plugin_for_mock(GitHubConfig {
            api_base_url: Some(server.base_url()),
            ..GitHubConfig::default()
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

    /// A hostile forge can point `Link: ...; rel="next"` at any origin, carrying
    /// our authenticated pagination request wherever it says. `rebase_to_origin`
    /// pins pagination to the request origin, keeping only the candidate's path
    /// and query — this proves page 2 is fetched from the mock server itself,
    /// never from the attacker-controlled origin the header names.
    ///
    /// gitlab's and forgejo's pagination glue use the identical
    /// `rebase_to_origin` call; the origin-pinning behavior itself is covered
    /// by that shared helper's unit tests in `http_client.rs`.
    #[tokio::test]
    async fn fetch_releases_rebases_cross_origin_pagination_link_onto_request_origin() {
        let server = httpmock::MockServer::start_async().await;
        let page1 = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/releases");
                then.status(200)
                    .header(
                        "link",
                        r#"<http://evil.example/api/steal?page=2>; rel="next""#,
                    )
                    .json_body(serde_json::json!([{
                        "tag_name": "v1.0.0",
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
        let page2 = server
            .mock_async(|when, then| {
                when.method(httpmock::Method::GET)
                    .path("/api/steal")
                    .query_param("page", "2");
                then.status(200).json_body(serde_json::json!([]));
            })
            .await;

        let plugin = plugin_for_mock(GitHubConfig {
            api_base_url: Some(server.base_url()),
            ..GitHubConfig::default()
        });
        let releases = plugin
            .fetch_releases("o/r")
            .await
            .expect("page 1's single release must survive");
        assert_eq!(releases.len(), 1);

        page1.assert_async().await;
        assert_eq!(
            page2.calls_async().await,
            1,
            "rebased next-page URL must be fetched from the mock server, not evil.example"
        );
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
                "/repos/o/r/releases".to_string()
            } else {
                format!("/repos/o/r/releases/p{n}")
            };
            let next_page = n + 1;
            let link_header = format!(
                r#"<{}/repos/o/r/releases/p{next_page}>; rel="next""#,
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

        let plugin = plugin_for_mock(GitHubConfig {
            api_base_url: Some(server.base_url()),
            ..GitHubConfig::default()
        });
        let err = plugin
            .fetch_releases("o/r")
            .await
            .expect_err("runaway Link chain must be rejected");
        assert!(
            matches!(err.current_context(), PluginError::Configuration(_)),
            "expected Configuration error, got: {err:?}"
        );

        // Exactly MAX_RELEASE_PAGES pages were fetched; the page just past
        // the cap (index total_pages - 1, i.e. page total_pages) must never
        // be requested.
        mocks
            .last()
            .expect("at least one mock registered")
            .assert_calls_async(0)
            .await;
    }

    /// A single page whose body claims an implausible number of releases
    /// must also be rejected — the invariant must not depend solely on the
    /// page-count cap.
    ///
    /// gitlab's and forgejo's pagination loops apply the identical
    /// `MAX_TOTAL_RELEASES` check; this test is not duplicated per-crate
    /// since the glue is byte-for-byte the same.
    #[tokio::test]
    async fn pagination_stops_at_cumulative_release_cap() {
        let server = httpmock::MockServer::start_async().await;
        let mut releases = String::from("[");
        for n in 0..=MAX_TOTAL_RELEASES {
            if n > 0 {
                releases.push(',');
            }
            releases.push_str(&format!(
                r#"{{"tag_name":"v1.0.{n}","name":null,"draft":false,"prerelease":false,"html_url":"https://example.com/r/v1.0.{n}","body":null,"published_at":null,"assets":[]}}"#
            ));
        }
        releases.push(']');

        let mock = server
            .mock_async(move |when, then| {
                when.method(httpmock::Method::GET)
                    .path("/repos/o/r/releases");
                then.status(200).body(&releases);
            })
            .await;

        let plugin = plugin_for_mock(GitHubConfig {
            api_base_url: Some(server.base_url()),
            ..GitHubConfig::default()
        });
        let err = plugin
            .fetch_releases("o/r")
            .await
            .expect_err("over-cap release count must be rejected");
        assert!(
            matches!(err.current_context(), PluginError::Configuration(_)),
            "expected Configuration error, got: {err:?}"
        );
        mock.assert_async().await;
    }
}
