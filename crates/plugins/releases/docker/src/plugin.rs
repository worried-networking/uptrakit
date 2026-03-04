use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use serde_json::json;
use uptrakit_plugin_infrastructure_core::mpsc;

use uptrakit_plugin_infrastructure_core::command::{
    CommandExecutor, CommandSpec, send_output, shell_escape,
};
use uptrakit_plugin_infrastructure_core::{
    DiscoveredSoftware, OutputStreamType, Plugin, PluginCapability, PluginError, PluginType,
    ReleaseInfo, TrackingSystem, UpdateOutputLine, UpstreamRelease, Version,
};

/// Type-erased RAII handle kept alive alongside the Docker client.
type OpaqueHandle = Option<Box<dyn std::any::Any + Send + Sync>>;

use crate::config::DockerConfig;
#[cfg(feature = "daemon")]
use crate::docker_client::BollardDockerClient;
use crate::docker_client::{DockerClient, NoopDockerClient};
use crate::error::Result;
use crate::image_ref::ImageRef;
use crate::registry::RegistryClient;
#[cfg(feature = "daemon")]
use uptrakit_plugin_infrastructure_core::HostCompatibility;

/// Docker plugin implementation.
///
/// Tracks container image updates by monitoring the SHA-256 manifest digest
/// of a specific tag (e.g. `latest`). When the remote digest differs from the
/// locally installed digest, an update is available.
///
/// Also supports autodiscovery of running/stopped containers via Bollard.
pub struct DockerPlugin {
    config: DockerConfig,
    registry_client: RegistryClient,
    docker_client: Arc<dyn DockerClient>,
    executor: Arc<dyn CommandExecutor>,
    /// RAII handle for the Docker socket proxy (Unix-only, daemon feature).
    ///
    /// When an executor supports stdio tunnels and no explicit `docker_host`
    /// is configured, a [`crate::docker_proxy::DockerSocketProxy`] is started
    /// and stored here. The proxy is stopped and the socket removed when the
    /// plugin is dropped.
    _proxy_handle: OpaqueHandle,
}

impl DockerPlugin {
    /// Create a new `DockerPlugin` from the given configuration.
    ///
    /// With the `daemon` feature enabled, connects to the Docker daemon via
    /// bollard. Without it, uses [`NoopDockerClient`] so the plugin can still
    /// serve registry-only capabilities (e.g. `ControllerSideFetchReleases`).
    pub async fn new(config: DockerConfig, executor: Arc<dyn CommandExecutor>) -> Result<Self> {
        // Always create a NoopDockerClient and a None proxy handle as
        // starting values. When the `daemon` feature is enabled,
        // upgrade_to_daemon_client replaces them with a real
        // BollardDockerClient (and optionally a proxy handle), consuming
        // the stub values in the process.
        let docker_client: Arc<dyn DockerClient> = Arc::new(NoopDockerClient);
        let proxy_handle: OpaqueHandle = None;
        #[cfg(feature = "daemon")]
        let (docker_client, proxy_handle) =
            Self::upgrade_to_daemon_client(docker_client, proxy_handle, &config, &executor).await?;
        Self::init(config, executor, docker_client, proxy_handle)
    }

    /// Replace a stub Docker client with a real [`BollardDockerClient`].
    ///
    /// When the executor supports stdio tunnels (e.g. SSH) and no explicit
    /// `docker_host` is configured, a [`crate::docker_proxy::DockerSocketProxy`]
    /// is started and the client connects to the local proxy socket. Otherwise
    /// falls through to the standard bollard connection logic.
    ///
    /// The `_stub` parameter is the [`NoopDockerClient`] created unconditionally
    /// in [`Self::new`]. Accepting it here ensures the initial binding is read,
    /// suppressing `unused_assignments` and `dead_code` lints, while making it
    /// explicit that the daemon path fully replaces the stub.
    #[cfg(feature = "daemon")]
    async fn upgrade_to_daemon_client(
        _stub: Arc<dyn DockerClient>,
        _proxy_stub: OpaqueHandle,
        config: &DockerConfig,
        executor: &Arc<dyn CommandExecutor>,
    ) -> Result<(Arc<dyn DockerClient>, OpaqueHandle)> {
        // When the executor supports stdio tunnels and no explicit docker_host
        // is configured, start a local Unix socket proxy. This avoids bollard's
        // SSH codepath (which spawns a second SSH connection via the system ssh
        // binary) and instead tunnels Docker API traffic over the existing
        // russh session.
        #[cfg(unix)]
        if executor.supports_stdio_tunnel() && config.docker_host.is_none() {
            let proxy = crate::docker_proxy::DockerSocketProxy::start(Arc::clone(executor)).await?;
            let uri = proxy.socket_uri();
            tracing::info!(
                proxy_socket = %uri,
                "Docker socket proxy started; connecting bollard via proxy"
            );
            let client = Arc::new(BollardDockerClient::new(Some(&uri), None)?);
            let handle: Box<dyn std::any::Any + Send + Sync> = Box::new(proxy);
            return Ok((client, Some(handle)));
        }

        let client = Arc::new(BollardDockerClient::new(
            config.docker_host.as_deref(),
            config.ssh_key_path.as_deref(),
        )?);
        Ok((client, None))
    }

    /// Internal constructor that accepts any [`DockerClient`] implementation.
    fn init(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
        proxy_handle: OpaqueHandle,
    ) -> Result<Self> {
        config.validate()?;

        let registry_client = RegistryClient::new(config.auth.clone())?;

        Ok(Self {
            config,
            registry_client,
            docker_client,
            executor,
            _proxy_handle: proxy_handle,
        })
    }

    /// Compile-time capabilities for the Docker plugin.
    ///
    /// Read directly by the registry macro for sync capability queries.
    pub const CAPABILITIES: &'static [PluginCapability] = if cfg!(feature = "daemon") {
        &[
            PluginCapability::ControllerSideFetchReleases,
            PluginCapability::DiscoverLocalSoftware,
            PluginCapability::DetectHostCompatibility,
        ]
    } else {
        &[PluginCapability::ControllerSideFetchReleases]
    };

    /// Test constructor that injects a custom [`DockerClient`].
    #[cfg(all(test, feature = "daemon"))]
    pub(crate) fn new_for_test(
        config: DockerConfig,
        executor: Arc<dyn CommandExecutor>,
        docker_client: Arc<dyn DockerClient>,
    ) -> Result<Self> {
        Self::init(config, executor, docker_client, None)
    }
}

#[async_trait]
impl Plugin for DockerPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::ReleasesDocker
    }

    fn capabilities(&self) -> &'static [PluginCapability] {
        Self::CAPABILITIES
    }

    #[cfg(feature = "daemon")]
    async fn detect_host_compatibility(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<HostCompatibility> {
        // Ping the Docker daemon directly rather than checking for the CLI binary.
        // This validates that the daemon is actually running and reachable (including
        // over SSH tunnels for remote hosts), not just that the docker binary exists.
        //
        // Apply a short timeout so that a frozen or unresponsive daemon (e.g.
        // Docker Desktop restarting) does not block host-compatibility probing
        // indefinitely. The bollard `connect_with_defaults()` path carries no
        // explicit request timeout, so without this guard a single slow daemon
        // stalls the entire sudoers-generation step for the duration of the OS
        // socket/HTTP timeout (potentially minutes).
        const COMPAT_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
        match tokio::time::timeout(COMPAT_PROBE_TIMEOUT, self.docker_client.ping()).await {
            Ok(Ok(())) => Ok(HostCompatibility::Compatible),
            Ok(Err(e)) => Ok(HostCompatibility::Incompatible(format!(
                "Docker daemon not accessible: {e}"
            ))),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "Docker daemon ping timed out".to_string(),
            )),
        }
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

        let tag = self.config.resolved_tracked_tag(&ir.tag);
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
            category: None,
        };

        tracing::debug!(
            digest = %digest,
            tag = %tag,
            image = %ir.image,
            "fetched Docker release (digest mode)"
        );
        Ok(vec![release])
    }

    async fn detect_installed_version(
        &self,
        package_identifier: &str,
    ) -> uptrakit_plugin_infrastructure_core::Result<Option<Version>> {
        let ir: ImageRef =
            package_identifier
                .parse()
                .map_err(|e: crate::image_ref::ParseImageRefError| {
                    PluginError::PluginInternal(e.to_string())
                })?;

        let tag = self.config.resolved_tracked_tag(&ir.tag);
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
        _to_version: &str,
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
        // Always pull by the configured tag (e.g. "latest"), not by digest.
        let tag = self.config.resolved_tracked_tag(&ir.tag);
        let full_ref = format!("{image}:{tag}");
        let mut output = String::new();

        // Pre-pull: collect running/stopped state of containers using this image.
        // Used for compose direction (up -d vs --no-start) and for auto-recreation.
        let containers_before = self
            .docker_client
            .list_containers_for_image(&full_ref)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    image = %full_ref,
                    error = %e,
                    "failed to list containers before pull; recreation will be skipped"
                );
                vec![]
            });

        send_output(
            output_tx,
            &format!("Pulling Docker image {image}:{tag}"),
            OutputStreamType::Stdout,
        )
        .await;
        output.push_str(&format!("Pulling Docker image {image}:{tag}\n"));

        tracing::debug!(image = %image, tag = %tag, "pulling Docker image");
        let pull_output = self
            .docker_client
            .pull_image(image, tag, self.config.auth.as_ref(), output_tx)
            .await
            .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
        tracing::debug!("Docker image pull completed");
        output.push_str(&pull_output);

        // Run compose_restart if configured.
        // Direction: any containers running before pull → `up -d` (recreate and start);
        // all stopped → `up --no-start` (recreate without starting).
        if let Some(ref cr) = self.config.compose_restart {
            let any_running = containers_before.iter().any(|c| c.is_running);

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
            if any_running {
                parts.push("-d".to_string());
            } else {
                parts.push("--no-start".to_string());
            }

            if let Some(ref service) = cr.service {
                parts.push(shell_escape(service));
            }

            let cmd = parts.join(" ");
            tracing::debug!(command = %cmd, "running docker compose restart");
            let compose_msg = format!("Running docker compose: {cmd}");
            send_output(output_tx, &compose_msg, OutputStreamType::Stdout).await;
            output.push_str(&compose_msg);
            output.push('\n');

            let cmd_output = self
                .executor
                .execute(&CommandSpec::shell(&cmd), output_tx)
                .await
                .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            output.push_str(&cmd_output.output);
        }

        // Run post_pull_command if configured.
        if let Some(ref cmd_str) = self.config.post_pull_command {
            // Try to get local digest for {digest} substitution.
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

        // Auto-recreate containers when neither compose_restart nor post_pull_command
        // is configured.  Containers are recreated in-place, preserving all settings.
        // Running containers are started again; stopped containers remain stopped.
        if self.config.compose_restart.is_none() && self.config.post_pull_command.is_none() {
            for container in &containers_before {
                tracing::info!(
                    container = %container.name,
                    was_running = container.is_running,
                    "recreating container after image update"
                );
                let line = format!(
                    "Recreating container {} (was {})",
                    container.name,
                    if container.is_running {
                        "running"
                    } else {
                        "stopped"
                    }
                );
                send_output(output_tx, &line, OutputStreamType::Stdout).await;
                output.push_str(&line);
                output.push('\n');

                self.docker_client
                    .recreate_container(&container.name, container.is_running)
                    .await
                    .context_transform(|e| PluginError::InstallFailed(e.to_string()))?;
            }
        }

        Ok(output)
    }

    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        use std::collections::HashMap;
        use uptrakit_plugin_infrastructure_core::{DiscoveryTarget, PluginRole};

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

        // When the plugin was invoked without a pre-existing plugin config
        // (config is all-defaults / `{}`), emit a DiscoveryTarget so the
        // controller can auto-create a default "Docker" plugin config and
        // the role assignments.  When a real config exists, the server
        // sends plugin_config_id and the items are handled via the
        // config-ID path (no targets needed).
        let emit_targets = self.config.is_discover_all_mode();

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

            let targets = if emit_targets {
                vec![DiscoveryTarget {
                    plugin_type: PluginType::ReleasesDocker,
                    plugin_config: json!({}),
                    plugin_config_name: "Docker".to_string(),
                    roles: vec![
                        PluginRole::DetectVersion,
                        PluginRole::FetchReleases,
                        PluginRole::ExecuteUpdate,
                    ],
                    package_identifier: None,
                    config_override: None,
                    execution_site: None,
                }]
            } else {
                vec![]
            };

            discoveries.push(DiscoveredSoftware {
                package_identifier: ir.full_ref.clone(),
                name,
                installed_version: digest,
                targets,
                extra: Some(json!({ "containers": container_names })),
                tracking_system: TrackingSystem::Targeted,
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

#[cfg(all(test, feature = "daemon"))]
mod tests {
    use super::*;
    use crate::docker_client::{ContainerForImage, LocalContainerInfo, MockDockerClient};
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
        Arc::new(MockDockerClient::default())
    }

    #[test]
    fn plugin_creation_succeeds_with_empty_config() {
        let config = DockerConfig::default();
        assert!(DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).is_ok());
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
    fn capabilities_includes_detect_host_compatibility() {
        let plugin = DockerPlugin::new_for_test(
            DockerConfig::default(),
            test_executor(),
            default_mock_client(),
        )
        .unwrap();
        assert!(plugin.has_capability(PluginCapability::DetectHostCompatibility));
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

    // ── detect_host_compatibility ─────────────────────────────────────────────

    #[tokio::test]
    async fn detect_host_compatibility_compatible_when_daemon_reachable() {
        let mock = Arc::new(MockDockerClient::default());
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        assert_eq!(result, HostCompatibility::Compatible);
    }

    #[tokio::test]
    async fn detect_host_compatibility_incompatible_when_daemon_unreachable() {
        let mock = Arc::new(MockDockerClient {
            ping_should_fail: true,
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let result = plugin.detect_host_compatibility().await.expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert!(
                    msg.contains("Docker daemon"),
                    "reason should mention Docker daemon: {msg}"
                );
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn detect_host_compatibility_incompatible_when_daemon_times_out() {
        let mock = Arc::new(MockDockerClient {
            ping_should_hang: true,
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        // Spawn so we can advance virtual time while the probe is in flight.
        let check = tokio::spawn(async move { plugin.detect_host_compatibility().await });
        tokio::task::yield_now().await;
        // Advance past the 5-second COMPAT_PROBE_TIMEOUT.
        tokio::time::advance(std::time::Duration::from_secs(10)).await;
        let result = check.await.expect("join").expect("ok");
        match result {
            HostCompatibility::Incompatible(msg) => {
                assert!(
                    msg.contains("timed out"),
                    "reason should mention timeout: {msg}"
                );
            }
            HostCompatibility::Compatible => panic!("expected Incompatible"),
            _ => panic!("unexpected HostCompatibility variant"),
        }
    }

    #[tokio::test]
    async fn detect_installed_version_returns_digest_when_image_present() {
        let digest = "sha256:abc123def456".to_string();
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some(digest.clone()),
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let result = plugin.detect_installed_version("nginx").await.unwrap();
        assert_eq!(result.map(|v| v.to_string()), Some(digest));
    }

    #[tokio::test]
    async fn detect_installed_version_returns_none_when_image_absent() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: None,
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let result = plugin.detect_installed_version("nginx").await.unwrap();
        assert!(result.is_none());
    }

    // ── execute_update ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_pulls_by_tag_not_digest() {
        let pull_output = "mock pull output".to_string();
        let mock = Arc::new(MockDockerClient {
            pull_output: pull_output.clone(),
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        // `to_version` is the digest — execute_update must pull by tag ("latest"), not by digest.
        let result = plugin
            .execute_update("nginx", "sha256:deadbeef", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(
            result.contains("Pulling Docker image nginx:latest"),
            "should pull by tag, not digest: {result}"
        );
        assert!(result.contains(&pull_output));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_pull_failure_propagates_error() {
        let mock = Arc::new(MockDockerClient {
            pull_should_fail: true,
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:deadbeef", None, &tx)
            .await;

        assert!(result.is_err(), "pull failure should be propagated");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_recreates_running_containers() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(result.contains("Recreating container my-nginx"));
        assert!(result.contains("running"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_recreates_stopped_containers() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "stopped-nginx".to_string(),
                is_running: false,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update should succeed");

        assert!(result.contains("Recreating container stopped-nginx"));
        assert!(result.contains("stopped"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_recreate_failure_propagates_error() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "bad-container".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            recreate_should_fail: true,
            ..Default::default()
        });
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await;

        assert!(result.is_err(), "recreate failure should propagate");

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_no_containers_succeeds() {
        // No containers for this image — pull succeeds, recreation loop is a no-op.
        let mock = Arc::new(MockDockerClient::default());
        let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
            .expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("should succeed with no containers");

        assert!(result.contains("Pulling Docker image nginx:latest"));

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_post_pull_command_skips_recreation() {
        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let config = DockerConfig {
            post_pull_command: Some("echo post-pull {image}:{tag}".to_string()),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update with post_pull_command should succeed");

        assert!(result.contains("Pulling Docker image nginx:latest"));
        // post_pull_command is set, so auto-recreate must be skipped
        assert!(
            !result.contains("Recreating container"),
            "recreation should be skipped when post_pull_command is set"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_compose_restart_running_uses_detach() {
        use crate::config::ComposeRestartConfig;

        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: true,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: Some("docker-compose.yml".to_string()),
                service: Some("myapp".to_string()),
                working_dir: None,
            }),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update with compose_restart should succeed");

        // When containers were running, compose command must include `-d`
        assert!(result.contains("docker compose"));
        assert!(result.contains("-d"), "running state should use -d flag");
        assert!(
            !result.contains("--no-start"),
            "should not use --no-start when containers were running"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_with_compose_restart_stopped_uses_no_start() {
        use crate::config::ComposeRestartConfig;

        let mock = Arc::new(MockDockerClient {
            containers_for_image: vec![ContainerForImage {
                name: "my-nginx".to_string(),
                is_running: false,
                labels: Default::default(),
            }],
            ..Default::default()
        });
        let config = DockerConfig {
            compose_restart: Some(ComposeRestartConfig {
                compose_file: None,
                service: Some("myapp".to_string()),
                working_dir: None,
            }),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("execute_update with compose_restart should succeed");

        // When containers were stopped, compose command must include `--no-start`
        assert!(result.contains("docker compose"));
        assert!(
            result.contains("--no-start"),
            "stopped state should use --no-start flag"
        );
        assert!(
            !result.contains(" -d "),
            "should not use -d when containers were stopped"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_tracked_tag_override_respected() {
        let mock = Arc::new(MockDockerClient::default());
        let config = DockerConfig {
            tracked_tag: Some("stable".to_string()),
            ..Default::default()
        };
        let plugin =
            DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("nginx", "sha256:abc", None, &tx)
            .await
            .expect("should succeed");

        assert!(
            result.contains("Pulling Docker image nginx:stable"),
            "should pull by configured tracked_tag: {result}"
        );

        rx.close();
        while rx.recv().await.is_some() {}
    }

    // ── discover_software ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn discover_software_groups_by_image() {
        let mock = Arc::new(MockDockerClient {
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
            ..Default::default()
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
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "sha256:deadbeef".to_string(),
                names: vec!["bare-sha-container".to_string()],
            }],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert!(discoveries.is_empty(), "SHA images should be skipped");
    }

    #[tokio::test]
    async fn discover_software_skips_images_without_repo_digests() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: None, // No digest — locally built
            containers: vec![LocalContainerInfo {
                image: "my-local-image:dev".to_string(),
                names: vec!["local-container".to_string()],
            }],
            ..Default::default()
        });
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();
        assert!(
            discoveries.is_empty(),
            "images without RepoDigests should be skipped"
        );
    }

    // ── discover_software target emission ─────────────────────────────────────

    #[tokio::test]
    async fn discover_software_emits_targets_when_default_config() {
        use uptrakit_plugin_infrastructure_core::{PluginRole, PluginType};

        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["my-nginx".to_string()],
            }],
            ..Default::default()
        });
        // Default config (empty `{}`) → discover-all mode → targets must be emitted.
        let plugin =
            DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();

        assert_eq!(discoveries.len(), 1);
        assert_eq!(discoveries[0].targets.len(), 1);
        let target = &discoveries[0].targets[0];
        assert_eq!(target.plugin_type, PluginType::ReleasesDocker);
        assert_eq!(target.plugin_config_name, "Docker");
        assert_eq!(target.plugin_config, serde_json::json!({}));
        assert!(target.roles.contains(&PluginRole::DetectVersion));
        assert!(target.roles.contains(&PluginRole::FetchReleases));
        assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
    }

    #[tokio::test]
    async fn discover_software_no_targets_when_custom_config() {
        let mock = Arc::new(MockDockerClient {
            inspect_result: Some("sha256:abc123".to_string()),
            containers: vec![LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["my-nginx".to_string()],
            }],
            ..Default::default()
        });
        // Non-default config (docker_host set) → config-ID path → no targets.
        let config = DockerConfig {
            docker_host: Some("unix:///var/run/docker.sock".to_string()),
            ..Default::default()
        };
        let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
        let discoveries = plugin.discover_software().await.unwrap();

        assert_eq!(discoveries.len(), 1);
        assert!(
            discoveries[0].targets.is_empty(),
            "customized config must not emit targets (config-ID path)"
        );
    }
}
