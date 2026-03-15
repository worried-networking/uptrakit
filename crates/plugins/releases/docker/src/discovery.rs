use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use crate::image_ref::ImageRef;
use crate::plugin::DockerPlugin;
use uptrakit_plugin_infrastructure_core::DiscoveredSoftware;

#[cfg(feature = "daemon")]
use crate::config::ContainerRuntime;
#[cfg(feature = "daemon")]
use std::time::Duration;
#[cfg(feature = "daemon")]
use uptrakit_plugin_infrastructure_core::HostCompatibility;

#[async_trait]
impl uptrakit_plugin_infrastructure_core::DiscoveryPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        use std::collections::HashMap;
        use uptrakit_plugin_infrastructure_core::{DiscoveryTarget, PluginRole, PluginType};

        let client = Arc::clone(&*self.docker_client.lock());
        let containers = client.list_containers(true).await.map_err(|e| {
            uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
        })?;

        // Always emit a DiscoveryTarget so the controller can find-or-create
        // a "Docker" plugin config and the role assignments.

        // Inspect each unique image ref only once.  Multiple containers using
        // the same image share the same digest, so there is no point calling
        // the Docker daemon more than once per image.
        let mut digest_cache: HashMap<String, Option<crate::docker_client::LocalImageDigest>> =
            HashMap::new();

        let mut discoveries = Vec::new();

        for container in containers {
            let raw_image = container.image.trim();
            if raw_image.is_empty() {
                continue;
            }

            // Skip bare SHA refs — they have no registry provenance.
            if raw_image.starts_with("sha256:") {
                continue;
            }

            // Parse the image ref (may or may not have a tag).
            let ir: ImageRef = match raw_image.parse() {
                Ok(r) => r,
                Err(_) => continue,
            };

            // Take the first container name, stripping any leading '/'.
            let container_name = match container.names.first() {
                Some(n) => n.trim_start_matches('/').to_string(),
                None => continue,
            };
            if container_name.is_empty() {
                continue;
            }

            // Apply label filter when labels are populated (may be empty from list_containers).
            if !self.container_passes_label_filter(&container.labels) {
                continue;
            }

            // Inspect the image once, then reuse the cached result for every
            // subsequent container that references the same image.
            let digest_info = match digest_cache.entry(ir.full_ref.clone()) {
                std::collections::hash_map::Entry::Occupied(e) => match e.get().clone() {
                    Some(d) => d,
                    // Already determined: no registry digest (locally built). Skip.
                    None => continue,
                },
                std::collections::hash_map::Entry::Vacant(e) => {
                    let client = Arc::clone(&*self.docker_client.lock());
                    let outcome = match client.inspect_image(&ir.full_ref).await {
                        Ok(Some(d)) => {
                            tracing::debug!(
                                image = %ir.full_ref,
                                digest = %d.digest,
                                "inspected image for discovery"
                            );
                            Some(d)
                        }
                        Ok(None) => {
                            tracing::debug!(
                                image = %ir.full_ref,
                                "skipping locally built image (no RepoDigests)"
                            );
                            None
                        }
                        Err(err) => {
                            tracing::warn!(
                                image = %ir.full_ref,
                                error = %err,
                                "failed to inspect image during discovery"
                            );
                            None
                        }
                    };
                    let digest_opt = outcome.clone();
                    e.insert(outcome);
                    match digest_opt {
                        Some(d) => d,
                        None => continue,
                    }
                }
            };

            // Image-level package identifier: shared by all containers using the same image.
            let pkg_id = ir.full_ref.clone();

            // Software item name: just the image reference.
            let name = ir.full_ref.clone();

            // Container-qualified identifier used for per-container plugin operations.
            // Stored in host_software_item_plugin.package_identifier so execute_update
            // can target the specific container.
            let plugin_pkg_id = format!("{}#{}", ir.full_ref, container_name);

            // Compute platform from the installed image's inspect data.
            let platform = crate::config::form_platform_string(
                digest_info.os.as_deref(),
                digest_info.architecture.as_deref(),
                digest_info.variant.as_deref(),
            );
            let config_override = platform.as_deref().map(|p| json!({"platform": p}));

            let targets = vec![DiscoveryTarget {
                plugin_type: PluginType::ReleasesDocker,
                plugin_config: json!({}),
                plugin_config_name: "Docker".to_string(),
                roles: vec![
                    PluginRole::DetectVersion,
                    PluginRole::FetchReleases,
                    PluginRole::ExecuteUpdate,
                ],
                package_identifier: None,
                config_override,
                execution_site: None,
            }];

            discoveries.push(DiscoveredSoftware {
                package_identifier: pkg_id,
                name,
                installed_version: digest_info.digest,
                targets,
                extra: Some(json!({ "container": container_name })),
                qualifier: Some(container_name.clone()),
                plugin_package_identifier: Some(plugin_pkg_id),
                featured: true,
            });
        }

        tracing::debug!(count = discoveries.len(), "docker autodiscovery completed");
        Ok(discoveries)
    }

    #[cfg(feature = "daemon")]
    #[tracing::instrument(skip_all)]
    async fn detect_host_compatibility(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<
        uptrakit_plugin_infrastructure_core::HostCompatibility,
    > {
        // When container_runtime is Auto, probe the executor to discover which
        // runtime (Docker or Podman) is available. For SSH executors that support
        // stdio tunnels we also restart the proxy with the correct command so
        // all subsequent daemon operations use the right runtime.
        if self.config.container_runtime == ContainerRuntime::Auto {
            match self.detect_and_apply_runtime().await {
                Ok(Some(rt)) => {
                    *self.detected_runtime.lock() = Some(rt);
                }
                Ok(None) => {
                    return Ok(HostCompatibility::Incompatible(
                        "no container runtime (Docker or Podman) found on this host".to_string(),
                    ));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "runtime detection failed; proceeding with current client");
                }
            }
        }

        const COMPAT_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
        let client = Arc::clone(&*self.docker_client.lock());
        match tokio::time::timeout(COMPAT_PROBE_TIMEOUT, client.ping()).await {
            Ok(Ok(())) => Ok(HostCompatibility::Compatible),
            Ok(Err(e)) => Ok(HostCompatibility::Incompatible(format!(
                "Docker daemon not accessible: {e}"
            ))),
            Err(_) => Ok(HostCompatibility::Incompatible(
                "Docker daemon ping timed out".to_string(),
            )),
        }
    }
}
