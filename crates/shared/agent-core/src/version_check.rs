use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_internal_wire::PluginAssignment;
use uptrakit_plugin_infrastructure_registry::PluginRegistry;

use crate::connection_context::ConnectionContext;

/// Result of a version check for a single software item.
pub struct VersionCheckOutcome {
    /// Detected installed version, if any.
    pub installed_version: Option<String>,
    /// Latest available version from the local package index, if the plugin
    /// supports agent-side release fetching.
    pub latest_version: Option<String>,
    /// Error message if detection failed.
    pub error: Option<String>,
}

/// Check the installed version (and optionally the latest version) for a
/// software item using role-based plugin assignments.
///
/// - `detect`: plugin assignment for detecting the installed version. If `None`,
///   `installed_version` will be `None` in the outcome.
/// - `fetch`: plugin assignment for fetching the latest available version from
///   a local package index. If `None`, `latest_version` will be `None`.
///
/// The `ctx` parameter is used to inject connection-specific overrides (e.g.
/// a remote Docker host for the SSH agent) into the plugin config before
/// instantiation.
pub async fn check_version(
    detect: Option<&PluginAssignment>,
    fetch: Option<&PluginAssignment>,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> VersionCheckOutcome {
    let installed_version = if let Some(assignment) = detect {
        detect_installed(assignment, Arc::clone(&executor), ctx).await
    } else {
        Ok(None)
    };

    let (installed_version, detect_error) = match installed_version {
        Ok(v) => (v, None),
        Err(e) => (None, Some(e)),
    };

    let latest_version = if let Some(assignment) = fetch {
        fetch_latest(assignment, Arc::clone(&executor), ctx).await
    } else {
        Ok(None)
    };

    let (latest_version, fetch_error) = match latest_version {
        Ok(v) => (v, None),
        Err(e) => (None, Some(e)),
    };

    // Combine errors if both roles failed.
    let error = match (detect_error, fetch_error) {
        (Some(d), Some(f)) => Some(format!("detect: {d}; fetch: {f}")),
        (Some(d), None) => Some(d),
        (None, Some(f)) => Some(f),
        (None, None) => None,
    };

    VersionCheckOutcome {
        installed_version,
        latest_version,
        error,
    }
}

/// Detect the installed version using a specific plugin assignment.
async fn detect_installed(
    assignment: &PluginAssignment,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Result<Option<String>, String> {
    tracing::debug!(
        plugin_type = ?assignment.plugin_type,
        package = %assignment.package_identifier,
        "detecting installed version"
    );

    let mut effective_config = assignment.config.clone();
    ctx.apply_to_config(&assignment.plugin_type, &mut effective_config);

    let plugin =
        PluginRegistry::create_plugin(assignment.plugin_type.clone(), &effective_config, executor)
            .map_err(|e| e.to_string())?;

    match plugin
        .detect_installed_version(&assignment.package_identifier)
        .await
    {
        Ok(Some(version)) => {
            tracing::debug!(version = %version, "installed version detected");
            Ok(Some(version.to_string()))
        }
        Ok(None) => {
            tracing::debug!("no installed version detected");
            Ok(None)
        }
        Err(e) => Err(format!("detection failed: {e}")),
    }
}

/// Fetch the latest available version using a specific plugin assignment.
async fn fetch_latest(
    assignment: &PluginAssignment,
    executor: Arc<dyn CommandExecutor>,
    ctx: &ConnectionContext,
) -> Result<Option<String>, String> {
    tracing::debug!(
        plugin_type = ?assignment.plugin_type,
        package = %assignment.package_identifier,
        "fetching releases"
    );

    let mut effective_config = assignment.config.clone();
    ctx.apply_to_config(&assignment.plugin_type, &mut effective_config);

    let plugin =
        PluginRegistry::create_plugin(assignment.plugin_type.clone(), &effective_config, executor)
            .map_err(|e| e.to_string())?;

    match plugin.fetch_releases(&assignment.package_identifier).await {
        Ok(releases) => {
            tracing::debug!(count = releases.len(), "releases fetched");
            Ok(releases.first().map(|r| r.version.to_string()))
        }
        Err(e) => {
            tracing::debug!(error = %e, "failed to fetch latest version from plugin");
            Err(format!("fetch_releases failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::LocalCommandExecutor;
    use uptrakit_internal_wire::PluginType;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn no_ctx() -> ConnectionContext {
        ConnectionContext::default()
    }

    fn gh_assignment() -> PluginAssignment {
        PluginAssignment {
            plugin_type: PluginType::ReleasesGithub,
            package_identifier: "octocat/hello-world".to_string(),
            // GitHub plugin config no longer contains owner/repo — those are
            // expressed via package_identifier at the software item level.
            config: serde_json::json!({}),
        }
    }

    #[tokio::test]
    async fn check_version_github_detect_not_supported() {
        // The GitHub plugin is fetch-only; using it for detect returns an error.
        let assignment = gh_assignment();
        let outcome =
            check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        // The GitHub plugin's default detect_installed_version returns an error.
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_docker_stub_returns_none() {
        let assignment = PluginAssignment {
            plugin_type: PluginType::ReleasesDocker,
            package_identifier: "nginx".to_string(),
            config: serde_json::json!({}),
        };
        let outcome =
            check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }

    #[tokio::test]
    async fn check_version_proxmox_is_discovery_only() {
        // PHS is discovery-only; `detect_installed_version` is not supported.
        let assignment = PluginAssignment {
            plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
            package_identifier: "booklore".to_string(),
            config: serde_json::json!({}),
        };
        let outcome =
            check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        // The trait default returns an error for unsupported operations.
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_github_invalid_config() {
        // A non-https api_base_url fails GitHub config validation.
        let assignment = PluginAssignment {
            plugin_type: PluginType::ReleasesGithub,
            package_identifier: "octocat/hello-world".to_string(),
            config: serde_json::json!({"api_base_url": "http://api.github.com"}),
        };
        let outcome =
            check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.error.is_some());
        assert!(outcome.error.unwrap().contains("https"));
    }

    #[tokio::test]
    async fn check_version_homebrew_default_returns_none() {
        let assignment = PluginAssignment {
            plugin_type: PluginType::PackageManagerHomebrew,
            package_identifier: String::new(),
            config: serde_json::json!({}),
        };
        let outcome =
            check_version(Some(&assignment), None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_some());
    }

    #[tokio::test]
    async fn check_version_docker_context_injects_docker_host() {
        let assignment = PluginAssignment {
            plugin_type: PluginType::ReleasesDocker,
            package_identifier: "nginx".to_string(),
            config: serde_json::json!({}),
        };
        let ctx = ConnectionContext {
            docker_host_override: Some("ssh://user@host:2222".to_string()),
            ssh_key_path: None,
        };
        // With a valid docker host override, the plugin is created with the
        // injected host. The check itself will fail (no daemon) but that proves
        // the injection path runs without panicking.
        let outcome =
            check_version(Some(&assignment), None, test_executor(), &ctx).await;
        let _ = outcome;
    }

    #[tokio::test]
    async fn check_version_no_assignments_returns_empty() {
        let outcome =
            check_version(None, None, test_executor(), &no_ctx()).await;
        assert!(outcome.installed_version.is_none());
        assert!(outcome.latest_version.is_none());
        assert!(outcome.error.is_none());
    }
}
