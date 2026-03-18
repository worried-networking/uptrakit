use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{ContainerRuntime, DockerConfig};
use crate::docker_client::{ContainerForImage, DockerClient, LocalContainerInfo, MockDockerClient};
use crate::plugin::DockerPlugin;
use uptrakit_plugin_infrastructure_core::command::{CommandExecutor, CommandSpec};
use uptrakit_plugin_infrastructure_core::mpsc;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, Discoverer, HostCompatibility, PluginCapability, UpdateExecutor as _,
    UpdateOutputLine, VersionDetector,
};

fn test_executor() -> Arc<dyn CommandExecutor> {
    Arc::new(uptrakit_plugin_infrastructure_core::LocalCommandExecutor)
}

/// A mock executor that records commands without executing them.
struct MockCommandExecutor;

#[async_trait]
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

/// A mock executor that simulates runtime detection probes.
///
/// `probe_results` is a list of exit codes returned in order for each
/// call to `execute_quiet`. Index 0 = first call (docker check),
/// index 1 = second call (podman check), etc.
struct DetectionMockExecutor {
    probe_results: Vec<i32>,
    call_count: std::sync::atomic::AtomicUsize,
}

impl DetectionMockExecutor {
    fn new(results: Vec<i32>) -> Self {
        Self {
            probe_results: results,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl CommandExecutor for DetectionMockExecutor {
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
        let idx = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let exit_code = self.probe_results.get(idx).copied().unwrap_or(1);
        Ok(uptrakit_plugin_infrastructure_core::CommandOutput {
            output: String::new(),
            exit_code,
        })
    }
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
fn descriptor_capabilities_includes_discover_local_software() {
    assert!(
        crate::plugin::DESCRIPTOR
            .capabilities
            .contains(&PluginCapability::DiscoverLocalSoftware)
    );
}

#[test]
fn descriptor_capabilities_includes_detect_host_compatibility() {
    assert!(
        crate::plugin::DESCRIPTOR
            .capabilities
            .contains(&PluginCapability::DetectHostCompatibility)
    );
}

#[test]
fn descriptor_capabilities_excludes_refresh_package_index() {
    assert!(
        !crate::plugin::DESCRIPTOR
            .capabilities
            .contains(&PluginCapability::RefreshPackageIndex)
    );
}

// ── detect_host_compatibility ─────────────────────────────────────────────

#[tokio::test]
async fn detect_host_compatibility_compatible_when_daemon_reachable() {
    let mock = Arc::new(MockDockerClient::default());
    // Use DetectionMockExecutor that returns 0 so docker is found during Auto probe.
    let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
    let result = plugin.detect_host_compatibility().await.expect("ok");
    assert_eq!(result, HostCompatibility::Compatible);
}

#[tokio::test]
async fn detect_host_compatibility_incompatible_when_daemon_unreachable() {
    let mock = Arc::new(MockDockerClient {
        ping_should_fail: true,
        ..Default::default()
    });
    // Use DetectionMockExecutor that returns 0 so docker is found during Auto probe.
    let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
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
    // Use DetectionMockExecutor that returns exit 0 for the docker probe so
    // runtime detection succeeds, then the daemon ping hangs and must time out.
    let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
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
    let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
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
    let plugin = DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
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
    let plugin = DockerPlugin::new_for_test(config, mock_executor(), mock).expect("valid config");
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
    let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).expect("valid config");
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
async fn discover_software_emits_one_item_per_container() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["web-server".to_string()],
                labels: Default::default(),
            },
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["api-proxy".to_string()],
                labels: Default::default(),
            },
        ],
        ..Default::default()
    });
    let plugin =
        DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
    let mut discoveries = plugin.discover_software().await.unwrap();
    // Two containers → two software items, even though both use the same image.
    assert_eq!(discoveries.len(), 2);

    // Both items share the same image-level package_identifier and name.
    // The container name is carried in `qualifier` and `plugin_package_identifier`.
    discoveries.sort_by(|a, b| {
        a.qualifier
            .as_deref()
            .unwrap_or("")
            .cmp(b.qualifier.as_deref().unwrap_or(""))
    });
    assert_eq!(discoveries[0].package_identifier, "nginx:latest");
    // Name is the image reference without the tag (tag-agnostic).
    assert_eq!(discoveries[0].name, "nginx");
    assert_eq!(discoveries[0].installed_version, "sha256:abc123");
    assert_eq!(discoveries[0].qualifier.as_deref(), Some("api-proxy"));
    assert_eq!(
        discoveries[0].plugin_package_identifier.as_deref(),
        Some("nginx:latest#api-proxy")
    );

    assert_eq!(discoveries[1].package_identifier, "nginx:latest");
    assert_eq!(discoveries[1].name, "nginx");
    assert_eq!(discoveries[1].installed_version, "sha256:abc123");
    assert_eq!(discoveries[1].qualifier.as_deref(), Some("web-server"));
    assert_eq!(
        discoveries[1].plugin_package_identifier.as_deref(),
        Some("nginx:latest#web-server")
    );
}

#[tokio::test]
async fn discover_software_single_container_uses_image_based_name() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![LocalContainerInfo {
            image: "nginx:latest".to_string(),
            names: vec!["my-nginx".to_string()],
            labels: Default::default(),
        }],
        ..Default::default()
    });
    let plugin =
        DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
    let discoveries = plugin.discover_software().await.unwrap();

    assert_eq!(discoveries.len(), 1);
    // Package identifier is the full image reference (shared across containers).
    assert_eq!(discoveries[0].package_identifier, "nginx:latest");
    // Name is the image without the tag so it stays stable across tag switches.
    assert_eq!(discoveries[0].name, "nginx");
    assert_eq!(discoveries[0].installed_version, "sha256:abc123");
    // Container name is carried in qualifier and plugin_package_identifier.
    assert_eq!(discoveries[0].qualifier.as_deref(), Some("my-nginx"));
    assert_eq!(
        discoveries[0].plugin_package_identifier.as_deref(),
        Some("nginx:latest#my-nginx")
    );
}

#[tokio::test]
async fn discover_software_strips_leading_slash_from_container_name() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![LocalContainerInfo {
            image: "nginx:latest".to_string(),
            // BollardDockerClient strips the leading '/' before returning
            // LocalContainerInfo, but the mock may supply pre-stripped names.
            names: vec!["my-nginx".to_string()],
            labels: Default::default(),
        }],
        ..Default::default()
    });
    let plugin =
        DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
    let discoveries = plugin.discover_software().await.unwrap();
    assert_eq!(discoveries.len(), 1);
    // package_identifier is the image-level identifier; qualifier holds the container name.
    assert_eq!(discoveries[0].package_identifier, "nginx:latest");
    assert_eq!(discoveries[0].qualifier.as_deref(), Some("my-nginx"));
    assert_eq!(
        discoveries[0].plugin_package_identifier.as_deref(),
        Some("nginx:latest#my-nginx")
    );
}

#[tokio::test]
async fn discover_software_skips_sha_images() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![LocalContainerInfo {
            image: "sha256:deadbeef".to_string(),
            names: vec!["bare-sha-container".to_string()],
            labels: Default::default(),
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
            labels: Default::default(),
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
    use uptrakit_plugin_infrastructure_core::{PluginRole, plugin_ids};

    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![LocalContainerInfo {
            image: "nginx:latest".to_string(),
            names: vec!["my-nginx".to_string()],
            labels: Default::default(),
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
    assert_eq!(target.plugin_type, plugin_ids::RELEASES_DOCKER.clone());
    assert_eq!(target.plugin_config_name, "Docker");
    assert_eq!(target.plugin_config, serde_json::json!({}));
    assert!(target.roles.contains(&PluginRole::DetectVersion));
    assert!(target.roles.contains(&PluginRole::FetchReleases));
    assert!(target.roles.contains(&PluginRole::ExecuteUpdate));
}

#[tokio::test]
async fn discover_software_emits_targets_with_custom_config() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![LocalContainerInfo {
            image: "nginx:latest".to_string(),
            names: vec!["my-nginx".to_string()],
            labels: Default::default(),
        }],
        ..Default::default()
    });
    // Custom config still emits targets.
    let config = DockerConfig {
        docker_host: Some("unix:///var/run/docker.sock".to_string()),
        ..Default::default()
    };
    let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
    let discoveries = plugin.discover_software().await.unwrap();

    assert_eq!(discoveries.len(), 1);
    assert_eq!(discoveries[0].targets.len(), 1);
}

// ── execute_update — container-qualified identifiers ──────────────────────

#[tokio::test]
async fn execute_update_with_container_qualifier_only_recreates_named_container() {
    // Two containers share the same image.
    let mock = Arc::new(MockDockerClient {
        containers_for_image: vec![
            ContainerForImage {
                name: "web-server".to_string(),
                is_running: true,
                labels: Default::default(),
            },
            ContainerForImage {
                name: "api-proxy".to_string(),
                is_running: true,
                labels: Default::default(),
            },
        ],
        ..Default::default()
    });
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
        .expect("valid config");
    let (tx, mut rx) = mpsc::channel(100);
    // Container-qualified identifier: only "web-server" should be touched.
    let result = plugin
        .execute_update("nginx:latest#web-server", "sha256:abc", None, &tx)
        .await
        .expect("execute_update should succeed");

    assert!(
        result.contains("Recreating container web-server"),
        "web-server must be recreated: {result}"
    );
    assert!(
        !result.contains("Recreating container api-proxy"),
        "api-proxy must NOT be recreated: {result}"
    );

    rx.close();
    while rx.recv().await.is_some() {}
}

#[tokio::test]
async fn execute_update_without_qualifier_recreates_all_containers() {
    // Unqualified identifier (no `#container_name`) → legacy behaviour.
    let mock = Arc::new(MockDockerClient {
        containers_for_image: vec![
            ContainerForImage {
                name: "web-server".to_string(),
                is_running: true,
                labels: Default::default(),
            },
            ContainerForImage {
                name: "api-proxy".to_string(),
                is_running: true,
                labels: Default::default(),
            },
        ],
        ..Default::default()
    });
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
        .expect("valid config");
    let (tx, mut rx) = mpsc::channel(100);
    let result = plugin
        .execute_update("nginx:latest", "sha256:abc", None, &tx)
        .await
        .expect("should succeed");

    assert!(
        result.contains("Recreating container web-server"),
        "web-server must be recreated: {result}"
    );
    assert!(
        result.contains("Recreating container api-proxy"),
        "api-proxy must be recreated: {result}"
    );

    rx.close();
    while rx.recv().await.is_some() {}
}

#[tokio::test]
async fn execute_update_container_not_found_succeeds_silently() {
    // The named container is not in the list returned by list_containers_for_image.
    let mock = Arc::new(MockDockerClient {
        containers_for_image: vec![ContainerForImage {
            name: "other-container".to_string(),
            is_running: true,
            labels: Default::default(),
        }],
        ..Default::default()
    });
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
        .expect("valid config");
    let (tx, mut rx) = mpsc::channel(100);
    let result = plugin
        .execute_update("nginx:latest#missing-container", "sha256:abc", None, &tx)
        .await
        .expect("should succeed even when container not found");

    // Pull happened but no containers were recreated.
    assert!(
        result.contains("Pulling Docker image nginx:latest"),
        "should pull the image: {result}"
    );
    assert!(
        !result.contains("Recreating"),
        "no containers should be recreated: {result}"
    );

    rx.close();
    while rx.recv().await.is_some() {}
}

// ── batch_detect_installed_version ────────────────────────────────────────

#[tokio::test]
async fn batch_detect_deduplicates_inspections_for_shared_image() {
    use std::sync::Arc as StdArc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // Custom mock that counts inspect_image calls.
    struct CountingMockClient {
        inspect_count: StdArc<AtomicUsize>,
        digest: String,
    }

    #[async_trait]
    impl DockerClient for CountingMockClient {
        #[cfg(feature = "daemon")]
        async fn ping(&self) -> crate::error::Result<()> {
            Ok(())
        }

        async fn pull_image(
            &self,
            _image: &str,
            _tag: &str,
            _auth: Option<&crate::config::DockerAuth>,
            _output_tx: &mpsc::Sender<UpdateOutputLine>,
        ) -> crate::error::Result<String> {
            Ok(String::new())
        }

        async fn inspect_image(
            &self,
            _full_ref: &str,
        ) -> crate::error::Result<Option<crate::docker_client::LocalImageDigest>> {
            self.inspect_count.fetch_add(1, Ordering::SeqCst);
            Ok(Some(crate::docker_client::LocalImageDigest {
                digest: self.digest.clone(),
                os: None,
                architecture: None,
                variant: None,
            }))
        }

        async fn list_containers(
            &self,
            _all: bool,
        ) -> crate::error::Result<Vec<crate::docker_client::LocalContainerInfo>> {
            Ok(vec![])
        }

        async fn list_containers_for_image(
            &self,
            _full_ref: &str,
        ) -> crate::error::Result<Vec<crate::docker_client::ContainerForImage>> {
            Ok(vec![])
        }

        async fn recreate_container(
            &self,
            _name: &str,
            _was_running: bool,
        ) -> crate::error::Result<()> {
            Ok(())
        }
    }

    let inspect_count = StdArc::new(AtomicUsize::new(0));
    let mock = Arc::new(CountingMockClient {
        inspect_count: StdArc::clone(&inspect_count),
        digest: "sha256:abc".to_string(),
    });
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
        .expect("valid config");

    // Three items all using the same image via container-qualified identifiers.
    let items = vec![
        BatchDetectItem::new("nginx:latest#web-server".to_string()),
        BatchDetectItem::new("nginx:latest#api-proxy".to_string()),
        BatchDetectItem::new("nginx:latest#worker".to_string()),
    ];

    let results = plugin
        .batch_detect(&items)
        .await
        .expect("batch detect should succeed");

    // All three get the digest.
    assert_eq!(results.len(), 3);
    for r in &results {
        assert_eq!(
            r.installed_version
                .as_ref()
                .map(|v| v.to_string())
                .as_deref(),
            Some("sha256:abc")
        );
        assert!(r.error.is_none());
    }

    // Exactly one inspect call despite three items (deduplication).
    assert_eq!(
        inspect_count.load(Ordering::SeqCst),
        1,
        "image should be inspected only once regardless of how many containers use it"
    );
}

#[tokio::test]
async fn batch_detect_returns_none_for_uninstalled_image() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: None,
        ..Default::default()
    });
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
        .expect("valid config");

    let items = vec![BatchDetectItem::new("nginx:latest#web-server".to_string())];
    let results = plugin.batch_detect(&items).await.expect("ok");
    assert_eq!(results.len(), 1);
    assert!(results[0].installed_version.is_none());
    assert!(results[0].error.is_none());
}

#[tokio::test]
async fn batch_detect_handles_unqualified_identifiers() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:def456".to_string()),
        ..Default::default()
    });
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock)
        .expect("valid config");

    let items = vec![BatchDetectItem::new("nginx".to_string())];
    let results = plugin.batch_detect(&items).await.expect("ok");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]
            .installed_version
            .as_ref()
            .map(|v| v.to_string())
            .as_deref(),
        Some("sha256:def456")
    );
}

// ── Label filter ─────────────────────────────────────────────────────────

#[tokio::test]
async fn discover_software_include_label_filter_skips_non_matching() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["managed-nginx".to_string()],
                labels: [("com.example.managed".to_string(), "true".to_string())]
                    .into_iter()
                    .collect(),
            },
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["unmanaged-nginx".to_string()],
                labels: Default::default(),
            },
        ],
        ..Default::default()
    });
    let mut config = DockerConfig::default();
    config
        .include_labels
        .insert("com.example.managed".to_string(), "true".to_string());
    let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
    let discoveries = plugin.discover_software().await.unwrap();
    assert_eq!(discoveries.len(), 1);
    assert_eq!(discoveries[0].qualifier.as_deref(), Some("managed-nginx"));
}

#[tokio::test]
async fn discover_software_exclude_label_filter_skips_excluded() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["prod-nginx".to_string()],
                labels: Default::default(),
            },
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["dev-nginx".to_string()],
                labels: [("env".to_string(), "dev".to_string())]
                    .into_iter()
                    .collect(),
            },
        ],
        ..Default::default()
    });
    let mut config = DockerConfig::default();
    config
        .exclude_labels
        .insert("env".to_string(), "dev".to_string());
    let plugin = DockerPlugin::new_for_test(config, test_executor(), mock).unwrap();
    let discoveries = plugin.discover_software().await.unwrap();
    assert_eq!(discoveries.len(), 1);
    assert_eq!(discoveries[0].qualifier.as_deref(), Some("prod-nginx"));
}

#[tokio::test]
async fn discover_software_no_label_filter_includes_all() {
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some("sha256:abc123".to_string()),
        containers: vec![
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["a".to_string()],
                labels: Default::default(),
            },
            LocalContainerInfo {
                image: "nginx:latest".to_string(),
                names: vec!["b".to_string()],
                labels: [("x".to_string(), "y".to_string())].into_iter().collect(),
            },
        ],
        ..Default::default()
    });
    let plugin =
        DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();
    let discoveries = plugin.discover_software().await.unwrap();
    assert_eq!(discoveries.len(), 2);
}

// ── ContainerRuntime detection ────────────────────────────────────────────

#[tokio::test]
async fn detect_host_compat_auto_selects_docker_when_available() {
    // probe_results[0] = docker check returns 0 (found)
    let executor = Arc::new(DetectionMockExecutor::new(vec![0]));
    let mock = Arc::new(MockDockerClient::default());
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
    let result = plugin.detect_host_compatibility().await.expect("ok");
    assert_eq!(result, HostCompatibility::Compatible);
    assert_eq!(
        *plugin.detected_runtime.lock(),
        Some(ContainerRuntime::Docker)
    );
    assert_eq!(
        plugin.effective_dial_stdio_command(),
        "docker system dial-stdio"
    );
}

#[tokio::test]
async fn detect_host_compat_auto_selects_podman_when_only_podman_found() {
    // probe_results[0] = docker returns 1 (not found), [1] = podman returns 0
    let executor = Arc::new(DetectionMockExecutor::new(vec![1, 0]));
    let mock = Arc::new(MockDockerClient::default());
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
    let result = plugin.detect_host_compatibility().await.expect("ok");
    assert_eq!(result, HostCompatibility::Compatible);
    assert_eq!(
        *plugin.detected_runtime.lock(),
        Some(ContainerRuntime::Podman)
    );
    assert_eq!(
        plugin.effective_dial_stdio_command(),
        "podman system dial-stdio"
    );
}

#[tokio::test]
async fn detect_host_compat_auto_incompatible_when_neither_found() {
    // Both docker and podman checks fail
    let executor = Arc::new(DetectionMockExecutor::new(vec![1, 1]));
    let mock = Arc::new(MockDockerClient::default());
    let plugin = DockerPlugin::new_for_test(DockerConfig::default(), executor, mock).unwrap();
    let result = plugin.detect_host_compatibility().await.expect("ok");
    match result {
        HostCompatibility::Incompatible(msg) => {
            assert!(
                msg.contains("container runtime"),
                "message should mention container runtime: {msg}"
            );
        }
        HostCompatibility::Compatible => panic!("expected Incompatible"),
        _ => panic!("unexpected variant"),
    }
}

#[test]
fn effective_dial_stdio_command_docker_explicit() {
    let config = DockerConfig {
        container_runtime: ContainerRuntime::Docker,
        ..Default::default()
    };
    let plugin =
        DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).unwrap();
    assert_eq!(
        plugin.effective_dial_stdio_command(),
        "docker system dial-stdio"
    );
}

#[test]
fn effective_dial_stdio_command_podman_explicit() {
    let config = DockerConfig {
        container_runtime: ContainerRuntime::Podman,
        ..Default::default()
    };
    let plugin =
        DockerPlugin::new_for_test(config, test_executor(), default_mock_client()).unwrap();
    assert_eq!(
        plugin.effective_dial_stdio_command(),
        "podman system dial-stdio"
    );
}

#[test]
fn effective_dial_stdio_command_auto_defaults_to_docker() {
    let plugin = DockerPlugin::new_for_test(
        DockerConfig::default(),
        test_executor(),
        default_mock_client(),
    )
    .unwrap();
    // No detection run yet: defaults to docker
    assert_eq!(
        plugin.effective_dial_stdio_command(),
        "docker system dial-stdio"
    );
}

// ── detect_installed_version — digest consistency with fetch_releases ─────
//
// These tests guard against the regression where platform metadata available
// in the local Docker inspect result caused detect_installed_version to call
// get_platform_manifest_digest and return a platform-specific manifest digest,
// while fetch_releases (without an explicit platform) returned the image-index
// digest. The two digests can never be equal, producing a permanent spurious
// "update available".
//
// The fix: do NOT auto-detect the platform from the local image's os/arch
// fields. Only use config.platform when it is explicitly set.

/// When no platform is configured and the local image has os/arch metadata,
/// detect_installed_version must return the local inspect digest (image-index
/// digest from RepoDigests) — not a platform-specific manifest digest fetched
/// from the registry. This keeps installed_version in the same digest namespace
/// as latest_version produced by fetch_releases (which also uses the image-index
/// digest when no platform is configured).
#[tokio::test]
async fn detect_installed_version_returns_local_digest_not_platform_digest_when_no_platform_config()
{
    let local_digest =
        "sha256:7c1b20687bd3016e61b4a67f6b232c10881bc979ac8ed12cbda8e0b99fe4b5ab".to_string();
    // Simulate a multi-arch image whose inspect result carries os/arch metadata.
    // Previously this triggered the auto-detection path which called
    // get_platform_manifest_digest and returned a *different* digest.
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some(local_digest.clone()),
        inspect_os: Some("linux".to_string()),
        inspect_architecture: Some("amd64".to_string()),
        ..Default::default()
    });
    // No platform in config — mirrors the case where the user manually added
    // a Docker item without going through autodiscovery (which stores the
    // detected platform in the config).
    let plugin =
        DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();

    let result = plugin
        .detect_installed_version("traefik:v2.11")
        .await
        .unwrap();

    // Must return the digest from docker inspect (image-index digest from
    // RepoDigests), NOT a platform-specific manifest digest from the registry.
    assert_eq!(
        result.map(|v| v.to_string()).as_deref(),
        Some(&*local_digest)
    );
}

// ── detect_installed_version — transient platform registry failure ────────────

/// When `platform` is configured and `get_platform_manifest_digest` fails
/// transiently, `detect_installed_version` must return `Err` rather than
/// falling back to the image-index digest from the local Docker daemon.
///
/// **Regression guard:** the old fallback caused a permanent digest-namespace
/// mismatch — `installed_version` held the index digest while `fetch_releases`
/// always returned the platform-specific digest.  The two can never be equal,
/// so the item appeared perpetually updatable even though nothing had changed.
#[tokio::test]
async fn detect_installed_version_errors_on_transient_platform_registry_failure() {
    use crate::config::DockerConfig;
    use crate::registry::MockRegistryClient;

    let local_digest =
        "sha256:7fbf01d7aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let mock_docker = Arc::new(MockDockerClient {
        inspect_result: Some(local_digest.clone()),
        ..Default::default()
    });
    let mock_registry = Arc::new(MockRegistryClient {
        platform_digest_should_fail: true,
        ..Default::default()
    });
    let config = DockerConfig {
        platform: Some("linux/amd64".to_string()),
        ..Default::default()
    };
    let plugin = DockerPlugin::new_for_test_with_registry(
        config,
        mock_executor(),
        mock_docker,
        mock_registry,
    )
    .unwrap();

    let result = plugin
        .detect_installed_version("adguard/adguardhome:latest")
        .await;

    // Must propagate the error — falling back to the index digest would create
    // a permanent mismatch with the platform-specific digest from fetch_releases.
    assert!(
        result.is_err(),
        "expected Err when platform registry call fails transiently, got Ok({result:?})"
    );
}

/// Same regression guard for the batch path.
#[tokio::test]
async fn batch_detect_errors_on_transient_platform_registry_failure() {
    use crate::config::DockerConfig;
    use crate::registry::MockRegistryClient;

    let local_digest =
        "sha256:7fbf01d7aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let mock_docker = Arc::new(MockDockerClient {
        inspect_result: Some(local_digest.clone()),
        ..Default::default()
    });
    let mock_registry = Arc::new(MockRegistryClient {
        platform_digest_should_fail: true,
        ..Default::default()
    });
    let config = DockerConfig {
        platform: Some("linux/amd64".to_string()),
        ..Default::default()
    };
    let plugin = DockerPlugin::new_for_test_with_registry(
        config,
        mock_executor(),
        mock_docker,
        mock_registry,
    )
    .unwrap();

    let items = vec![BatchDetectItem::new(
        "adguard/adguardhome:latest".to_string(),
    )];
    let results = plugin.batch_detect(&items).await.unwrap();

    assert_eq!(results.len(), 1);
    // Must record an error for the item — not return the index digest as
    // installed_version, which would create a permanent spurious update signal.
    assert!(
        results[0].error.is_some(),
        "expected batch result to carry an error when platform registry call fails"
    );
    assert!(results[0].installed_version.is_none());
}

/// Same regression guard for the batch path.
#[tokio::test]
async fn batch_detect_returns_local_digest_not_platform_digest_when_no_platform_config() {
    let local_digest =
        "sha256:7c1b20687bd3016e61b4a67f6b232c10881bc979ac8ed12cbda8e0b99fe4b5ab".to_string();
    let mock = Arc::new(MockDockerClient {
        inspect_result: Some(local_digest.clone()),
        inspect_os: Some("linux".to_string()),
        inspect_architecture: Some("amd64".to_string()),
        ..Default::default()
    });
    let plugin =
        DockerPlugin::new_for_test(DockerConfig::default(), test_executor(), mock).unwrap();

    let items = vec![BatchDetectItem::new("traefik:v2.11".to_string())];
    let results = plugin.batch_detect(&items).await.unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]
            .installed_version
            .as_ref()
            .map(|v| v.to_string())
            .as_deref(),
        Some(&*local_digest)
    );
    assert!(results[0].error.is_none());
}

// ── discover_software — platform-digest namespace fix ─────────────────────
//
// When auto-discovery inspects a multi-arch image and detects a platform from
// the os/arch fields, it must store the *platform-specific* manifest digest as
// `installed_version` (not the image-index digest from RepoDigests).  This
// keeps `installed_version` in the same digest namespace that `fetch_releases`
// and `detect_version` use after they read `config_override = {"platform": "…"}`,
// preventing the perpetual false "update available".

/// When a platform is auto-detected during discovery and the registry call
/// succeeds, `installed_version` must be the platform-specific manifest digest
/// (not the image-index digest from `inspect_result`).
///
/// Regression: previously the image-index digest was always used, which
/// never matched the platform digest returned by `fetch_releases`, causing a
/// permanent spurious update signal.
#[tokio::test]
async fn discover_software_uses_platform_manifest_digest_as_installed_version() {
    use crate::config::DockerConfig;
    use crate::registry::{ManifestInfo, MockRegistryClient};

    let index_digest =
        "sha256:6dd50763aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
    let platform_digest =
        "sha256:f9086bfdbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();

    let mock_docker = Arc::new(MockDockerClient {
        inspect_result: Some(index_digest.clone()),
        inspect_os: Some("linux".to_string()),
        inspect_architecture: Some("amd64".to_string()),
        containers: vec![LocalContainerInfo {
            image: "containrrr/watchtower:latest".to_string(),
            names: vec!["watchtower".to_string()],
            labels: Default::default(),
        }],
        ..Default::default()
    });
    let mock_registry = Arc::new(MockRegistryClient {
        platform_digest_result: Some(ManifestInfo {
            digest: platform_digest.clone(),
            created_at: None,
        }),
        ..Default::default()
    });

    let plugin = DockerPlugin::new_for_test_with_registry(
        DockerConfig::default(),
        mock_executor(),
        mock_docker,
        mock_registry,
    )
    .unwrap();

    let discoveries = plugin.discover_software().await.unwrap();

    assert_eq!(discoveries.len(), 1);
    // Must use the platform-specific digest, not the image-index digest.
    assert_eq!(
        discoveries[0].installed_version, platform_digest,
        "installed_version should be the platform digest, not the image-index digest"
    );
    assert_ne!(
        discoveries[0].installed_version, index_digest,
        "installed_version must not be the image-index digest when platform is detected"
    );
    // config_override must carry the detected platform so subsequent
    // fetch_releases / detect_version calls stay in the same digest namespace.
    let target = &discoveries[0].targets[0];
    assert_eq!(
        target.config_override,
        Some(serde_json::json!({"platform": "linux/amd64"}))
    );
}

/// When auto-discovery detects a platform but the registry call fails
/// (transient error or platform not found), `discover_software` must not
/// crash — it falls back to the image-index digest for `installed_version`.
/// `detect_version` will correct the value on its next scheduled run.
#[tokio::test]
async fn discover_software_falls_back_to_index_digest_when_platform_registry_fails() {
    use crate::config::DockerConfig;
    use crate::registry::MockRegistryClient;

    let index_digest =
        "sha256:6dd50763cccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".to_string();

    let mock_docker = Arc::new(MockDockerClient {
        inspect_result: Some(index_digest.clone()),
        inspect_os: Some("linux".to_string()),
        inspect_architecture: Some("amd64".to_string()),
        containers: vec![LocalContainerInfo {
            image: "containrrr/watchtower:latest".to_string(),
            names: vec!["watchtower".to_string()],
            labels: Default::default(),
        }],
        ..Default::default()
    });
    let mock_registry = Arc::new(MockRegistryClient {
        platform_digest_should_fail: true,
        ..Default::default()
    });

    let plugin = DockerPlugin::new_for_test_with_registry(
        DockerConfig::default(),
        mock_executor(),
        mock_docker,
        mock_registry,
    )
    .unwrap();

    let discoveries = plugin.discover_software().await.unwrap();

    // Discovery must succeed (graceful degradation — no crash).
    assert_eq!(discoveries.len(), 1);
    // Falls back to the image-index digest.
    assert_eq!(
        discoveries[0].installed_version, index_digest,
        "should fall back to image-index digest when platform registry call fails"
    );
    // Platform is still detected, so config_override is set — detect_version
    // will use it to resolve the correct digest on the next run.
    let target = &discoveries[0].targets[0];
    assert_eq!(
        target.config_override,
        Some(serde_json::json!({"platform": "linux/amd64"}))
    );
}
