//! Shared contract for the global GitHub provider used by global plugins.

use std::sync::Arc;

use async_trait::async_trait;

/// Stable identifier for a consumer of the global GitHub provider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GlobalProviderConsumerId(&'static str);

impl GlobalProviderConsumerId {
    /// Create a new consumer identifier.
    pub const fn new(value: &'static str) -> Self {
        Self(value)
    }

    /// Return the identifier as a string slice.
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

/// Global GitHub provider consumer used by the dashboard icons plugin.
pub const DASHBOARD_ICONS: GlobalProviderConsumerId =
    GlobalProviderConsumerId::new("dashboard-icons");

/// Global GitHub provider consumer used by the package-manager-skills plugin.
pub const PACKAGE_MANAGER_SKILLS: GlobalProviderConsumerId =
    GlobalProviderConsumerId::new("package-manager-skills");

/// Shared error type for GitHub provider calls.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum GitHubProviderError {
    Throttled,
    AuthFailed(String),
    UpstreamUnavailable(String),
    RequestFailed(String),
    Misconfigured(String),
}

impl std::fmt::Display for GitHubProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Throttled => f.write_str("request throttled"),
            Self::AuthFailed(message) => write!(f, "authentication failed: {message}"),
            Self::UpstreamUnavailable(message) => write!(f, "upstream unavailable: {message}"),
            Self::RequestFailed(message) => write!(f, "request failed: {message}"),
            Self::Misconfigured(message) => write!(f, "misconfigured: {message}"),
        }
    }
}

impl std::error::Error for GitHubProviderError {}

impl GitHubProviderError {
    /// Return whether this error should be retried by the caller.
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Throttled | Self::UpstreamUnavailable(_))
    }
}

/// Repository tree response owned by Uptrakit, not octocrab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubRepositoryTree {
    pub truncated: bool,
    pub entries: Vec<GitHubTreeEntry>,
}

/// One entry in a GitHub repository tree response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubTreeEntry {
    pub path: String,
    pub kind: GitHubTreeEntryKind,
    pub sha: String,
}

/// Tree entry kind supported by the shared contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubTreeEntryKind {
    Blob,
    Tree,
}

/// A commit returned by [`GitHubProviderClient::list_recent_commit_dates_for_path`].
///
/// `tree_sha_at_path` is the SHA of the **subtree at the queried `path`** as of this
/// commit — not the commit's root tree SHA. `committed_at` is the committer date
/// (`commit.committer.date`), chosen over author date for rebase-stability.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeCommit {
    pub tree_sha_at_path: String,
    pub committed_at: time::OffsetDateTime,
}

impl TreeCommit {
    /// Construct a new tree commit. Use this from external crates since `TreeCommit`
    /// is `#[non_exhaustive]` and struct-literal construction is forbidden.
    pub fn new(tree_sha_at_path: String, committed_at: time::OffsetDateTime) -> Self {
        Self {
            tree_sha_at_path,
            committed_at,
        }
    }
}

/// Opaque handle stored in the plugin catalog lookup table.
pub struct GitHubProviderHandle {
    client: Arc<dyn GitHubProviderClient>,
}

impl GitHubProviderHandle {
    /// Wrap a provider client in a type-erased handle.
    pub fn new(client: Arc<dyn GitHubProviderClient>) -> Self {
        Self { client }
    }

    /// Return a cloned client reference.
    pub fn client(&self) -> Arc<dyn GitHubProviderClient> {
        Arc::clone(&self.client)
    }
}

/// Host-owned GitHub provider interface injected into global plugins.
#[async_trait]
pub trait GitHubProviderClient: Send + Sync {
    async fn fetch_repository_tree(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        git_ref: &str,
        recursive: bool,
    ) -> Result<GitHubRepositoryTree, GitHubProviderError>;

    /// Return up to `min(limit, 90)` recent commits that touched `path`, oldest-first,
    /// each annotated with the subtree SHA at `path` as of that commit.
    ///
    /// `expected_shas` lets the caller short-circuit the walk when every target SHA
    /// has been bound — pass `&HashSet::new()` to force the full walk.
    ///
    /// Default impl returns `Misconfigured("...not implemented")` so existing
    /// implementors compile unchanged. The Skills enricher treats this error like
    /// any other provider failure: log + write `None`.
    async fn list_recent_commit_dates_for_path(
        &self,
        consumer: GlobalProviderConsumerId,
        owner: &str,
        repo: &str,
        path: &str,
        limit: usize,
        expected_shas: &std::collections::HashSet<String>,
    ) -> std::result::Result<Vec<TreeCommit>, GitHubProviderError> {
        let _ = (consumer, owner, repo, path, limit, expected_shas);
        Err(GitHubProviderError::Misconfigured(
            "list_recent_commit_dates_for_path not implemented".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_icons_consumer_id_is_stable() {
        assert_eq!(DASHBOARD_ICONS.as_str(), "dashboard-icons");
    }

    #[test]
    fn package_manager_skills_consumer_id_is_stable() {
        assert_eq!(PACKAGE_MANAGER_SKILLS.as_str(), "package-manager-skills");
    }

    #[test]
    fn github_provider_error_auth_failed_is_not_retryable() {
        assert!(!GitHubProviderError::AuthFailed("bad token".into()).is_retryable());
    }

    #[test]
    fn github_provider_error_request_failed_is_not_retryable() {
        assert!(!GitHubProviderError::RequestFailed("bad request".into()).is_retryable());
    }
}

#[cfg(test)]
mod tree_commit_tests {
    use super::*;
    use std::collections::HashSet;

    struct UnimplementedProvider;
    #[async_trait::async_trait]
    impl GitHubProviderClient for UnimplementedProvider {
        async fn fetch_repository_tree(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _git_ref: &str,
            _recursive: bool,
        ) -> std::result::Result<GitHubRepositoryTree, GitHubProviderError> {
            Err(GitHubProviderError::Misconfigured(
                "test fixture: fetch_repository_tree must not be called".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn list_recent_commit_dates_for_path_default_returns_misconfigured() {
        let p = UnimplementedProvider;
        let expected: HashSet<String> = HashSet::new();
        let err = p
            .list_recent_commit_dates_for_path(
                PACKAGE_MANAGER_SKILLS,
                "owner",
                "repo",
                "skills/x",
                90,
                &expected,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubProviderError::Misconfigured(_)),
            "expected Misconfigured, got: {err:?}"
        );
    }
}
