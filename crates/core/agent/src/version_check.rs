use uptrakit_provider_core::LocalProvider;
use uptrakit_provider_docker_registry::DockerRegistryLocalProvider;
use uptrakit_provider_github::{GitHubConfig, GitHubLocalProvider};
use uptrakit_provider_proxmox_helper_scripts::ProxmoxHelperScriptsLocalProvider;

/// Check the installed version for a software item.
///
/// Returns `(installed_version, error)` where exactly one is `Some`.
pub async fn check_version(
    provider_type: &str,
    package_identifier: &str,
    config: &serde_json::Value,
) -> (Option<String>, Option<String>) {
    match provider_type {
        "github_releases" => {
            let github_config: GitHubConfig = match serde_json::from_value(config.clone()) {
                Ok(c) => c,
                Err(e) => {
                    return (None, Some(format!("failed to parse GitHub config: {e}")));
                }
            };
            let provider = GitHubLocalProvider::new(github_config, package_identifier.to_string());
            match provider.detect_installed_version().await {
                Ok(Some(version)) => (Some(version.to_string()), None),
                Ok(None) => (None, None),
                Err(e) => (None, Some(format!("detection failed: {e}"))),
            }
        }
        "docker_registry" => {
            let provider = DockerRegistryLocalProvider::new();
            match provider.detect_installed_version().await {
                Ok(Some(version)) => (Some(version.to_string()), None),
                Ok(None) => (None, None),
                Err(e) => (None, Some(format!("detection failed: {e}"))),
            }
        }
        "proxmox_helper_scripts" => {
            let provider = ProxmoxHelperScriptsLocalProvider::new(package_identifier.to_string());
            match provider.detect_installed_version().await {
                Ok(Some(version)) => (Some(version.to_string()), None),
                Ok(None) => (None, None),
                Err(e) => (None, Some(format!("detection failed: {e}"))),
            }
        }
        _ => (
            None,
            Some(format!("unsupported provider type: {provider_type}")),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn check_version_unsupported_provider() {
        let (version, error) =
            check_version("unknown_provider", "test", &serde_json::json!({})).await;
        assert!(version.is_none());
        assert!(error.is_some());
        assert!(error.unwrap().contains("unsupported provider type"));
    }

    #[tokio::test]
    async fn check_version_github_stub_returns_none() {
        let config = serde_json::json!({
            "owner": "octocat",
            "repo": "hello-world"
        });
        let (version, error) =
            check_version("github_releases", "octocat/hello-world", &config).await;
        // Stub implementation returns None for installed_version
        assert!(version.is_none());
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn check_version_docker_stub_returns_none() {
        let (version, error) =
            check_version("docker_registry", "nginx:latest", &serde_json::json!({})).await;
        assert!(version.is_none());
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn check_version_proxmox_stub_returns_none() {
        let (version, error) = check_version(
            "proxmox_helper_scripts",
            "test-script",
            &serde_json::json!({}),
        )
        .await;
        assert!(version.is_none());
        assert!(error.is_none());
    }

    #[tokio::test]
    async fn check_version_github_invalid_config() {
        let config = serde_json::json!({
            "invalid": "config"
        });
        let (version, error) = check_version("github_releases", "test", &config).await;
        assert!(version.is_none());
        assert!(error.is_some());
        assert!(error.unwrap().contains("failed to parse"));
    }
}
