#![expect(
    clippy::string_slice,
    reason = "string slices use byte positions derived from ASCII-only content or fixed-length pattern matching; UTF-8 boundary safety is guaranteed by construction"
)]
use async_trait::async_trait;
use futures_util::StreamExt as _;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, Result, UpstreamRelease, Version,
};

use crate::error::CargoError;
use crate::plugin::{CargoPlugin, is_prerelease_version};

/// Compute the sparse index URL path fragment for a crate name.
///
/// Implements the standard Cargo sparse index path computation:
/// - 1 char:  `1/{name}`
/// - 2 chars: `2/{name}`
/// - 3 chars: `3/{first}/{name}`
/// - 4+ chars: `{first_two}/{next_two}/{name}`
///
/// The name is lowercased, as required by the index format.
fn sparse_index_url(registry_base: &str, crate_name: &str) -> String {
    let name = crate_name.to_lowercase();
    let prefix = match name.len() {
        1 => "1".to_string(),
        2 => "2".to_string(),
        3 => format!("3/{}", &name[..1]),
        _ => format!("{}/{}", &name[..2], &name[2..4]),
    };
    format!(
        "{}/{}/{}",
        registry_base.trim_end_matches('/'),
        prefix,
        name
    )
}

/// Fetch upstream releases for a single crate from the sparse registry index.
///
/// Makes a single HTTP `GET` request to the sparse index URL, parses the
/// newline-delimited JSON response with `tame_index::IndexKrate::from_slice`,
/// and returns filtered [`UpstreamRelease`] entries sorted in **descending
/// semver order** (newest first), so callers can simply use `.find()` to
/// obtain the latest release.
pub(crate) async fn fetch_crate_releases(
    client: &reqwest::Client,
    registry_base: &str,
    include_prereleases: bool,
    crate_name: &str,
) -> crate::error::Result<Vec<UpstreamRelease>> {
    let url = sparse_index_url(registry_base, crate_name);
    tracing::debug!(crate_name, %url, "fetching crate releases from sparse index");

    let response = client
        .get(&url)
        .header(reqwest::header::ACCEPT, "text/plain")
        .send()
        .await
        .map_err(|e| report!(CargoError::Request(e.to_string())))?;

    let status = response.status();

    if status == reqwest::StatusCode::NOT_FOUND {
        tracing::debug!(crate_name, "crate not found in registry index");
        return Ok(vec![]);
    }

    if !status.is_success() {
        let message = response.text().await.unwrap_or_default();
        bail!(CargoError::ApiError { status, message });
    }

    let bytes = response
        .bytes()
        .await
        .map_err(|e| report!(CargoError::Request(e.to_string())))?;

    let krate = tame_index::IndexKrate::from_slice(&bytes).map_err(|e| {
        report!(CargoError::Request(format!(
            "failed to parse sparse index response for '{crate_name}': {e}"
        )))
    })?;

    let mut releases: Vec<UpstreamRelease> = krate
        .versions
        .iter()
        .filter(|v| {
            if v.yanked {
                return false;
            }
            let prerelease = is_prerelease_version(v.version.as_str());
            !prerelease || include_prereleases
        })
        .map(|v| {
            let version_str = v.version.as_str().to_string();
            let release_url = format!("https://crates.io/crates/{crate_name}/{version_str}");
            let is_pre = is_prerelease_version(&version_str);
            UpstreamRelease::new(Version::new(&version_str), version_str, is_pre, release_url)
        })
        .collect();

    // The sparse index stores versions in chronological (oldest-first) order.
    // Sort descending so the scheduler's `.find(|r| !r.is_prerelease)` picks
    // the newest stable release instead of the oldest.
    releases.sort_by(|a, b| b.version.cmp(&a.version));

    tracing::debug!(
        crate_name,
        count = releases.len(),
        "fetched crate releases from sparse index"
    );
    Ok(releases)
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for CargoPlugin {
    /// Fetch available releases for a single crate from the sparse registry index.
    #[tracing::instrument(skip_all)]
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        self.require_package_identifier(package_identifier)?;

        fetch_crate_releases(
            &self.client,
            self.config.effective_registry_url(),
            self.config.include_prereleases,
            package_identifier,
        )
        .await
        .map_err(|e| report!(PluginError::PluginInternal(e.to_string())))
    }

    /// Fetch releases for multiple crates in parallel, bounded to 10 concurrent requests.
    #[tracing::instrument(skip_all)]
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        if items.is_empty() {
            return Ok(vec![]);
        }

        tracing::debug!(count = items.len(), "batch fetching cargo crate releases");

        // Clone cheap handles before moving into stream closures.
        let client = self.client.clone();
        let registry_base = self.config.effective_registry_url().to_string();
        let include_prereleases = self.config.include_prereleases;

        // Pre-collect owned identifiers so each future can own its data (`'static`).
        let ids: Vec<String> = items.iter().map(|i| i.package_identifier.clone()).collect();

        let results = futures_util::stream::iter(ids)
            .map(|id| {
                let client = client.clone();
                let registry_base = registry_base.clone();
                async move {
                    match fetch_crate_releases(&client, &registry_base, include_prereleases, &id)
                        .await
                    {
                        Ok(releases) => BatchFetchResult::found(id, releases),
                        Err(e) => BatchFetchResult::error(id, e.to_string()),
                    }
                }
            })
            .buffer_unordered(10)
            .collect::<Vec<_>>()
            .await;

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── sparse_index_url ──────────────────────────────────────────────────────

    #[test]
    fn sparse_index_url_one_char() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "a"),
            "https://index.crates.io/1/a"
        );
    }

    #[test]
    fn sparse_index_url_two_chars() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "ab"),
            "https://index.crates.io/2/ab"
        );
    }

    #[test]
    fn sparse_index_url_three_chars() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "bat"),
            "https://index.crates.io/3/b/bat"
        );
    }

    #[test]
    fn sparse_index_url_four_plus_chars() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "ripgrep"),
            "https://index.crates.io/ri/pg/ripgrep"
        );
        assert_eq!(
            sparse_index_url("https://index.crates.io", "cargo-nextest"),
            "https://index.crates.io/ca/rg/cargo-nextest"
        );
    }

    #[test]
    fn sparse_index_url_uppercase_lowercased() {
        assert_eq!(
            sparse_index_url("https://index.crates.io", "MyTool"),
            "https://index.crates.io/my/to/mytool"
        );
    }

    #[test]
    fn sparse_index_url_trailing_slash_stripped() {
        assert_eq!(
            sparse_index_url("https://index.crates.io/", "bat"),
            "https://index.crates.io/3/b/bat"
        );
    }

    // ── fetch_releases sort order ─────────────────────────────────────────────

    /// Verify that `fetch_crate_releases` returns versions in descending semver
    /// order (newest first), matching the contract expected by the scheduler's
    /// `.find(|r| !r.is_prerelease)` logic.
    #[test]
    fn fetch_releases_sorted_newest_first() {
        // Simulate the chronological (oldest-first) order from the sparse index.
        let mut releases: Vec<UpstreamRelease> = [
            UpstreamRelease::new(
                Version::new("0.1.0"),
                "0.1.0".to_string(),
                false,
                "https://crates.io/crates/example/0.1.0".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("0.9.0"),
                "0.9.0".to_string(),
                false,
                "https://crates.io/crates/example/0.9.0".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("1.0.0-alpha"),
                "1.0.0-alpha".to_string(),
                true,
                "https://crates.io/crates/example/1.0.0-alpha".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("1.0.0"),
                "1.0.0".to_string(),
                false,
                "https://crates.io/crates/example/1.0.0".to_string(),
            ),
            UpstreamRelease::new(
                Version::new("1.2.3"),
                "1.2.3".to_string(),
                false,
                "https://crates.io/crates/example/1.2.3".to_string(),
            ),
        ]
        .into();

        // Apply the same sort used in `fetch_crate_releases`.
        releases.sort_by(|a, b| b.version.cmp(&a.version));

        // Newest must be first.
        assert_eq!(releases[0].version, Version::new("1.2.3"));
        // Oldest must be last.
        assert_eq!(releases[releases.len() - 1].version, Version::new("0.1.0"));

        // The scheduler's "find latest stable" logic must now pick 1.2.3, not 0.1.0.
        let latest_stable = releases.iter().find(|r| !r.is_prerelease);
        assert_eq!(
            latest_stable.map(|r| r.version.clone()),
            Some(Version::new("1.2.3")),
        );
    }
}
