use std::sync::Arc;

use crate::image_ref::ImageRef;
use crate::plugin::DockerPlugin;
use async_trait::async_trait;
use uptrakit_plugin_infrastructure_core::{
    BatchDetectItem, BatchDetectResult, PluginError, Version,
};

/// The locally installed version and display version for a Docker image.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedImageVersion {
    /// Canonical version: platform manifest digest (when platform configured) or image-index digest.
    pub version: String,
    /// Human-readable display version from the manifest's `created_at`, or `None`.
    pub display_version: Option<String>,
}

impl DockerPlugin {
    /// Inspect the locally installed image and optionally resolve the
    /// platform-specific manifest from the registry.
    ///
    /// `platform` — when `Some`, fetches the platform manifest digest and
    /// `created_at` to produce a human-readable display version. When `None`,
    /// the local image's `os`/`architecture` metadata is used to make a
    /// best-effort registry call for `created_at` **for display purposes only**.
    /// The canonical `version` always remains the image-index digest in this
    /// case so the update comparison is unaffected.
    ///
    /// Return values:
    /// - `Ok(None)` — image not present locally.
    /// - `Ok(Some(resolved))` — installed; `display_version` is `None` when
    ///   the local image has no `os`/`arch` metadata or the registry call fails.
    /// - `Err(PlatformNotAvailable)` — `platform` was `Some` but absent from
    ///   the manifest list (definitive; caller must surface this as an error).
    /// - `Err(PluginInternal)` — Docker daemon error **or** transient registry
    ///   failure when `platform` is `Some`. The caller should treat this as
    ///   retryable: the last known `installed_version` stays in the DB until
    ///   the registry is reachable again, preventing a false-positive mismatch
    ///   with the platform-specific digest returned by `fetch_releases`.
    pub(crate) async fn resolve_image_info(
        &self,
        ir: &crate::image_ref::ImageRef,
        tag: &str,
        docker_client: std::sync::Arc<dyn crate::docker_client::DockerClient>,
        platform: Option<&str>,
    ) -> uptrakit_plugin_infrastructure_core::Result<Option<ResolvedImageVersion>> {
        use uptrakit_plugin_infrastructure_core::PluginError;

        let full_ref = format!("{}:{tag}", ir.image);
        let Some(digest_info) = docker_client
            .inspect_image(&full_ref)
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))?
        else {
            return Ok(None);
        };

        tracing::debug!(
            digest = %digest_info.digest,
            image = %ir.image,
            "detected installed digest"
        );

        if let Some(p) = platform {
            match self
                .registry_client
                .get_platform_manifest_digest(&ir.registry, &ir.repository, tag, p)
                .await
            {
                Ok(Some(info)) => {
                    tracing::debug!(
                        platform = %p,
                        digest = %info.digest,
                        image = %ir.image,
                        "resolved platform-specific digest"
                    );
                    return Ok(Some(ResolvedImageVersion {
                        version: info.digest.clone(),
                        display_version: info
                            .created_at
                            .map(crate::registry::format_display_version),
                    }));
                }
                Ok(None) => {
                    return Err(PluginError::PluginInternal(
                        crate::error::DockerError::PlatformNotAvailable {
                            platform: p.to_string(),
                            image: ir.image.clone(),
                            tag: tag.to_string(),
                        }
                        .to_string(),
                    )
                    .into());
                }
                Err(e) => {
                    // When platform is configured, falling back to the image-index
                    // digest would create a permanent digest-namespace mismatch:
                    // `detect_installed_version` would return the index digest
                    // while `fetch_releases` always returns the platform-specific
                    // digest, making the item appear perpetually updatable.
                    //
                    // Instead, propagate the error as retryable so the scheduler
                    // retries on the next cycle. The `installed_version` in the DB
                    // keeps its last known good platform-specific digest until the
                    // registry is reachable again.
                    tracing::warn!(
                        error = %e,
                        image = %ir.image,
                        platform = %p,
                        "transient failure fetching platform manifest digest; \
                         returning error to preserve digest-namespace consistency"
                    );
                    return Err(PluginError::PluginInternal(format!(
                        "transient failure fetching platform manifest for {}/{} \
                         (platform: {p}): {e}",
                        ir.registry, ir.repository,
                    ))
                    .into());
                }
            }
        }

        // No explicit platform configured.  Attempt a best-effort, display-only
        // registry call using the platform detected from the local image's
        // os/arch metadata.  This gives multi-arch images (index manifests) a
        // human-readable publish date without changing the canonical `version`,
        // which stays as the image-index digest so update comparisons are
        // unaffected.
        let display_version = {
            let detected = crate::config::form_platform_string(
                digest_info.os.as_deref(),
                digest_info.architecture.as_deref(),
                digest_info.variant.as_deref(),
            );
            if let Some(ref p) = detected {
                match self
                    .registry_client
                    .get_platform_manifest_digest(&ir.registry, &ir.repository, tag, p)
                    .await
                {
                    Ok(Some(info)) => info.created_at.map(crate::registry::format_display_version),
                    Ok(None) | Err(_) => None,
                }
            } else {
                None
            }
        };

        Ok(Some(ResolvedImageVersion {
            version: digest_info.digest,
            display_version,
        }))
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for DockerPlugin {
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
        #[cfg(feature = "daemon")]
        self.ensure_daemon_client()
            .await
            .map_err(|e| PluginError::Configuration(e.to_string()))?;
        let client = Arc::clone(&*self.docker_client.lock());
        match self
            .resolve_image_info(&ir, tag, client, self.config.platform.as_deref())
            .await?
        {
            Some(resolved) => Ok(Some(Version::new(&resolved.version))),
            None => Ok(None),
        }
    }

    /// Batch installed-version detection with image-level deduplication.
    ///
    /// Multiple containers that share the same image (after `tracked_tag`
    /// resolution) are inspected only once, avoiding redundant Docker daemon
    /// calls for the common case where several items use e.g. `nginx:latest`.
    #[tracing::instrument(skip_all)]
    async fn batch_detect(
        &self,
        items: &[BatchDetectItem],
    ) -> uptrakit_plugin_infrastructure_core::Result<Vec<BatchDetectResult>> {
        use std::collections::HashMap;

        // Resolution cache: "image:tag::platform" → Ok(Some(ResolvedImageVersion)) | Ok(None) | Err(String)
        let mut resolution_cache: HashMap<
            String,
            std::result::Result<Option<ResolvedImageVersion>, String>,
        > = HashMap::new();

        #[cfg(feature = "daemon")]
        self.ensure_daemon_client()
            .await
            .map_err(|e| PluginError::Configuration(e.to_string()))?;

        // Pre-populate cache for all unique (image, tag, platform) combos.
        for item in items {
            let ir: ImageRef = match item.package_identifier.parse::<ImageRef>() {
                Ok(r) => r,
                Err(_) => continue,
            };
            let tag = self.config.resolved_tracked_tag(&ir.tag);
            let platform = self.config.platform.as_deref();
            let cache_key = format!("{}:{}::{}", ir.image, tag, platform.unwrap_or(""));

            if resolution_cache.contains_key(&cache_key) {
                continue;
            }

            let client = Arc::clone(&*self.docker_client.lock());
            let outcome = match self.resolve_image_info(&ir, tag, client, platform).await {
                Ok(r) => Ok(r),
                Err(e) => Err(e.to_string()),
            };
            resolution_cache.insert(cache_key, outcome);
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
            let platform = self.config.platform.as_deref();
            let cache_key = format!("{}:{}::{}", ir.image, tag, platform.unwrap_or(""));

            match resolution_cache.get(&cache_key) {
                Some(Ok(Some(resolved))) => {
                    let display_version = resolved.display_version.clone();
                    let mut r = BatchDetectResult::found(
                        item.package_identifier.clone(),
                        Version::new(&resolved.version),
                    );
                    r.display_version = display_version;
                    results.push(r);
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
