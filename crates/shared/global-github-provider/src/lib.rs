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

/// Look up the GitHub provider client from the plugin catalog config.
pub fn lookup_github_provider(
    config: &uptrakit_plugin_infrastructure_core::CatalogConfig,
) -> Option<Arc<dyn GitHubProviderClient>> {
    let lookup = config.global_provider_lookup.as_ref()?;
    let handle = lookup.lookup("github")?;
    let handle = Arc::downcast::<GitHubProviderHandle>(handle).ok()?;
    Some(handle.client())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::Any;

    struct TestClient;

    #[async_trait]
    impl GitHubProviderClient for TestClient {
        async fn fetch_repository_tree(
            &self,
            _consumer: GlobalProviderConsumerId,
            _owner: &str,
            _repo: &str,
            _git_ref: &str,
            _recursive: bool,
        ) -> Result<GitHubRepositoryTree, GitHubProviderError> {
            Ok(GitHubRepositoryTree {
                truncated: false,
                entries: vec![],
            })
        }
    }

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

    #[test]
    fn lookup_returns_the_underlying_client() {
        use uptrakit_plugin_infrastructure_core::{CatalogConfig, GlobalProviderLookup};

        struct Lookup {
            handle: Arc<dyn Any + Send + Sync>,
        }

        impl GlobalProviderLookup for Lookup {
            fn lookup(&self, provider_id: &str) -> Option<Arc<dyn Any + Send + Sync>> {
                match provider_id {
                    "github" => Some(Arc::clone(&self.handle)),
                    _ => None,
                }
            }
        }

        let client: Arc<dyn GitHubProviderClient> = Arc::new(TestClient);
        let handle: Arc<dyn Any + Send + Sync> =
            Arc::new(GitHubProviderHandle::new(Arc::clone(&client)));
        let config = CatalogConfig {
            global_provider_lookup: Some(Arc::new(Lookup { handle })),
            ..CatalogConfig::default()
        };

        let looked_up = lookup_github_provider(&config).expect("provider should be found");
        assert!(Arc::ptr_eq(&looked_up, &client));
    }

    #[test]
    fn lookup_returns_none_for_wrong_provider_id() {
        use uptrakit_plugin_infrastructure_core::{CatalogConfig, GlobalProviderLookup};

        struct Lookup;

        impl GlobalProviderLookup for Lookup {
            fn lookup(&self, _provider_id: &str) -> Option<Arc<dyn Any + Send + Sync>> {
                None
            }
        }

        let config = CatalogConfig {
            global_provider_lookup: Some(Arc::new(Lookup)),
            ..CatalogConfig::default()
        };

        assert!(lookup_github_provider(&config).is_none());
    }

    #[test]
    fn lookup_returns_none_for_wrong_handle_type() {
        use uptrakit_plugin_infrastructure_core::{CatalogConfig, GlobalProviderLookup};

        struct Lookup;

        impl GlobalProviderLookup for Lookup {
            fn lookup(&self, provider_id: &str) -> Option<Arc<dyn Any + Send + Sync>> {
                (provider_id == "github").then(|| Arc::new("wrong") as Arc<dyn Any + Send + Sync>)
            }
        }

        let config = CatalogConfig {
            global_provider_lookup: Some(Arc::new(Lookup)),
            ..CatalogConfig::default()
        };

        assert!(lookup_github_provider(&config).is_none());
    }
}
