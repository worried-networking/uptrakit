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
    AttestationStatus, ConfigModel, ConfigTestKind, HostRequirements, HostRuntime,
    OutputStreamType, PluginCapability, PluginError, PluginFamily, ReleaseAsset, ReleaseInfo,
    SudoCommandEntry, UpdateOutputLine, UpstreamRelease, Version, declare_plugin,
};
use uptrakit_plugin_infrastructure_core::{PluginHttpClientConfig, build_plugin_http_client};

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

        build_plugin_http_client(PluginHttpClientConfig {
            user_agent: concat!(
                "uptrakit-plugin-releases-github/",
                env!("CARGO_PKG_VERSION")
            ),
            default_headers: Some(headers),
            ..Default::default()
        })
        .map_err(|e| report!(GitHubError::Request(e)))
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

declare_plugin!(GitHubPlugin, GitHubConfig, "releases_github", {
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

        const MAX_PAGES: usize = 10;
        let mut all_releases: Vec<GitHubRelease> = Vec::new();
        let mut url = initial_url;

        'pages: for _ in 0..MAX_PAGES {
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

            let next_url = parse_link_next(response.headers());
            let page: Vec<GitHubRelease> = response.json().await.map_err(|e| {
                report!(PluginError::Serialization(format!(
                    "failed to parse GitHub API response: {e}"
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

        let mut upstream_releases: Vec<UpstreamRelease> = all_releases
            .iter()
            .filter_map(|r| self.convert_release(r))
            .collect();

        tracing::debug!(
            count = upstream_releases.len(),
            total = all_releases.len(),
            "fetched GitHub releases"
        );

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
    ) -> uptrakit_plugin_infrastructure_core::Result<String> {
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
                redirect_policy: reqwest::redirect::Policy::limited(10),
                ..Default::default()
            })
            .map_err(|e| report!(PluginError::InstallFailed(e)))?;

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

        let body_bytes = download_resp.bytes().await.map_err(|e| {
            report!(PluginError::InstallFailed(format!(
                "failed to read asset body: {e}"
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
        let digest_hex = format!("{digest:x}");

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

        Ok(summary)
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
    use super::*;
    use crate::api_types::{GitHubAsset, GitHubRelease};
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, PluginCapability, PosixHostRuntime, UpdateExecutor as _,
    };

    fn test_config() -> GitHubConfig {
        GitHubConfig::default()
    }

    fn test_runtime() -> Arc<dyn HostRuntime> {
        let executor = Arc::new(uptrakit_plugin_infrastructure_core::LocalCommandExecutor)
            as Arc<dyn uptrakit_plugin_infrastructure_core::command::CommandExecutor>;
        let caps = HostCapabilities::default();
        Arc::new(PosixHostRuntime::new(executor, caps))
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
        assert_eq!(DESCRIPTOR.type_id, "releases_github");
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
}
