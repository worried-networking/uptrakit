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
}

/// Tree entry kind supported by the shared contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitHubTreeEntryKind {
    Blob,
    Tree,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_icons_consumer_id_is_stable() {
        assert_eq!(DASHBOARD_ICONS.as_str(), "dashboard-icons");
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
