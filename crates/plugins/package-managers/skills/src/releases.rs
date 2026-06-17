//! `ReleaseFetcher` implementation for the Agent Skills plugin.
//!
//! Skills are versioned by the subtree SHA of the skill directory in the source
//! repository as of the newest commit touching that path
//! ([`list_recent_commit_dates_for_path`]). Each [`UpstreamRelease`] carries the
//! corresponding commit date in `display_version` (strict ISO 8601 UTC second
//! precision). `batch_fetch` groups items by `(owner, repo, skill_dir)` and
//! makes one provider call per unique group.
//!
//! [`list_recent_commit_dates_for_path`]: uptrakit_global_github_provider::GitHubProviderClient::list_recent_commit_dates_for_path

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_global_github_provider::{GitHubProviderError, PACKAGE_MANAGER_SKILLS, TreeCommit};
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, ReleaseFetcher, Result, UpstreamRelease, Version,
};

use crate::error::SkillsError;
use crate::lock::parse_skill_identifier;
use crate::plugin::SkillsPlugin;

/// Strict ISO 8601 UTC second-precision format for `display_version`.
///
/// The frontend matches this exact shape with the regex
/// `^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$`; any deviation (e.g. RFC 3339
/// subsecond precision) breaks the formatter.
pub(crate) const DISPLAY_FMT: &[time::format_description::FormatItem<'_>] =
    time::macros::format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]Z");

/// Hard cap on the number of recent commits requested from the provider per skill
/// directory. Matches the provider's own `min(limit, 90)` ceiling.
pub(crate) const COMMIT_WINDOW: usize = 90;

/// Turn a [`TreeCommit`] into an [`UpstreamRelease`] for the given skill directory.
fn commit_to_release(
    commit: &TreeCommit,
    owner: &str,
    repo: &str,
    skill_dir: &str,
) -> UpstreamRelease {
    let sha = commit.tree_sha_at_path.clone();
    let url = format!("https://github.com/{owner}/{repo}/tree/HEAD/{skill_dir}");
    let mut release = UpstreamRelease::new(Version::new(sha.clone()), sha, false, url);
    release.display_version = commit.committed_at.format(&DISPLAY_FMT).ok();
    release
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Derive the skill directory from a `skill_path` like `"skills/foo/SKILL.md"`.
///
/// Returns everything up to (but not including) the last `/`. When the path has
/// no `/`, returns an empty string (the root of the repository).
pub(crate) fn derive_skill_dir(skill_path: &str) -> &str {
    skill_path
        .rfind('/')
        .map(|i| skill_path.split_at(i).0)
        .unwrap_or("")
}

/// Map a [`GitHubProviderError`] to a [`PluginError`] wrapped in a [`Report`].
fn map_provider_error(e: GitHubProviderError) -> Report<PluginError> {
    match e {
        GitHubProviderError::Throttled => {
            report!(PluginError::PluginInternal(
                "GitHub rate limit exceeded".to_string()
            ))
        }
        GitHubProviderError::AuthFailed(_) | GitHubProviderError::Misconfigured(_) => {
            report!(PluginError::Configuration(format!("GitHub provider: {e}")))
        }
        _ => report!(PluginError::PluginInternal(format!(
            "GitHub provider error: {e}"
        ))),
    }
}

/// Extract `(owner, repo)` from a `github.com` URL.
///
/// Accepts `https://github.com/owner/repo` and strips trailing `.git` or extra
/// path segments — only the first two path components are used.
pub(crate) fn parse_github_owner_repo(url: &url::Url) -> Result<(String, String)> {
    let host = url.host_str().unwrap_or("");
    if !host.eq_ignore_ascii_case("github.com") {
        return Err(report!(SkillsError::UnsupportedSource(format!(
            "expected github.com host, got '{host}'"
        ))))
        .context_to();
    }

    let path = url.path().trim_start_matches('/').trim_end_matches('/');
    let path = path.trim_end_matches(".git");

    let mut parts = path.splitn(3, '/');
    let owner = parts.next().unwrap_or("").to_string();
    let repo = parts.next().unwrap_or("").to_string();

    if owner.is_empty() || repo.is_empty() {
        return Err(report!(SkillsError::UnsupportedSource(format!(
            "could not extract owner/repo from URL '{url}'"
        ))))
        .context_to();
    }

    Ok((owner, repo))
}

// ── `ReleaseFetcher` impl ─────────────────────────────────────────────────────

#[async_trait]
impl ReleaseFetcher for SkillsPlugin {
    /// Fetch the single upstream release for a skill identifier.
    ///
    /// A skill identifier has the form `<source_url>#<skill_path>`. The version
    /// is the SHA of the skill's subdirectory as of the newest commit touching
    /// it (within the recent commit window); `display_version` is the commit
    /// date formatted as strict ISO 8601 UTC second precision.
    async fn fetch_releases(&self, package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| {
                report!(SkillsError::ProviderUnavailable(
                    "skills release fetching requires the global GitHub provider".to_string()
                ))
            })
            .context_to::<PluginError>()?;

        let (source_url, skill_path) = parse_skill_identifier(package_identifier).context_to()?;

        let (owner, repo) = parse_github_owner_repo(&source_url)?;

        let skill_dir = derive_skill_dir(&skill_path);

        if skill_dir.is_empty() {
            tracing::warn!(
                skill_path = %skill_path,
                "skill path has no parent directory; no releases available"
            );
            return Ok(vec![]);
        }

        let entries = provider
            .list_recent_commit_dates_for_path(
                PACKAGE_MANAGER_SKILLS,
                &owner,
                &repo,
                skill_dir,
                COMMIT_WINDOW,
                &HashSet::new(),
            )
            .await
            .map_err(map_provider_error)?;

        // Provider contract: entries are oldest-first; the newest entry is `.last()`.
        let Some(top) = entries.last() else {
            return Ok(vec![]);
        };

        Ok(vec![commit_to_release(top, &owner, &repo, skill_dir)])
    }

    /// Fetch releases for multiple skills in a single API call per skill directory.
    ///
    /// Groups items by `(owner, repo, skill_dir)`, performs one
    /// `list_recent_commit_dates_for_path` call per group, then resolves the
    /// per-skill newest entry into a [`BatchFetchResult`].
    async fn batch_fetch(&self, items: &[BatchFetchItem]) -> Result<Vec<BatchFetchResult>> {
        // ── 1. Resolve identifiers ────────────────────────────────────────────
        struct Resolved {
            package_identifier: String,
            owner: String,
            repo: String,
            skill_dir: String,
        }

        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| {
                report!(SkillsError::ProviderUnavailable(
                    "skills release fetching requires the global GitHub provider".to_string()
                ))
            })
            .context_to::<PluginError>()?;

        let mut resolved: Vec<std::result::Result<Resolved, (String, String)>> =
            Vec::with_capacity(items.len());

        for item in items {
            let id = &item.package_identifier;
            let r = (|| -> Result<Resolved> {
                let (source_url, skill_path) = parse_skill_identifier(id).context_to()?;
                let (owner, repo) = parse_github_owner_repo(&source_url)?;
                let skill_dir = derive_skill_dir(&skill_path).to_string();
                Ok(Resolved {
                    package_identifier: id.clone(),
                    owner,
                    repo,
                    skill_dir,
                })
            })();
            match r {
                Ok(r) => resolved.push(Ok(r)),
                Err(e) => resolved.push(Err((id.clone(), e.to_string()))),
            }
        }

        // ── 2. Group by (owner, repo, skill_dir); skip empty skill_dir ────────
        // Map (owner, repo, skill_dir) → Result<top TreeCommit, error string>.
        // `None` for the inner option means the group was queried successfully
        // but the path has no commits in the recent window.
        let mut commit_cache: HashMap<
            (String, String, String),
            std::result::Result<Option<TreeCommit>, String>,
        > = HashMap::new();

        for item_result in &resolved {
            let Ok(r) = item_result else { continue };
            if r.skill_dir.is_empty() {
                continue;
            }
            let key = (r.owner.clone(), r.repo.clone(), r.skill_dir.clone());
            if commit_cache.contains_key(&key) {
                continue;
            }
            let resp = provider
                .list_recent_commit_dates_for_path(
                    PACKAGE_MANAGER_SKILLS,
                    &r.owner,
                    &r.repo,
                    &r.skill_dir,
                    COMMIT_WINDOW,
                    &HashSet::new(),
                )
                .await;
            match resp {
                Ok(mut entries) => {
                    // Oldest-first; pop the newest off the tail.
                    commit_cache.insert(key, Ok(entries.pop()));
                }
                Err(e) => {
                    commit_cache.insert(key, Err(e.to_string()));
                }
            }
        }

        // ── 3. Build per-item results ─────────────────────────────────────────
        let mut results = Vec::with_capacity(items.len());
        for item_result in resolved {
            match item_result {
                Err((id, err)) => {
                    results.push(BatchFetchResult::error(id, err));
                }
                Ok(r) => {
                    if r.skill_dir.is_empty() {
                        results.push(BatchFetchResult::empty(r.package_identifier));
                        continue;
                    }
                    let key = (r.owner.clone(), r.repo.clone(), r.skill_dir.clone());
                    match commit_cache.get(&key) {
                        None => {
                            // Should not happen — every successful resolve seeds the cache.
                            results.push(BatchFetchResult::error(
                                r.package_identifier,
                                "commit window not fetched (internal error)".to_string(),
                            ));
                        }
                        Some(Err(msg)) => {
                            results
                                .push(BatchFetchResult::error(r.package_identifier, msg.clone()));
                        }
                        Some(Ok(None)) => {
                            results.push(BatchFetchResult::empty(r.package_identifier));
                        }
                        Some(Ok(Some(commit))) => {
                            let release =
                                commit_to_release(commit, &r.owner, &r.repo, &r.skill_dir);
                            results
                                .push(BatchFetchResult::found(r.package_identifier, vec![release]));
                        }
                    }
                }
            }
        }
        Ok(results)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::collections::HashSet;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use uptrakit_global_github_provider::{
        GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GlobalProviderConsumerId,
        TreeCommit,
    };
    use uptrakit_plugin_infrastructure_core::PluginError;
    use uptrakit_plugin_infrastructure_core::ReleaseFetcher as _;
    use uptrakit_plugin_infrastructure_core::batch_fetch::BatchFetchItem;
    use uptrakit_plugin_infrastructure_core::testing::test_runtime;

    use crate::config::SkillsConfig;
    use crate::plugin::SkillsPlugin;

    // ── helpers ───────────────────────────────────────────────────────────────

    fn make_plugin_with_provider(provider: Arc<dyn GitHubProviderClient>) -> SkillsPlugin {
        let mut plugin =
            SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create plugin");
        plugin.provider = Some(provider);
        plugin
    }

    fn skill_id(source_url: &str, skill_path: &str) -> String {
        format!("{source_url}#{skill_path}")
    }

    fn fixed_date() -> time::OffsetDateTime {
        time::macros::datetime!(2026-06-11 01:15:00 UTC)
    }

    // ── FakeProvider ─────────────────────────────────────────────────────────

    /// Returns a fixed `path → SHA` mapping. Each lookup yields a single
    /// (oldest = newest) `TreeCommit`. Paths absent from the map return an
    /// empty vec (path not in the recent commit window).
    struct FakeProvider {
        commits_by_path: HashMap<String, String>,
    }

    #[async_trait]
    impl GitHubProviderClient for FakeProvider {
        async fn fetch_repository_tree(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _git_ref: &str,
            _recursive: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            Err(GitHubProviderError::Misconfigured(
                "test double: Skills release fetching no longer calls fetch_repository_tree; \
                 update the test to stub list_recent_commit_dates_for_path instead"
                    .to_string(),
            ))
        }

        async fn list_recent_commit_dates_for_path(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            path: &str,
            _limit: usize,
            _expected: &HashSet<String>,
        ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
            match self.commits_by_path.get(path) {
                Some(sha) => Ok(vec![TreeCommit::new(sha.clone(), fixed_date())]),
                None => Ok(vec![]),
            }
        }
    }

    // ── CountingProvider ─────────────────────────────────────────────────────

    /// Counts calls to `list_recent_commit_dates_for_path` and returns a
    /// fixed `path → SHA` mapping.
    struct CountingProvider {
        commits_by_path: HashMap<String, String>,
        calls: AtomicUsize,
    }

    impl CountingProvider {
        fn new(commits_by_path: HashMap<String, String>) -> Arc<Self> {
            Arc::new(Self {
                commits_by_path,
                calls: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait]
    impl GitHubProviderClient for CountingProvider {
        async fn fetch_repository_tree(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _git_ref: &str,
            _recursive: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            Err(GitHubProviderError::Misconfigured(
                "test double: Skills release fetching no longer calls fetch_repository_tree; \
                 update the test to stub list_recent_commit_dates_for_path instead"
                    .to_string(),
            ))
        }

        async fn list_recent_commit_dates_for_path(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            path: &str,
            _limit: usize,
            _expected: &HashSet<String>,
        ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.commits_by_path.get(path) {
                Some(sha) => Ok(vec![TreeCommit::new(sha.clone(), fixed_date())]),
                None => Ok(vec![]),
            }
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_releases_skill_folder_found_returns_one_release() {
        let sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let provider = Arc::new(FakeProvider {
            commits_by_path: HashMap::from([("skills/brainstorming".to_string(), sha.to_string())]),
        });
        let plugin = make_plugin_with_provider(provider);

        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let releases = plugin.fetch_releases(&id).await.expect("fetch ok");

        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, sha);
        assert_eq!(releases[0].version.as_str(), sha);
        assert!(!releases[0].is_prerelease);
        assert!(releases[0].release_url.contains("obra/superpowers"));
    }

    #[tokio::test]
    async fn fetch_releases_skill_folder_missing_returns_empty() {
        // No mapping for `skills/brainstorming` → provider returns no commits.
        // Miss collapses to an empty release list; the frontend short-SHA
        // fallback renders without an error path.
        let provider = Arc::new(FakeProvider {
            commits_by_path: HashMap::new(),
        });
        let plugin = make_plugin_with_provider(provider);

        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let releases = plugin
            .fetch_releases(&id)
            .await
            .expect("missing skill dir is not an error");
        assert!(releases.is_empty(), "expected no releases for missing path");
    }

    #[tokio::test]
    async fn fetch_releases_no_provider_returns_error() {
        let plugin =
            SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create plugin");

        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let result = plugin.fetch_releases(&id).await;
        assert!(
            result.is_err(),
            "expected error when no provider is configured"
        );
    }

    #[tokio::test]
    async fn fetch_releases_non_github_id_returns_error() {
        let provider = Arc::new(FakeProvider {
            commits_by_path: HashMap::new(),
        });
        let plugin = make_plugin_with_provider(provider);

        // gitlab.com URL — not a GitHub source
        let id = skill_id(
            "https://gitlab.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let result = plugin.fetch_releases(&id).await;
        assert!(result.is_err(), "expected error for non-github URL");
    }

    #[tokio::test]
    async fn fetch_releases_throttled_maps_to_plugin_internal() {
        struct ThrottledProvider;

        #[async_trait]
        impl GitHubProviderClient for ThrottledProvider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured(
                    "test double: Skills release fetching no longer uses fetch_repository_tree"
                        .to_string(),
                ))
            }
            async fn list_recent_commit_dates_for_path(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _path: &str,
                _limit: usize,
                _expected: &HashSet<String>,
            ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
                Err(GitHubProviderError::Throttled)
            }
        }

        let plugin = make_plugin_with_provider(Arc::new(ThrottledProvider));
        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let result = plugin.fetch_releases(&id).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("rate limit"),
            "expected rate limit message, got: {msg}"
        );
    }

    #[tokio::test]
    async fn fetch_releases_auth_failed_maps_to_configuration() {
        struct AuthFailedProvider;

        #[async_trait]
        impl GitHubProviderClient for AuthFailedProvider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured(
                    "test double: Skills release fetching no longer uses fetch_repository_tree"
                        .to_string(),
                ))
            }
            async fn list_recent_commit_dates_for_path(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _path: &str,
                _limit: usize,
                _expected: &HashSet<String>,
            ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
                Err(GitHubProviderError::AuthFailed("bad token".to_string()))
            }
        }

        let plugin = make_plugin_with_provider(Arc::new(AuthFailedProvider));
        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let result = plugin.fetch_releases(&id).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), PluginError::Configuration(_)),
            "AuthFailed must map to Configuration, got: {:?}",
            err.current_context()
        );
    }

    #[tokio::test]
    async fn fetch_releases_misconfigured_maps_to_configuration() {
        struct MisconfiguredProvider;

        #[async_trait]
        impl GitHubProviderClient for MisconfiguredProvider {
            async fn fetch_repository_tree(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _git_ref: &str,
                _recursive: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured(
                    "test double: Skills release fetching no longer uses fetch_repository_tree"
                        .to_string(),
                ))
            }
            async fn list_recent_commit_dates_for_path(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _path: &str,
                _limit: usize,
                _expected: &HashSet<String>,
            ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured("bad config".to_string()))
            }
        }

        let plugin = make_plugin_with_provider(Arc::new(MisconfiguredProvider));
        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let result = plugin.fetch_releases(&id).await;
        let err = result.unwrap_err();
        assert!(
            matches!(err.current_context(), PluginError::Configuration(_)),
            "Misconfigured must map to Configuration, got: {:?}",
            err.current_context()
        );
    }

    #[tokio::test]
    async fn fetch_releases_sets_display_version_to_iso_8601_commit_date() {
        let sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";

        struct DateProvider {
            sha: &'static str,
        }
        #[async_trait]
        impl GitHubProviderClient for DateProvider {
            async fn fetch_repository_tree(
                &self,
                _: GlobalProviderConsumerId,
                _: &str,
                _: &str,
                _: &str,
                _: bool,
            ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
                Err(GitHubProviderError::Misconfigured(
                    "test double: fetch_releases must not call fetch_repository_tree anymore"
                        .to_string(),
                ))
            }
            async fn list_recent_commit_dates_for_path(
                &self,
                _consumer: GlobalProviderConsumerId,
                _owner: &str,
                _repo: &str,
                _path: &str,
                _limit: usize,
                _expected: &HashSet<String>,
            ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
                Ok(vec![TreeCommit::new(
                    self.sha.to_string(),
                    time::macros::datetime!(2026-06-11 01:15:00 UTC),
                )])
            }
        }

        let plugin = make_plugin_with_provider(Arc::new(DateProvider { sha }));
        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let releases = plugin.fetch_releases(&id).await.expect("fetch ok");
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].tag, sha, "tag still carries the SHA");
        assert_eq!(
            releases[0].display_version.as_deref(),
            Some("2026-06-11T01:15:00Z"),
            "display_version is strict ISO 8601 UTC second-precision"
        );
    }

    #[tokio::test]
    async fn batch_fetch_groups_by_skill_dir() {
        // With the per-skill-directory commit-walk primitive, batch_fetch
        // makes exactly one provider call per unique (owner, repo, skill_dir).
        // Two skills sharing a directory collapse to a single call; two
        // skills in distinct directories of the same repo result in two.
        let sha1 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        let sha2 = "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";

        let counting = CountingProvider::new(HashMap::from([
            ("skills/brainstorming".to_string(), sha1.to_string()),
            ("skills/spec".to_string(), sha2.to_string()),
        ]));
        let plugin = {
            let mut p =
                SkillsPlugin::new(SkillsConfig::default(), test_runtime()).expect("create plugin");
            p.provider = Some(Arc::clone(&counting) as Arc<dyn GitHubProviderClient>);
            p
        };

        let items = vec![
            BatchFetchItem::new(skill_id(
                "https://github.com/obra/superpowers",
                "skills/brainstorming/SKILL.md",
            )),
            BatchFetchItem::new(skill_id(
                "https://github.com/obra/superpowers",
                "skills/spec/SKILL.md",
            )),
            // Same skill_dir as the first item → must reuse the cached call.
            BatchFetchItem::new(skill_id(
                "https://github.com/obra/superpowers",
                "skills/brainstorming/README.md",
            )),
        ];

        let results = plugin.batch_fetch(&items).await.expect("batch ok");

        assert_eq!(
            counting.calls.load(Ordering::SeqCst),
            2,
            "expected one provider call per unique (owner, repo, skill_dir)"
        );
        assert_eq!(results.len(), 3);

        let r0 = &results[0];
        assert!(r0.error.is_none(), "skill 0 error: {:?}", r0.error);
        assert_eq!(r0.releases.len(), 1);
        assert_eq!(r0.releases[0].tag, sha1);
        assert_eq!(
            r0.releases[0].display_version.as_deref(),
            Some("2026-06-11T01:15:00Z")
        );

        let r1 = &results[1];
        assert!(r1.error.is_none(), "skill 1 error: {:?}", r1.error);
        assert_eq!(r1.releases.len(), 1);
        assert_eq!(r1.releases[0].tag, sha2);

        let r2 = &results[2];
        assert!(r2.error.is_none(), "skill 2 error: {:?}", r2.error);
        assert_eq!(
            r2.releases.len(),
            1,
            "third item shares brainstorming dir; same SHA"
        );
        assert_eq!(r2.releases[0].tag, sha1);
    }
}
