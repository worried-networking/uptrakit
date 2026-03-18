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
impl uptrakit_plugin_infrastructure_core::Discoverer for DockerPlugin {
    #[tracing::instrument(skip_all)]
    async fn discover_software(
        &self,
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<DiscoveredSoftware>> {
        use std::collections::HashMap;
        use uptrakit_plugin_infrastructure_core::{DiscoveryTarget, PluginRole, plugin_ids};

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

        // Cache for (resolved_installed_version, display_version) per unique image ref.
        //
        // When a platform is auto-detected from the local image's os/arch fields,
        // `resolved_installed_version` is the platform-specific manifest digest returned
        // by `get_platform_manifest_digest`.  This keeps `installed_version` in the same
        // digest namespace as what `fetch_releases` and `detect_version` produce when
        // `config_override = {"platform": "…"}` is set, preventing the perpetual
        // false "update available" caused by comparing an image-index digest against a
        // platform-specific digest.
        //
        // Falls back to the image-index digest from `RepoDigests` if the registry call
        // fails; `detect_version` will correct it on the next scheduled run.
        let mut per_image_cache: HashMap<String, (String, Option<String>)> = HashMap::new();

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

            // Software item name: image reference without the tag so the name
            // remains stable across tag switches (e.g. "ghcr.io/xtls/xray-core"
            // instead of "ghcr.io/xtls/xray-core:25.8.3").
            let name = ir.image.clone();

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

            // Fetch from the registry (fault-tolerant: errors fall back to the
            // image-index digest).  The cache key is `ir.full_ref`; all containers
            // sharing the same image ref also share the same platform and therefore
            // the same resolved installed_version and display_version.
            let (resolved_installed_version, display_version) = match per_image_cache
                .entry(ir.full_ref.clone())
            {
                std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let (resolved_digest, dv) = if let Some(ref p) = platform {
                        match self
                            .registry_client
                            .get_platform_manifest_digest(&ir.registry, &ir.repository, &ir.tag, p)
                            .await
                        {
                            Ok(Some(info)) => (
                                info.digest.clone(),
                                info.created_at.map(crate::registry::format_display_version),
                            ),
                            Ok(None) | Err(_) => {
                                tracing::warn!(
                                    image = %ir.full_ref,
                                    platform = %p,
                                    "platform manifest lookup failed during discovery; \
                                     falling back to image-index digest (will self-correct)"
                                );
                                (digest_info.digest.clone(), None)
                            }
                        }
                    } else {
                        match self
                            .registry_client
                            .get_manifest_info(&ir.registry, &ir.repository, &ir.tag)
                            .await
                        {
                            Ok(info) => (
                                digest_info.digest.clone(),
                                info.created_at.map(crate::registry::format_display_version),
                            ),
                            Err(_) => (digest_info.digest.clone(), None),
                        }
                    };
                    tracing::debug!(
                        image = %ir.full_ref,
                        installed_version = %resolved_digest,
                        display_version = ?dv,
                        "resolved versions during discovery"
                    );
                    e.insert((resolved_digest.clone(), dv.clone()));
                    (resolved_digest, dv)
                }
            };

            let targets = vec![DiscoveryTarget {
                plugin_type: plugin_ids::RELEASES_DOCKER.clone(),
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
                installed_version: resolved_installed_version,
                targets,
                extra: Some(json!({ "container": container_name })),
                qualifier: Some(container_name.clone()),
                plugin_package_identifier: Some(plugin_pkg_id),
                featured: true,
                installed_display_version: display_version,
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
