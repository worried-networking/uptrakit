use std::collections::HashMap;
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct IconVariants {
    plain: bool,
    light: bool,
    dark: bool,
}

impl IconVariants {
    fn register_path(&mut self, path: &str) -> bool {
        let Some(name) = path.strip_prefix("svg/") else {
            return false;
        };

        if name.is_empty() {
            return false;
        }

        if name.ends_with("-light.svg") {
            self.light = true;
            return true;
        }

        if name.ends_with("-dark.svg") {
            self.dark = true;
            return true;
        }

        if name.ends_with(".svg") {
            self.plain = true;
            return true;
        }

        false
    }

    fn preferred_filename(self, slug: &str) -> Option<String> {
        if self.light {
            return Some(format!("{slug}-light.svg"));
        }
        if self.plain {
            return Some(format!("{slug}.svg"));
        }
        if self.dark {
            return Some(format!("{slug}-dark.svg"));
        }
        None
    }
}

fn slug_from_path(path: &str) -> Option<&str> {
    let name = path.strip_prefix("svg/")?;
    name.strip_suffix("-light.svg")
        .or_else(|| name.strip_suffix("-dark.svg"))
        .or_else(|| name.strip_suffix(".svg"))
}

/// Pre-cached set of icon slugs and their available variants.
pub struct DashboardIconCache {
    slugs: RwLock<HashMap<String, IconVariants>>,
    client: reqwest::Client,
}

impl DashboardIconCache {
    /// Build a new cache with the given HTTP client.
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            slugs: RwLock::new(HashMap::new()),
            client,
        }
    }

    /// Build a cache pre-populated with the given icon paths (for testing).
    #[cfg(test)]
    pub(crate) fn new_with_paths(client: reqwest::Client, paths: &[&str]) -> Self {
        let mut slugs: HashMap<String, IconVariants> = HashMap::new();
        for path in paths {
            let Some(slug) = slug_from_path(path) else {
                continue;
            };
            let entry = slugs.entry(slug.to_string()).or_default();
            let _ = entry.register_path(path);
        }
        Self {
            slugs: RwLock::new(slugs),
            client,
        }
    }

    /// Fetch the icon index from GitHub and populate the slug set.
    pub(crate) async fn refresh(&self) -> Result<usize> {
        self.refresh_from_url(GITHUB_TREE_URL).await
    }

    async fn refresh_from_url(&self, tree_url: &str) -> Result<usize> {
        let resp = self
            .client
            .get(tree_url)
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

        let mut new_slugs = HashMap::<String, IconVariants>::new();
        for entry in tree {
            if let Some(path) = entry["path"].as_str() {
                let Some(slug) = slug_from_path(path) else {
                    continue;
                };
                let variants = new_slugs.entry(slug.to_string()).or_default();
                let _ = variants.register_path(path);
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
        tracing::trace!(name, slug, "dashboard icons lookup");
        if slug.is_empty() {
            return None;
        }

        let slugs = self.slugs.read();
        if let Some(variants) = slugs.get(&slug).copied() {
            tracing::trace!(name, slug, ?variants, "dashboard icons cache hit");
            variants
                .preferred_filename(&slug)
                .map(|filename| format!("{CDN_BASE_URL}/{filename}"))
        } else {
            tracing::trace!(name, slug, "dashboard icons cache miss");
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
    use httpmock::prelude::*;

    fn cache_with_paths(paths: &[&str]) -> DashboardIconCache {
        DashboardIconCache::new_with_paths(reqwest::Client::new(), paths)
    }

    #[test]
    fn lookup_known_slug() {
        let cache = cache_with_paths(&["svg/nginx.svg", "svg/grafana.svg", "svg/redis.svg"]);
        assert_eq!(
            cache.lookup("Nginx"),
            Some("https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/nginx.svg".into())
        );
    }

    #[test]
    fn lookup_unknown_slug() {
        let cache = cache_with_paths(&["svg/nginx.svg"]);
        assert_eq!(cache.lookup("SomeRandomApp"), None);
    }

    #[test]
    fn lookup_empty_name() {
        let cache = cache_with_paths(&["svg/nginx.svg"]);
        assert_eq!(cache.lookup(""), None);
    }

    #[test]
    fn lookup_with_spaces() {
        let cache = cache_with_paths(&["svg/home-assistant.svg"]);
        assert_eq!(
            cache.lookup("Home Assistant"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/home-assistant.svg"
                    .into()
            )
        );
    }

    #[test]
    fn lookup_prefers_light_variant_when_available() {
        let cache = cache_with_paths(&[
            "svg/actual-budget.svg",
            "svg/actual-budget-light.svg",
            "svg/actual-budget-dark.svg",
        ]);
        assert_eq!(
            cache.lookup("Actual Budget"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/actual-budget-light.svg"
                    .into()
            )
        );
    }

    #[test]
    fn lookup_falls_back_to_plain_when_light_missing() {
        let cache = cache_with_paths(&["svg/actual-budget.svg", "svg/actual-budget-dark.svg"]);
        assert_eq!(
            cache.lookup("Actual Budget"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/actual-budget.svg"
                    .into()
            )
        );
    }

    #[test]
    fn lookup_falls_back_to_dark_when_only_dark_exists() {
        let cache = cache_with_paths(&["svg/actual-budget-dark.svg"]);
        assert_eq!(
            cache.lookup("Actual Budget"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/actual-budget-dark.svg"
                    .into()
            )
        );
    }

    #[tokio::test]
    async fn refresh_loads_all_icon_variants_from_mocked_tree_endpoint() {
        let server = MockServer::start();
        let tree_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/git/trees/main")
                .query_param("recursive", "1")
                .header("accept", "application/vnd.github+json");
            then.status(200).json_body(serde_json::json!({
                "tree": [
                    { "path": "svg/actual-budget.svg" },
                    { "path": "svg/actual-budget-light.svg" },
                    { "path": "svg/nginx-dark.svg" },
                    { "path": "svg/plain-only.svg" }
                ]
            }));
        });

        let client = reqwest::Client::new();
        let cache = DashboardIconCache::new(client);
        let count = cache
            .refresh_from_url(&format!("{}/git/trees/main?recursive=1", server.base_url()))
            .await
            .expect("refresh should succeed");

        assert_eq!(count, 3);
        assert_eq!(
            cache.lookup("Actual Budget"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/actual-budget-light.svg"
                    .into()
            )
        );
        assert_eq!(
            cache.lookup("Nginx"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/nginx-dark.svg".into()
            )
        );
        assert_eq!(
            cache.lookup("Plain Only"),
            Some(
                "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg/plain-only.svg".into()
            )
        );
        tree_mock.assert();
    }
}
