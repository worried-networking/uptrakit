use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use rootcause::prelude::*;
use serde_json::json;
use uptrakit_plugin_infrastructure_core::mpsc;

use uptrakit_plugin_infrastructure_core::command::{
    CommandExecutor, CommandSpec, send_output, shell_escape,
};
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, OutputStreamType, Plugin, PluginCapability, PluginError, PluginType,
    ReleaseInfo, UpdateOutputLine, UpstreamRelease, Version,
};

use crate::config::{DockerConfig, TrackingMode};
use crate::docker_client::{BollardDockerClient, DockerClient};
use crate::error::{DockerError, Result};
use crate::image_ref::ImageRef;
use crate::registry::RegistryClient;
use crate::tag::filter_and_sort_tags;

/// Docker plugin implementation.
///
/// Tracks container image tags from OCI/Docker registries.
/// Supports two tracking modes:
/// - **SemverTags**: filter tags by pattern, parse as semver, sort descending
/// - **DigestTracking**: track digest changes of a specific tag
///
/// Also supports autodiscovery of running/stopped containers via Bollard.
pub struct DockerPlugin {
    config: DockerConfig,
    registry_client: RegistryClient,
    tag_filters: Vec<Regex>,
    docker_client: Arc<dyn DockerClient>,
    executor: Arc<dyn CommandExecutor>,
}

impl DockerPlugin {
    /// Create a new `DockerPlugin` from the given configuration.
    pub fn new(config: DockerConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        let docker_client = Arc::new(BollardDockerClient::new(
            config.docker_host.as_deref(),
            config.ssh_key_path.as_deref(),
        )?);
        Self::init(config, executor, docker_client)
    }

    /// Internal constructor that accepts any [`DockerClient`] implementation.
    fn init(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
    ) -> Result<Self> {
        config.validate()?;

        let registry_client = RegistryClient::new(config.auth.clone(), config.page_size)?;

        let tag_filters: Vec<Regex> = config
            .tag_patterns
            .iter()
            .map(|p| {
                Regex::new(p).context_transform(|e| {
                    DockerError::InvalidPattern(format!("invalid regex '{p}': {e}"))
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            config,
            registry_client,
            tag_filters,
            docker_client,
            executor,
        })
    }

    /// Test constructor that injects a custom [`DockerClient`].
    #[cfg(test)]
    pub(crate) fn new_for_test(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
    ) -> Result<Self> {
        Self::init(config, executor, docker_client)
    }

    /// Convert filtered tags to upstream releases (semver mode).
    fn tags_to_releases(&self, ir: &ImageRef, tags: Vec<String>) -> Vec<UpstreamRelease> {
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
                let release_url = ir.web_url(&tv.tag);
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
impl Plugin for DockerPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::ReleasesDocker
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        &[
            PluginCapability::DiscoverLocalSoftware,
            PluginCapability::ControllerSideFetchReleases,
        ]
    }

    async fn fetch_releases(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<UpstreamRelease>> {
        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
                })?;

        match self.config.tracking_mode {
            TrackingMode::SemverTags => {
                let tags = self
                    .registry_client
                    .list_tags(&ir.registry, &ir.repository)
                    .await
                    .context_to()?;

                let releases = self.tags_to_releases(&ir, tags);
                tracing::debug!(
                    count = releases.len(),
                    image = %ir.image,
                    "fetched Docker releases (semver mode)"
                );
                Ok(releases)
            }
            TrackingMode::DigestTracking => {
                let tag = self.config.resolved_tracked_tag();
                let digest = self
                    .registry_client
                    .get_manifest_digest(&ir.registry, &ir.repository, tag)
                    .await
                    .context_to()?;

                let release_url = ir.web_url(&digest);
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
                    image = %ir.image,
                    "fetched Docker release (digest mode)"
                );
                Ok(vec![release])
            }
        }
    }

    async fn detect_installed_version(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Option<Version>> {
        if self.config.tracking_mode != TrackingMode::DigestTracking {
            return Ok(None);
        }

        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    PluginError::PluginInternal(e.to_string())
                })?;

        let tag = self.config.resolved_tracked_tag();
        let full_ref = format!("{}:{tag}", ir.image);

        match self.docker_client.inspect_image(&full_ref).await {
            Ok(Some(digest_info)) => {
                tracing::debug!(
                    digest = %digest_info.digest,
                    image = %ir.image,
                    "detected installed digest"
                );
                Ok(Some(Version::new(&digest_info.digest)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(
                uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
                    .into(),
            ),
        }
    }

    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        _release_info: Option<&ReleaseInfo>,
        output_tx: &mpsc::Sender<UpdateOutputLine>,
    ) -> uptrakit_plugin_infrastructure_core::Result<String> {
        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    PluginError::PluginInternal(e.to_string())
                })?;

        let image = &ir.image;
        let tag = to_version;
        let mut output = String::new();

        send_output(
            output_tx,
            &format!("Pulling Docker image {image}:{tag}"),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

        tracing::debug!(image = %image, "pulling Docker image");
        let pull_output = self
            .docker_client
            .pull_image(image, tag, self.config.auth.as_ref(), output_tx)
            .await
            .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
        tracing::debug!("Docker image pull completed");
        output.push_str(&pull_output);

        // Run compose_restart if configured
        if let Some(ref cr) = self.config.compose_restart {
            let mut parts: Vec<String> = Vec::new();

            if let Some(ref working_dir) = cr.working_dir {
                parts.push(format!("cd {}", shell_escape(working_dir)));
                parts.push("&&".to_string());
            }

            parts.push("docker".to_string());
            parts.push("compose".to_string());

            if let Some(ref file) = cr.compose_file {
                parts.push("-f".to_string());
                parts.push(shell_escape(file));
            }

            parts.push("up".to_string());
            parts.push("-d".to_string());

            if let Some(ref service) = cr.service {
                parts.push(shell_escape(service));
            }

            let cmd = parts.join(" ");
            tracing::debug!(command = %cmd, "running docker compose restart");
            send_output(
                output_tx,
                &format!("Running docker compose: {cmd}"),
                OutputStreamType::Stdout,
            )
            .await;

            let cmd_output = self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        // Run post_pull_command if configured
        if let Some(ref cmd_str) = self.config.post_pull_command {
            // Try to get local digest for {digest} substitution
            let full_ref = format!("{image}:{tag}");
            let digest = match self.docker_client.inspect_image(&full_ref).await {
                Ok(Some(d)) => d.digest,
                _ => String::new(),
            };

            let cmd = cmd_str
                .replace("{image}", &shell_escape(image))
                .replace("{tag}", &shell_escape(tag))
                .replace("{digest}", &shell_escape(&digest));

            tracing::debug!(command = %cmd, "running post-pull command");
            send_output(
                output_tx,
                &format!("Running post-pull command: {cmd}"),
                OutputStreamType::Stdout,
            )
            .await;

            let cmd_output = self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        Ok(output)
    }

    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        use std::collections::HashMap;

        let containers = self
            .docker_client
            .list_containers(true)
            .await
            .map_err(|e| {
                uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
            })?;

        // Group container names by full image ref (image:tag).
        // Skip bare SHA images (sha256:…) — they have no registry provenance.
        let mut groups: HashMap<String, (ImageRef, Vec<String>)> = HashMap::new();

        for container in containers {
            let raw_image = container.image.trim();
            if raw_image.is_empty() {
                continue;
            }

            // Skip bare SHA refs
            if raw_image.starts_with("sha256:") {
                continue;
            }

            // Parse the image ref (may or may not have a tag)
            let ir: ImageRef = match raw_image.parse() {
                Ok(r) => r,
                Err(_) => continue,
            };

            let entry = groups
                .entry(ir.full_ref.clone())
                .or_insert_with(|| (ir, Vec::new()));

            entry.1.extend(container.names);
        }

        // For each unique image, inspect locally to get its digest.
        let mut discoveries = Vec::new();

        for (full_ref, (ir, container_names)) in groups {
            let digest = match self.docker_client.inspect_image(&full_ref).await {
                Ok(Some(d)) => d.digest,
                Ok(None) => {
                    // Locally built image — no registry digest, skip
                    tracing::debug!(
                        image = %full_ref,
                        "skipping locally built image (no RepoDigests)"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(image = %full_ref, error = %e, "failed to inspect image");
                    continue;
                }
            };

            let name = derive_container_name(&ir, &container_names);

            discoveries.push(DiscoveredSoftware {
                package_identifier: ir.full_ref.clone(),
                name,
                installed_version: digest,
                targets: vec![],
                extra: Some(json!({ "containers": container_names })),
            });
        }

        tracing::debug!(count = discoveries.len(), "docker autodiscovery completed");
        Ok(discoveries)
    }
}

/// Derive a human-readable name for a discovered container.
///
/// - Single container: use the container name (leading `/` already stripped).
/// - Multiple containers sharing the same image: use `"image:tag"`.
fn derive_container_name(ir: &ImageRef, container_names: &[String]) -> String {
    if container_names.len() == 1 {
        container_names[0].clone()
    } else {
        format!("{}:{}", ir.image, ir.tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrackingMode;
    use crate::docker_client::{LocalContainerInfo, MockDockerClient};
    use uptrakit_plugin_infrastructure_core::LocalCommandExecutor;
    use uptrakit_plugin_infrastructure_core::mpsc;

    fn test_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(LocalCommandExecutor)
    }

    /// A mock executor that records commands without executing them.
    struct MockCommandExecutor;

    #[async_trait::async_trait]
    impl CommandExecutor for MockCommandExecutor {
        async fn execute(
            &self,
            _spec: &CommandSpec,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> uptrakit_command::Result<uptrakit_plugin_infrastructure_core::CommandOutput> {
            Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }

        async fn execute_quiet(
            &self,
            _spec: &CommandSpec,
        ) -> uptrakit_command::Result<uptrakit_plugin_infrastructure_core::CommandOutput> {
            Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
                output: String::new(),
                exit_code: 0,
            })
        }
    }

    fn mock_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(MockCommandExecutor)
    }

    fn default_mock_client() -> Arc<dyn DockerClient> {
        Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: None,
            containers: vec![],
        })
    }

    #[test]
    fn plugin_creation_succeeds_with_empty_config() {
        let config = DockerConfig::default();
        assert!(DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).is_ok());
    }

    #[test]
    fn plugin_creation_fails_with_invalid_regex() {
        let config = DockerConfig {
            tag_patterns: vec!["[bad".to_string()],
            ..Default::default()
        };
        assert!(
            DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).is_err()
        );
    }

    #[test]
    fn capabilities_includes_discover_local_software() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        assert!(plugin.has_capability(PluginCapability::DiscoverLocalSoftware));
    }

    #[test]
    fn capabilities_excludes_refresh_package_index() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        assert!(!plugin.has_capability(PluginCapability::RefreshPackageIndex));
    }

    #[tokio::test]
    async fn detect_installed_version_semver_mode_returns_none() {
        let config = DockerConfig {
            tracking_mode: TrackingMode::SemverTags,
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).unwrap();
        let result = plugin.detect_installed_version("nginx").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_installed_version_digest_mode_returns_digest() {
        let digest = "sha256:abc123def456".to_string();
        let config = DockerConfig {
            tracking_mode: TrackingMode::DigestTracking,
            ..Default::default()
        };
        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: Some(digest.clone()),
            containers: vec![],
        });
        let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
        let result = plugin.detect_installed_version("nginx").await.unwrap();
        assert_eq!(result.map(|v| v.to_string()), Some(digest));
    }

    #[tokio::test]
    async fn execute_update_calls_docker_client() {
        let pull_output = "mock pull output".to_string();
        let mock = Arc::new(MockDockerClient {
            pull_output: pull_output.clone(),
            pull_should_fail: false,
            inspect_result: None,
            containers: vec![],
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "1.25.0", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(result.contains("Pulling Docker image nginx:1.25.0"));
        assert!(result.contains(&pull_output));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_pull_failure_propagates_error() {
        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: true,
            inspect_result: None,
            containers: vec![],
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin.execute_update("nginx", "1.25.0", None, &tx).await;

        assert!(result.is_err(), "pull failure should be propagated");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_post_pull_command() {
        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: None,
            containers: vec![],
        });
        let config = DockerConfig {
            post_pull_command: Some("echo post-pull {image}:{tag}".to_string()),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "1.25.0", None, &tx)
            .await
            .expect("execute_update with post_pull_command should succeed");

        assert!(result.contains("Pulling Docker image nginx:1.25.0"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_compose_restart() {
        use crate::config::ComposeRestartConfig;

        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: None,
            containers: vec![],
        });
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: Some("docker-compose.yml".to_string()),
                service: Some("myapp".to_string()),
                working_dir: None,
            }),
            ..Default::default()
        };
        // Use a mock executor so the docker compose command is not actually run.
        let plugin =
            DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "1.25.0", None, &tx)
            .await
            .expect("execute_update with compose_restart should succeed");

        assert!(result.contains("Pulling Docker image nginx:1.25.0"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn discover_software_groups_by_image() {
        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["my-nginx".to_string()],
                },
                LocalContainerInfo {
                    image: "nginx:latest".to_string(),
                    names: vec!["nginx-2".to_string()],
                },
            ],
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        // Both containers share the same image, so they should be grouped
        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].package_identifier, "nginx:latest");
        assert_eq!(discoveries[0].installed_version, "sha256:abc123");
    }

    #[tokio::test]
    async fn discover_software_skips_sha_images() {
        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "sha256:deadbeef".to_string(),
                names: vec!["bare-sha-container".to_string()],
            }],
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert!(discoveries.is_empty(), "SHA images should be skipped");
    }

    #[tokio::test]
    async fn discover_software_skips_images_without_repo_digests() {
        let mock = Arc::new(MockDockerClient {
            pull_output: String::new(),
            pull_should_fail: false,
            inspect_result: None, // No digest — locally built
            containers: vec![LocalContainerInfo {
                image: "my-local-image:dev".to_string(),
                names: vec!["local-container".to_string()],
            }],
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert!(
            discoveries.is_empty(),
            "images without RepoDigests should be skipped"
        );
    }

    #[test]
    fn tags_to_releases_basic() {
        let mock = default_mock_client();
        let config = DockerConfig {
            tag_strip_prefix: String::new(),
            ..Default::default()
        };
        let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
        let ir: ImageRef = "nginx".parse().unwrap();
        let tags = vec![
            "1.25.0".to_string(),
            "1.24.0".to_string(),
            "1.26.0".to_string(),
            "latest".to_string(),
            "alpine".to_string(),
        ];
        let releases = plugin.tags_to_releases(&ir, tags);
        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0].version.as_str(), "1.26.0");
        assert_eq!(releases[1].version.as_str(), "1.25.0");
        assert_eq!(releases[2].version.as_str(), "1.24.0");
    }
}
