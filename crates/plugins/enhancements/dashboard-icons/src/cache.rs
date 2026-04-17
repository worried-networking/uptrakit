use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::RwLock;
use rootcause::prelude::*;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uptrakit_global_github_provider::{DASHBOARD_ICONS, GitHubProviderClient};
#[cfg(test)]
use uptrakit_global_github_provider::{
    GitHubProviderError, GitHubRepositoryTree, GlobalProviderConsumerId,
};

use crate::error::{DashboardIconsError, Result};
use crate::slugify::slugify;

/// CDN base URL for Dashboard Icons SVG assets.
const CDN_BASE_URL: &str = "https://cdn.jsdelivr.net/gh/homarr-labs/dashboard-icons/svg";

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
    github_provider: Arc<dyn GitHubProviderClient>,
    index_ready: AtomicBool,
    refresh_attempted: AtomicBool,
    refresh_lock: Mutex<()>,
}

impl DashboardIconCache {
    /// Build a new cache backed by the injected global GitHub provider.
    pub fn new(github_provider: Arc<dyn GitHubProviderClient>) -> Self {
        Self {
            slugs: RwLock::new(HashMap::new()),
            github_provider,
            index_ready: AtomicBool::new(false),
            refresh_attempted: AtomicBool::new(false),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Build a cache pre-populated with the given icon paths (for testing).
    #[cfg(test)]
    pub(crate) fn new_with_paths(paths: &[&str]) -> Self {
        struct UnusedProvider;

        #[async_trait::async_trait]
        impl GitHubProviderClient for UnusedProvider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                unreachable!("new_with_paths should not fetch from the provider")
            }
        }

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
            github_provider: Arc::new(UnusedProvider),
            index_ready: AtomicBool::new(true),
            refresh_attempted: AtomicBool::new(true),
            refresh_lock: Mutex::new(()),
        }
    }

    /// Fetch the icon index from GitHub and populate the slug set.
    pub(crate) async fn refresh(&self) -> Result<usize> {
        let count = self.refresh_from_provider(&self.github_provider).await?;
        self.index_ready.store(true, Ordering::Release);
        Ok(count)
    }

    /// Look up an icon and perform one on-demand refresh if the index is still cold.
    pub(crate) async fn lookup_or_try_refresh(&self, name: &str) -> Option<String> {
        if let Some(url) = self.lookup(name) {
            return Some(url);
        }

        if self.index_ready.load(Ordering::Acquire) {
            return None;
        }

        if self.refresh_attempted.swap(true, Ordering::AcqRel) {
            return None;
        }

        let _guard = self.refresh_lock.lock().await;
        if let Some(url) = self.lookup(name) {
            return Some(url);
        }

        if let Err(error) = self.refresh().await {
            tracing::warn!(error = %error, "dashboard icons cold-start refresh failed");
            return None;
        }

        self.lookup(name)
    }

    async fn refresh_from_provider(
        &self,
        provider: &Arc<dyn GitHubProviderClient>,
    ) -> Result<usize> {
        let tree = provider
            .fetch_repository_tree(
                DASHBOARD_ICONS,
                "homarr-labs",
                "dashboard-icons",
                "main",
                true,
            )
            .await
            .map_err(|e| report!(DashboardIconsError::IndexFetch(e.to_string())))?;
        let count = self.rebuild_from_paths(tree.entries.iter().map(|entry| entry.path.as_str()));
        tracing::info!(count, "dashboard icons index refreshed");
        Ok(count)
    }

    fn rebuild_from_paths<'a>(&self, paths: impl IntoIterator<Item = &'a str>) -> usize {
        let mut new_slugs = HashMap::<String, IconVariants>::new();
        for path in paths {
            let Some(slug) = slug_from_path(path) else {
                continue;
            };
            let variants = new_slugs.entry(slug.to_string()).or_default();
            let _ = variants.register_path(path);
        }

        let count = new_slugs.len();
        *self.slugs.write() = new_slugs;
        count
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uptrakit_global_github_provider::{
        GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntry,
        GitHubTreeEntryKind, GlobalProviderConsumerId,
    };

    fn cache_with_paths(paths: &[&str]) -> DashboardIconCache {
        DashboardIconCache::new_with_paths(paths)
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
    async fn refresh_loads_all_icon_variants_from_provider_tree() {
        struct VariantsProvider;

        #[async_trait::async_trait]
        impl GitHubProviderClient for VariantsProvider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Ok(GitHubRepositoryTree {
                    truncated: false,
                    entries: vec![
                        GitHubTreeEntry {
                            path: "svg/actual-budget.svg".to_string(),
                            kind: GitHubTreeEntryKind::Blob,
                        },
                        GitHubTreeEntry {
                            path: "svg/actual-budget-light.svg".to_string(),
                            kind: GitHubTreeEntryKind::Blob,
                        },
                        GitHubTreeEntry {
                            path: "svg/nginx-dark.svg".to_string(),
                            kind: GitHubTreeEntryKind::Blob,
                        },
                        GitHubTreeEntry {
                            path: "svg/plain-only.svg".to_string(),
                            kind: GitHubTreeEntryKind::Blob,
                        },
                    ],
                })
            }
        }

        let cache = DashboardIconCache::new(Arc::new(VariantsProvider));
        let count = cache.refresh().await.expect("refresh should succeed");

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
    }

    struct FakeProvider;

    #[async_trait::async_trait]
    impl GitHubProviderClient for FakeProvider {
        async fn fetch_repository_tree(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _git_ref: &str,
            _recursive: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            Ok(GitHubRepositoryTree {
                truncated: false,
                entries: vec![
                    GitHubTreeEntry {
                        path: "svg/nginx.svg".to_string(),
                        kind: GitHubTreeEntryKind::Blob,
                    },
                    GitHubTreeEntry {
                        path: "svg/actual-budget-light.svg".to_string(),
                        kind: GitHubTreeEntryKind::Blob,
                    },
                ],
            })
        }
    }

    #[tokio::test]
    async fn refresh_uses_injected_provider_when_available() {
        let cache = DashboardIconCache::new(Arc::new(FakeProvider));
        let count = cache.refresh().await.expect("refresh succeeds");
        assert_eq!(count, 2);
        assert!(cache.lookup("Nginx").is_some());
    }

    #[tokio::test]
    async fn cold_miss_only_attempts_one_refresh_after_failure() {
        struct FailingProvider {
            calls: AtomicUsize,
        }

        #[async_trait::async_trait]
        impl GitHubProviderClient for FailingProvider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                self.calls.fetch_add(1, Ordering::Relaxed);
                Err(GitHubProviderError::UpstreamUnavailable(
                    "transient".to_string(),
                ))
            }
        }

        let provider = Arc::new(FailingProvider {
            calls: AtomicUsize::new(0),
        });
        let cache = DashboardIconCache::new(provider.clone());

        assert!(cache.lookup_or_try_refresh("Nginx").await.is_none());
        assert!(cache.lookup_or_try_refresh("Nginx").await.is_none());
        assert_eq!(provider.calls.load(Ordering::Relaxed), 1);
    }
}
