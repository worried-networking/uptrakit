use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use uptrakit_provider_core::mpsc;

use uptrakit_provider_core::command::{CommandExecutor, CommandSpec, send_output, shell_escape};
use uptrakit_provider_core::{
    OutputStreamType, Provider, ProviderError, ProviderType, ReleaseInfo, UpdateOutputLine,
    UpstreamRelease, Version,
};

use crate::config::{DockerRegistryConfig, TrackingMode};
use crate::docker_puller::{BollardDockerPuller, DockerPuller};
use crate::error::{DockerRegistryError, Result};
use crate::registry::RegistryClient;
use crate::tag::filter_and_sort_tags;

/// Docker Registry provider implementation.
///
/// Tracks container image tags from OCI/Docker registries.
/// Supports two tracking modes:
/// - **SemverTags**: filter tags by pattern, parse as semver, sort descending
/// - **DigestTracking**: track digest changes of a specific tag
///
/// Image pulls communicate with the Docker daemon directly via bollard,
/// without requiring the `docker` CLI binary.
pub struct DockerRegistryProvider {
    config: DockerRegistryConfig,
    registry_client: RegistryClient,
    tag_filters: Vec<Regex>,
    docker_puller: Arc<dyn DockerPuller>,
    executor: Arc<dyn CommandExecutor>,
}

impl DockerRegistryProvider {
    /// Create a new `DockerRegistryProvider` from the given configuration.
    ///
    /// Validates the configuration, pre-compiles tag filter regexes, and
    /// connects the bollard Docker client to the local daemon socket.
    pub fn new(config: DockerRegistryConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        let docker_puller = Arc::new(BollardDockerPuller::new()?);
        Self::init(config, executor, docker_puller)
    }

    /// Internal constructor that accepts any [`DockerPuller`] implementation.
    fn init(
        config: DockerRegistryConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_puller: Arc<dyn DockerPuller>,
    ) -> Result<Self> {
        config.validate()?;

        let registry_client = RegistryClient::new(&config)?;

        let tag_filters: Vec<Regex> = config
            .tag_patterns
            .iter()
            .map(|p| {
                Regex::new(p).context_transform(|e| {
                    DockerRegistryError::InvalidPattern(format!("invalid regex '{p}': {e}"))
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            config,
            registry_client,
            tag_filters,
            docker_puller,
            executor,
        })
    }

    /// Test constructor that injects a custom [`DockerPuller`].
    #[cfg(test)]
    pub(crate) fn new_for_test(
        config: DockerRegistryConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_puller: Arc<dyn DockerPuller>,
    ) -> Result<Self> {
        Self::init(config, executor, docker_puller)
    }

    /// Convert filtered tags to upstream releases (semver mode).
    fn tags_to_releases(&self, tags: Vec<String>) -> Vec<UpstreamRelease> {
        let sorted = filter_and_sort_tags(
            &tags,
            &self.tag_filters,
            &self.config.tag_strip_prefix,
            self.config.include_prereleases,
        );

        sorted
            .into_iter()
            .map(|tv| {
                let is_prerelease = !tv.semver.pre.is_empty();
                let release_url = self.config.image_web_url(&tv.tag);
                UpstreamRelease {
                    version: Version::new(tv.version_str),
                    tag: tv.tag,
                    is_prerelease,
                    release_url,
                    release_notes: None,
                    published_at: None,
                    assets: vec![],
                }
            })
            .collect()
    }
}

#[async_trait]
impl Provider for DockerRegistryProvider {
    fn provider_type(&self) -> ProviderType {
        ProviderType::DockerRegistry
    }

    async fn fetch_releases(
        &self,
        _package_identifier: &str,
    ) -> uptrakit_provider_core::Result<Vec<UpstreamRelease>> {
        match self.config.tracking_mode {
            TrackingMode::SemverTags => {
                let tags = self.registry_client.list_tags().await.context_to()?;

                let releases = self.tags_to_releases(tags);
                tracing::debug!(
                    count = releases.len(),
                    image = %self.config.image,
                    "fetched Docker Registry releases (semver mode)"
                );
                Ok(releases)
            }
            TrackingMode::DigestTracking => {
                let tag = self.config.resolved_tracked_tag();
                let digest = self
                    .registry_client
                    .get_manifest_digest(tag)
                    .await
                    .context_to()?;

                let release_url = self.config.image_web_url(tag);
                let release = UpstreamRelease {
                    version: Version::new(&digest),
                    tag: tag.to_string(),
                    is_prerelease: false,
                    release_url,
                    release_notes: None,
                    published_at: None,
                    assets: vec![],
                };

                tracing::debug!(
                    digest = %digest,
                    tag = %tag,
                    image = %self.config.image,
                    "fetched Docker Registry release (digest mode)"
                );
                Ok(vec![release])
            }
        }
    }

    async fn detect_installed_version(
        &self,
        _package_identifier: &str,
    ) -> uptrakit_provider_core::Result<Option<Version>> {
        Ok(None)
    }

    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_provider_core::Result<String> {
        let mut output = String::new();

        let image = package_identifier;
        let tag = to_version;

        send_output(
            output_tx,
            &format!("Pulling Docker image {image}:{tag}"),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

        // Pull the image via bollard (direct daemon API — no `docker` CLI required).
        let pull_output = self
            .docker_puller
            .pull_image(image, tag, self.config.auth.as_ref(), output_tx)
            .await
            .context_transform(|e| ProviderError::InstallFailed(e.to_string()))?;
        output.push_str(&pull_output);

        if let Some(ref cmd_str) = self.config.restart_command {
            let cmd = cmd_str
                .replace("{image}", &shell_escape(image))
                .replace("{tag}", &shell_escape(tag))
                .replace("{version}", &shell_escape(to_version));

            send_output(
                output_tx,
                &format!("Running restart command: {cmd}"),
                OutputStreamType::Stdout,
            )
            .await;

            let cmd_output = self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| ProviderError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrackingMode;
    use crate::docker_puller::MockDockerPuller;
    use uptrakit_provider_core::LocalCommandExecutor;
    use uptrakit_provider_core::mpsc;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    fn test_config() -> DockerRegistryConfig {
        DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 100,
            restart_command: None,
        }
    }

    fn test_provider() -> DockerRegistryProvider {
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        DockerRegistryProvider::new_for_test(test_config(), test_executor(), puller)
            .expect("valid config")
    }

    #[test]
    fn provider_creation_succeeds() {
        let config = test_config();
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        assert!(DockerRegistryProvider::new_for_test(config, test_executor(), puller).is_ok());
    }

    #[test]
    fn provider_creation_fails_with_invalid_config() {
        let config = DockerRegistryConfig {
            image: String::new(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec![],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 100,
            restart_command: None,
        };
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        assert!(DockerRegistryProvider::new_for_test(config, test_executor(), puller).is_err());
    }

    #[test]
    fn provider_creation_fails_with_invalid_regex() {
        let config = DockerRegistryConfig {
            image: "nginx".to_string(),
            registry: None,
            auth: None,
            tracking_mode: TrackingMode::SemverTags,
            tag_patterns: vec!["[bad".to_string()],
            tag_strip_prefix: "v".to_string(),
            include_prereleases: false,
            tracked_tag: None,
            page_size: 100,
            restart_command: None,
        };
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        assert!(DockerRegistryProvider::new_for_test(config, test_executor(), puller).is_err());
    }

    #[test]
    fn tags_to_releases_basic() {
        let provider = test_provider();
        let tags = vec![
            "1.25.0".to_string(),
            "1.24.0".to_string(),
            "1.26.0".to_string(),
            "latest".to_string(),
            "alpine".to_string(),
        ];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0].version.as_str(), "1.26.0");
        assert_eq!(releases[1].version.as_str(), "1.25.0");
        assert_eq!(releases[2].version.as_str(), "1.24.0");
    }

    #[test]
    fn tags_to_releases_with_prefix() {
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        let mut config = test_config();
        config.tag_strip_prefix = "v".to_string();
        let provider =
            DockerRegistryProvider::new_for_test(config, test_executor(), puller).expect("valid");
        let tags = vec!["v1.0.0".to_string(), "v2.0.0".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].version.as_str(), "2.0.0");
        assert_eq!(releases[0].tag, "v2.0.0");
    }

    #[test]
    fn tags_to_releases_no_semver_tags() {
        let provider = test_provider();
        let tags = vec!["latest".to_string(), "alpine".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert!(releases.is_empty());
    }

    #[test]
    fn tags_to_releases_release_url() {
        let provider = test_provider();
        let tags = vec!["1.25.0".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 1);
        assert!(releases[0].release_url.contains("hub.docker.com"));
        assert!(releases[0].release_url.contains("1.25.0"));
    }

    #[test]
    fn tags_to_releases_prerelease_detection() {
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        let mut config = test_config();
        config.include_prereleases = true;
        let provider =
            DockerRegistryProvider::new_for_test(config, test_executor(), puller).expect("valid");
        let tags = vec!["1.0.0".to_string(), "2.0.0-beta.1".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 2);
        assert!(!releases[1].is_prerelease); // 1.0.0
        assert!(releases[0].is_prerelease); // 2.0.0-beta.1
    }

    #[test]
    fn tags_to_releases_no_release_notes_or_published_at() {
        let provider = test_provider();
        let tags = vec!["1.0.0".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert!(releases[0].release_notes.is_none());
        assert!(releases[0].published_at.is_none());
        assert!(releases[0].assets.is_empty());
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none() {
        let provider = test_provider();
        let result = provider.detect_installed_version("example").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_update_calls_docker_puller() {
        let pull_output = "mock pull output".to_string();
        let puller = Arc::new(MockDockerPuller {
            output: pull_output.clone(),
            should_fail: false,
        });
        let provider =
            DockerRegistryProvider::new_for_test(test_config(), test_executor(), puller)
                .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = provider
            .execute_update("nginx", "1.25.0", None, &tx)
            .await
            .expect("execute_update should succeed");

        // Accumulated output contains the status prefix and the mock pull output.
        assert!(result.contains("Pulling Docker image nginx:1.25.0"));
        assert!(result.contains(&pull_output));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_pull_failure_propagates_error() {
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: true,
        });
        let provider =
            DockerRegistryProvider::new_for_test(test_config(), test_executor(), puller)
                .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = provider
            .execute_update("nginx", "1.25.0", None, &tx)
            .await;

        assert!(result.is_err(), "pull failure should be propagated");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_restart_command() {
        let puller = Arc::new(MockDockerPuller {
            output: String::new(),
            should_fail: false,
        });
        let mut config = test_config();
        config.restart_command = Some("echo restarting {image}:{tag}".to_string());
        let provider =
            DockerRegistryProvider::new_for_test(config, test_executor(), puller)
                .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = provider
            .execute_update("nginx", "1.25.0", None, &tx)
            .await
            .expect("execute_update with restart command should succeed");

        assert!(result.contains("Pulling Docker image nginx:1.25.0"));
        assert!(result.contains("restarting"));

        rx.close();
        while rx.recv().await.is_some() {}
    }
}
