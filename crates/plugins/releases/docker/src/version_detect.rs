use std::sync::Arc;

use crate::image_ref::ImageRef;
use crate::plugin::DockerPlugin;
use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Version,
};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetectorPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
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

        let client = Arc::clone(&*self.docker_client.lock());
        match client.inspect_image(&full_ref).await {
            Ok(Some(digest_info)) => {
                tracing::debug!(
                    digest = %digest_info.digest,
                    image = %ir.image,
                    "detected installed digest"
                );

                // When a platform is known (from config override or auto-detected from
                // the local image), compare platform-specific digests to avoid false
                // positives when only a different platform is updated.
                let platform = self.config.platform.clone().or_else(|| {
                    crate::config::form_platform_string(
                        digest_info.os.as_deref(),
                        digest_info.architecture.as_deref(),
                        digest_info.variant.as_deref(),
                    )
                });

                if let Some(ref p) = platform {
                    match self
                        .registry_client
                        .get_platform_manifest_digest(&ir.registry, &ir.repository, tag, p)
                        .await
                    {
                        Ok(Some(platform_digest)) => {
                            tracing::debug!(
                                platform = %p,
                                digest = %platform_digest,
                                image = %ir.image,
                                "resolved platform-specific digest"
                            );
                            return Ok(Some(Version::new(&platform_digest)));
                        }
                        Ok(None) => {
                            // Platform removed from the manifest list.
                            return Err(
                                uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(
                                    crate::error::DockerError::PlatformNotAvailable {
                                        platform: p.clone(),
                                        image: ir.image.clone(),
                                        tag: tag.to_string(),
                                    }
                                    .to_string(),
                                )
                                .into(),
                            );
                        }
                        Err(e) => {
                            // Transient registry failure — fall back to the image index digest
                            // so that a network hiccup does not block version detection.
                            tracing::warn!(
                                error = %e,
                                image = %ir.image,
                                "failed to fetch platform manifest digest; \
                                 falling back to image index digest"
                            );
                        }
                    }
                }

                Ok(Some(Version::new(&digest_info.digest)))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(
                uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(e.to_string())
                    .into(),
            ),
        }
    }

    /// Batch installed-version detection with image-level deduplication.
    ///
    /// Multiple containers that share the same image (after `tracked_tag`
    /// resolution) are inspected only once, avoiding redundant Docker daemon
    /// calls for the common case where several items use e.g. `nginx:latest`.
    #[tracing::instrument(skip_all)]
    async fn batch_detect_installed_version(
        &self,
        items: &[BatchDetectItem],
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<BatchDetectResult>> {
        use std::collections::HashMap;

        // Daemon inspect cache: "image:tag" → Result<Option<LocalImageDigest>, String>.
        let mut inspect_cache: HashMap<
            String,
            std::result::Result<Option<crate::docker_client::LocalImageDigest>, String>,
        > = HashMap::new();

        // Platform digest cache: "image:tag::platform" → Option<String>.
        let mut platform_cache: HashMap<String, Option<String>> = HashMap::new();

        // Populate inspect cache.
        for item in items {
            let ir: ImageRef = match item.package_identifier.parse::<ImageRef>() {
                Ok(r) => r,
                Err(_) => continue,
            };
            let tag = self.config.resolved_tracked_tag(&ir.tag);
            let resolved = format!("{}:{tag}", ir.image);

            if inspect_cache.contains_key(&resolved) {
                continue;
            }

            let client = Arc::clone(&*self.docker_client.lock());
            let outcome = match client.inspect_image(&resolved).await {
                Ok(Some(d)) => Ok(Some(d)),
                Ok(None) => Ok(None),
                Err(e) => Err(e.to_string()),
            };
            inspect_cache.insert(resolved, outcome);
        }

        let mut results = Vec::with_capacity(items.len());
        for item in items {
            let ir: ImageRef = match item.package_identifier.parse::<ImageRef>() {
                Ok(r) => r,
                Err(e) => {
                    results.push(BatchDetectResult::error(
                        item.package_identifier.clone(),
                        e.to_string(),
                    ));
                    continue;
                }
            };
            let tag = self.config.resolved_tracked_tag(&ir.tag);
            let resolved = format!("{}:{tag}", ir.image);

            match inspect_cache.get(&resolved) {
                Some(Ok(Some(digest_info))) => {
                    let platform = self.config.platform.clone().or_else(|| {
                        crate::config::form_platform_string(
                            digest_info.os.as_deref(),
                            digest_info.architecture.as_deref(),
                            digest_info.variant.as_deref(),
                        )
                    });

                    if let Some(ref p) = platform {
                        let cache_key = format!("{resolved}::{p}");
                        let platform_digest = match platform_cache.entry(cache_key) {
                            std::collections::hash_map::Entry::Occupied(e) => e.get().clone(),
                            std::collections::hash_map::Entry::Vacant(e) => {
                                let result = self
                                    .registry_client
                                    .get_platform_manifest_digest(
                                        &ir.registry,
                                        &ir.repository,
                                        tag,
                                        p,
                                    )
                                    .await
                                    .ok()
                                    .flatten();
                                e.insert(result.clone());
                                result
                            }
                        };

                        match platform_digest {
                            Some(pd) => {
                                results.push(BatchDetectResult::found(
                                    item.package_identifier.clone(),
                                    Version::new(&pd),
                                ));
                                continue;
                            }
                            None => {
                                // Platform not in manifest list — treat as not found.
                                results.push(BatchDetectResult::not_found(
                                    item.package_identifier.clone(),
                                ));
                                continue;
                            }
                        }
                    }

                    results.push(BatchDetectResult::found(
                        item.package_identifier.clone(),
                        Version::new(&digest_info.digest),
                    ));
                }
                Some(Ok(None)) | None => {
                    results.push(BatchDetectResult::not_found(
                        item.package_identifier.clone(),
                    ));
                }
                Some(Err(e)) => {
                    results.push(BatchDetectResult::error(
                        item.package_identifier.clone(),
                        e.clone(),
                    ));
                }
            }
        }

        Ok(results)
    }
}
