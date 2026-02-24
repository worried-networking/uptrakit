use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::ProviderType;
use uptrakit_provider_registry::{ProviderCapability, ProviderRegistry};

use crate::connection_context::ConnectionContext;

/// Result of a version check for a single software item.
pub struct VersionCheckOutcome {
    /// Detected installed version, if any.
    pub installed_version: Option<String>,
    /// Latest available version from the local package index, if the provider
    /// supports agent-side release fetching.
    pub latest_version: Option<String>,
    /// Error message if detection failed.
    pub error: Option<String>,
}

/// Check the installed version (and optionally the latest version) for a
/// software item.
///
/// If the provider supports `RefreshPackageIndex`, the latest available version
/// is also fetched via `fetch_releases()`. For providers that resolve latest
/// versions on the controller side, `latest_version` will be `None`.
///
/// The `ctx` parameter is used to inject connection-specific overrides (e.g.
/// a remote Docker host for the SSH agent) into the provider config before
/// instantiation.
pub async fn check_version(
    provider_type: ProviderType,
    config: &serde_json::Value,
    package_identifier: &str,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> VersionCheckOutcome {
    tracing::debug!(provider_type = ?provider_type, package_identifier, "checking version");

    let mut effective_config = config.clone();
    ctx.apply_to_config(&provider_type, &mut effective_config);

    let provider = match ProviderRegistry::create_provider(provider_type, &effective_config, executor) {
        Ok(p) => p,
        Err(e) => {
            return VersionCheckOutcome {
                installed_version: None,
                latest_version: None,
                error: Some(e.to_string()),
            };
        }
    };

    tracing::debug!("detecting installed version");
    let installed_version = match provider.detect_installed_version(package_identifier).await {
        Ok(Some(version)) => {
            tracing::debug!(version = %version, "installed version detected");
            Some(version.to_string())
        }
        Ok(None) => {
            tracing::debug!("no installed version detected");
            None
        }
        Err(e) => {
            return VersionCheckOutcome {
                installed_version: None,
                latest_version: None,
                error: Some(format!("detection failed: {e}")),
            };
        }
    };

    // For providers that can resolve latest versions locally (e.g., Homebrew),
    // also fetch the latest available version from the package index.
    let latest_version = if provider.has_capability(ProviderCapability::RefreshPackageIndex) {
        tracing::debug!("fetching releases from provider");
        match provider.fetch_releases(package_identifier).await {
            Ok(releases) => {
                tracing::debug!(count = releases.len(), "releases fetched");
                releases.first().map(|r| r.version.to_string())
            }
            Err(e) => {
                tracing::debug!(error = %e, "failed to fetch latest version from provider");
                None
            }
        }
    } else {
        None
    };

    VersionCheckOutcome {
        installed_version,
        latest_version,
        error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn no_ctx() -> ConnectionContext {
        ConnectionContext::default()
    }

    #[tokio::test]
    async fn check_version_github_stub_returns_none() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let outcome = check_version(
            ProviderType::GithubReleases,
            &config,
            "octocat/hello-world",
            test_executor(),
            &no_ctx(),
        )
        .await;
        // Stub implementation returns None for installed_version
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn check_version_docker_stub_returns_none() {
        // Empty config — valid for Docker
        let config = serde_json::json!({});
        let outcome = check_version(
            ProviderType::Docker,
            &config,
            "nginx",
            test_executor(),
            &no_ctx(),
        )
        .await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn check_version_proxmox_stub_returns_none() {
        let config = serde_json::json!({
            "script_url": "https://example.com/update.sh"
        });
        let outcome = check_version(
            ProviderType::ProxmoxHelperScripts,
            &config,
            "example",
            test_executor(),
            &no_ctx(),
        )
        .await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn check_version_github_invalid_config() {
        let config = serde_json::json!({
            "invalid": "config"
        });
        let outcome = check_version(
            ProviderType::GithubReleases,
            &config,
            "octocat/hello-world",
            test_executor(),
            &no_ctx(),
        )
        .await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.error.is_some());
        assert!(outcome.error.unwrap().contains("failed to parse"));
    }

    #[tokio::test]
    async fn check_version_homebrew_default_returns_none() {
        let config = serde_json::json!({});
        let outcome = check_version(ProviderType::Homebrew, &config, "", test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_docker_context_injects_docker_host() {
        let config = serde_json::json!({});
        let ctx = ConnectionContext {
            docker_host_override: Some("ssh://user@host:2222".to_string()),
            ssh_key_path: None,
        };
        // With a valid docker host override, the provider is created with the
        // injected host. The check itself will fail (no daemon) but that proves
        // the injection path runs without panicking.
        let outcome = check_version(
            ProviderType::Docker,
            &config,
            "nginx",
            test_executor(),
            &ctx,
        )
        .await;
        // We don't assert success — just that it didn't crash with bad injection
        let _ = outcome;
    }
}
