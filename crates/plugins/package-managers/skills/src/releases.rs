//! `ReleaseFetcher` implementation for the Agent Skills plugin.
//!
//! Skills are versioned by the SHA of the subdirectory in the source repository
//! (obtained via the GitHub Trees API). A single tree call covers all skills
//! that share the same `owner/repo`, so `batch_fetch` groups items before
//! making API calls.

use std::collections::HashMap;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_global_github_provider::{
    GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntryKind, PACKAGE_MANAGER_SKILLS,
};
use uptrakit_plugin_infrastructure_core::{
    BatchFetchItem, BatchFetchResult, PluginError, ReleaseFetcher, Result, UpstreamRelease, Version,
};

use crate::error::SkillsError;
use crate::lock::parse_skill_identifier;
use crate::plugin::SkillsPlugin;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Derive the skill directory from a `skill_path` like `"skills/foo/SKILL.md"`.
///
/// Returns everything up to (but not including) the last `/`. When the path has
/// no `/`, returns an empty string (the root of the repository).
fn derive_skill_dir(skill_path: &str) -> &str {
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
fn parse_github_owner_repo(url: &url::Url) -> Result<(String, String)> {
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

/// Convert a repository tree into an [`UpstreamRelease`] for the given `skill_dir`.
///
/// Looks for a `Tree`-kind entry whose path equals `skill_dir`. When `skill_dir`
/// is empty the repo root (SHA obtained from the tree itself) is used instead.
/// Returns `None` when the directory is not found in the tree.
fn tree_to_release(
    tree: &GitHubRepositoryTree,
    skill_dir: &str,
    owner: &str,
    repo: &str,
) -> Option<UpstreamRelease> {
    let sha = tree
        .entries
        .iter()
        .find(|e| e.kind == GitHubTreeEntryKind::Tree && e.path == skill_dir)
        .map(|e| e.sha.clone())?;

    let url = format!("https://github.com/{owner}/{repo}/tree/HEAD/{skill_dir}");
    Some(UpstreamRelease::new(
        Version::new(sha.clone()),
        sha,
        false,
        url,
    ))
}

// ── `ReleaseFetcher` impl ─────────────────────────────────────────────────────

#[async_trait]
impl ReleaseFetcher for SkillsPlugin {
    /// Fetch the single upstream release for a skill identifier.
    ///
    /// A skill identifier has the form `<source_url>#<skill_path>`. The version
    /// is the SHA of the skill's subdirectory from the GitHub Trees API.
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

        let tree = provider
            .fetch_repository_tree(PACKAGE_MANAGER_SKILLS, &owner, &repo, "HEAD", true)
            .await
            .map_err(map_provider_error)?;

        if tree.truncated {
            tracing::warn!(
                owner = %owner,
                repo = %repo,
                "repository tree is truncated; skill directory SHA may be missing"
            );
            return Err(report!(PluginError::PluginInternal(format!(
                "repository tree for {owner}/{repo} is truncated; cannot determine skill SHA"
            ))));
        }

        let skill_dir = derive_skill_dir(&skill_path);

        if skill_dir.is_empty() {
            tracing::warn!(
                skill_path = %skill_path,
                "skill path has no parent directory; no releases available"
            );
            return Ok(vec![]);
        }

        match tree_to_release(&tree, skill_dir, &owner, &repo) {
            Some(release) => Ok(vec![release]),
            None => Err(report!(PluginError::PluginInternal(format!(
                "skill directory '{skill_dir}' not found in {owner}/{repo} tree"
            )))),
        }
    }

    /// Fetch releases for multiple skills in a single API call per repository.
    ///
    /// Groups items by `(owner, repo)`, performs one `fetch_repository_tree`
    /// call per group, then resolves per-skill results from the shared tree.
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

        // ── 2. Group by (owner, repo) and fetch trees ─────────────────────────
        // Map "(owner, repo)" → GitHubRepositoryTree (or an error string).
        let mut tree_cache: HashMap<
            (String, String),
            std::result::Result<GitHubRepositoryTree, String>,
        > = HashMap::new();

        for item_result in &resolved {
            let Ok(r) = item_result else { continue };
            let key = (r.owner.clone(), r.repo.clone());
            if tree_cache.contains_key(&key) {
                continue;
            }
            let tree_result = provider
                .fetch_repository_tree(PACKAGE_MANAGER_SKILLS, &r.owner, &r.repo, "HEAD", true)
                .await;
            match tree_result {
                Ok(tree) if tree.truncated => {
                    tracing::warn!(
                        owner = %r.owner,
                        repo = %r.repo,
                        "repository tree is truncated; results may be incomplete"
                    );
                    tree_cache.insert(
                        key,
                        Err(format!(
                            "repository tree for {}/{} is truncated",
                            r.owner, r.repo
                        )),
                    );
                }
                Ok(tree) => {
                    tree_cache.insert(key, Ok(tree));
                }
                Err(e) => {
                    tree_cache.insert(key, Err(e.to_string()));
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
                    let key = (r.owner.clone(), r.repo.clone());
                    match tree_cache.get(&key) {
                        None => {
                            // No entry means we never tried (shouldn't happen).
                            results.push(BatchFetchResult::error(
                                r.package_identifier,
                                "tree not fetched (internal error)".to_string(),
                            ));
                        }
                        Some(Err(msg)) => {
                            results
                                .push(BatchFetchResult::error(r.package_identifier, msg.clone()));
                        }
                        Some(Ok(tree)) => {
                            if r.skill_dir.is_empty() {
                                results.push(BatchFetchResult::empty(r.package_identifier));
                            } else {
                                match tree_to_release(tree, &r.skill_dir, &r.owner, &r.repo) {
                                    Some(release) => {
                                        results.push(BatchFetchResult::found(
                                            r.package_identifier,
                                            vec![release],
                                        ));
                                    }
                                    None => {
                                        results.push(BatchFetchResult::error(
                                            r.package_identifier,
                                            format!(
                                                "skill directory '{}' not found in {}/{} tree",
                                                r.skill_dir, r.owner, r.repo
                                            ),
                                        ));
                                    }
                                }
                            }
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use uptrakit_global_github_provider::{
        GitHubProviderClient, GitHubProviderError, GitHubRepositoryTree, GitHubTreeEntry,
        GitHubTreeEntryKind, GlobalProviderConsumerId,
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

    fn make_tree(entries: Vec<GitHubTreeEntry>) -> GitHubRepositoryTree {
        GitHubRepositoryTree {
            truncated: false,
            entries,
        }
    }

    fn tree_entry(path: &str, kind: GitHubTreeEntryKind, sha: &str) -> GitHubTreeEntry {
        GitHubTreeEntry {
            path: path.to_string(),
            kind,
            sha: sha.to_string(),
        }
    }

    // ── FakeProvider ─────────────────────────────────────────────────────────

    /// Returns a fixed tree regardless of owner/repo.
    struct FakeProvider {
        tree: GitHubRepositoryTree,
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
            Ok(self.tree.clone())
        }
    }

    // ── CountingProvider ─────────────────────────────────────────────────────

    /// Counts calls and returns a fixed tree.
    struct CountingProvider {
        tree: GitHubRepositoryTree,
        calls: AtomicUsize,
    }

    impl CountingProvider {
        fn new(tree: GitHubRepositoryTree) -> Arc<Self> {
            Arc::new(Self {
                tree,
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
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.tree.clone())
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fetch_releases_skill_folder_found_returns_one_release() {
        let sha = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let tree = make_tree(vec![
            tree_entry("skills", GitHubTreeEntryKind::Tree, "aaaa"),
            tree_entry("skills/brainstorming", GitHubTreeEntryKind::Tree, sha),
            tree_entry(
                "skills/brainstorming/SKILL.md",
                GitHubTreeEntryKind::Blob,
                "bbbb",
            ),
        ]);
        let provider = Arc::new(FakeProvider { tree });
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
    async fn fetch_releases_skill_folder_missing_returns_error() {
        let tree = make_tree(vec![tree_entry(
            "skills/other",
            GitHubTreeEntryKind::Tree,
            "cccc",
        )]);
        let provider = Arc::new(FakeProvider { tree });
        let plugin = make_plugin_with_provider(provider);

        let id = skill_id(
            "https://github.com/obra/superpowers",
            "skills/brainstorming/SKILL.md",
        );
        let result = plugin.fetch_releases(&id).await;
        assert!(result.is_err(), "expected error when skill dir is missing");
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
        let tree = make_tree(vec![]);
        let provider = Arc::new(FakeProvider { tree });
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
    async fn batch_fetch_one_tree_call_per_repo() {
        let sha1 = "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
        let sha2 = "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";
        let tree = make_tree(vec![
            tree_entry("skills/brainstorming", GitHubTreeEntryKind::Tree, sha1),
            tree_entry("skills/spec", GitHubTreeEntryKind::Tree, sha2),
        ]);

        let counting = CountingProvider::new(tree);
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
        ];

        let results = plugin.batch_fetch(&items).await.expect("batch ok");

        // Exactly one tree call for both items (same owner/repo).
        assert_eq!(
            counting.calls.load(Ordering::SeqCst),
            1,
            "expected exactly one tree call for two skills in the same repo"
        );
        assert_eq!(results.len(), 2);

        let r0 = &results[0];
        assert!(r0.error.is_none(), "skill 0 error: {:?}", r0.error);
        assert_eq!(r0.releases.len(), 1);
        assert_eq!(r0.releases[0].tag, sha1);

        let r1 = &results[1];
        assert!(r1.error.is_none(), "skill 1 error: {:?}", r1.error);
        assert_eq!(r1.releases.len(), 1);
        assert_eq!(r1.releases[0].tag, sha2);
    }
}
