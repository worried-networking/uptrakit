use uptrakit_internal_wire::ProviderType;
use uptrakit_provider_registry::ProviderRegistry;

/// Check the installed version for a software item.
///
/// Returns `(installed_version, error)` where exactly one is `Some`.
pub async fn check_version(
    provider_type: ProviderType,
    config: &serde_json::Value,
) -> (Option<String>, Option<String>) {
    match ProviderRegistry::create_provider(provider_type, config) {
        Ok(provider) => match provider.detect_installed_version().await {
            Ok(Some(version)) => (Some(version.to_string()), None),
            Ok(None) => (None, None),
            Err(e) => (None, Some(format!("detection failed: {e}"))),
        },
        Err(e) => (None, Some(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_version_github_stub_returns_none() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let (version, error) = check_version(ProviderType::GithubReleases, &config).await;
        // Stub implementation returns None for installed_version
        assert!(version.is_none());
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn check_version_docker_stub_returns_none() {
        let config = serde_json::json!({
            "image": "nginx"
        });
        let (version, error) = check_version(ProviderType::DockerRegistry, &config).await;
        assert!(version.is_none());
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn check_version_proxmox_stub_returns_none() {
        let (version, error) =
            check_version(ProviderType::ProxmoxHelperScripts, &serde_json::json!({})).await;
        assert!(version.is_none());
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn check_version_github_invalid_config() {
        let config = serde_json::json!({
            "invalid": "config"
        });
        let (version, error) = check_version(ProviderType::GithubReleases, &config).await;
        assert!(version.is_none());
        assert!(error.is_some());
        assert!(error.unwrap().contains("failed to parse"));
    }
}
