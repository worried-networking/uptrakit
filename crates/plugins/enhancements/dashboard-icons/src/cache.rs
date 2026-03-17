use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::RwLock;
use rootcause::prelude::*;
use tokio_util::sync::CancellationToken;

use crate::error::{DashboardIconsError, Result};
use crate::slugify::slugify;

/// CDN base URL for Dashboard Icons SVG assets.
const CDN_BASE_URL: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg";

/// GitHub API endpoint for the repository tree.
const GITHUB_TREE_URL: &str =
    "https://api.github.com/repos/homarr-labs/dashboard-icons/git/trees/main?recursive=1";

/// How often to refresh the icon index (6 hours).
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Pre-cached set of icon slugs from the Dashboard Icons repository.
pub struct DashboardIconCache {
    slugs: RwLock<HashSet<String>>,
    client: reqwest::Client,
}

impl DashboardIconCache {
    /// Build a new cache with the given HTTP client.
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            slugs: RwLock::new(HashSet::new()),
            client,
        }
    }

    /// Build a cache pre-populated with the given slug set (for testing).
    #[cfg(test)]
    pub(crate) fn new_with_slugs(client: reqwest::Client, slugs: HashSet<String>) -> Self {
        Self {
            slugs: RwLock::new(slugs),
            client,
        }
    }

    /// Fetch the icon index from GitHub and populate the slug set.
    pub(crate) async fn refresh(&self) -> Result<usize> {
        let resp = self
            .client
            .get(GITHUB_TREE_URL)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| report!(DashboardIconsError::IndexFetch(e.to_string())))?;

        if !resp.status().is_success() {
            bail!(DashboardIconsError::IndexFetch(format!(
                "HTTP {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| report!(DashboardIconsError::IndexParse(e.to_string())))?;

        let tree = body["tree"]
            .as_array()
            .ok_or_else(|| report!(DashboardIconsError::IndexParse("missing tree".to_string())))?;

        let mut new_slugs = HashSet::new();
        for entry in tree {
            if let Some(path) = entry["path"].as_str() {
                // We only index `svg/<name>-light.svg` files to avoid
                // duplicates between light/dark variants.
                if let Some(name) = path
                    .strip_prefix("svg/")
                    .and_then(|p| p.strip_suffix("-light.svg"))
                {
                    new_slugs.insert(name.to_string());
                }
            }
        }

        let count = new_slugs.len();
        *self.slugs.write() = new_slugs;

        tracing::info!(count, "dashboard icons index refreshed");
        Ok(count)
    }

    /// Look up a software name and return the CDN URL if a matching icon exists.
    pub fn lookup(&self, name: &str) -> Option<String> {
        let slug = slugify(name);
        if slug.is_empty() {
            return None;
        }

        let slugs = self.slugs.read();
        if slugs.contains(&slug) {
            Some(format!("{CDN_BASE_URL}/{slug}.svg"))
        } else {
            None
        }
    }

    /// Spawn a background loop that refreshes the cache on startup and then
    /// periodically.
    pub fn spawn_refresh_loop(cache: Arc<Self>, cancel: CancellationToken) {
        tokio::spawn(async move {
            // Initial refresh — log errors but don't crash.
            if let Err(e) = cache.refresh().await {
                tracing::warn!(error = %e, "initial dashboard icons refresh failed");
            }

            loop {
                tokio::select! {
                    () = cancel.cancelled() => {
                        tracing::debug!("dashboard icons refresh loop shutting down");
                        return;
                    }
                    () = tokio::time::sleep(REFRESH_INTERVAL) => {
                        if let Err(e) = cache.refresh().await {
                            tracing::warn!(error = %e, "periodic dashboard icons refresh failed");
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_with_slugs(slugs: &[&str]) -> DashboardIconCache {
        let client = reqwest::Client::new();
        let set: HashSet<String> = slugs.iter().map(|s| s.to_string()).collect();
        DashboardIconCache {
            slugs: RwLock::new(set),
            client,
        }
    }

    #[test]
    fn lookup_known_slug() {
        let cache = cache_with_slugs(&["nginx", "grafana", "redis"]);
        assert_eq!(
            cache.lookup("Nginx"),
            Some("https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/nginx.svg".into())
        );
    }

    #[test]
    fn lookup_unknown_slug() {
        let cache = cache_with_slugs(&["nginx"]);
        assert_eq!(cache.lookup("SomeRandomApp"), None);
    }

    #[test]
    fn lookup_empty_name() {
        let cache = cache_with_slugs(&["nginx"]);
        assert_eq!(cache.lookup(""), None);
    }

    #[test]
    fn lookup_with_spaces() {
        let cache = cache_with_slugs(&["home-assistant"]);
        assert_eq!(
            cache.lookup("Home Assistant"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/home-assistant.svg"
                    .into()
            )
        );
    }
}
