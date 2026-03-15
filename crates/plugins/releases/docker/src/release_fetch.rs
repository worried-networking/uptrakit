use async_trait::async_trait;
use rootcause::prelude::*;

use crate::image_ref::ImageRef;
use crate::plugin::DockerPlugin;
use uptrakit_plugin_infrastructure_core::{UpstreamRelease, Version};

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcherPlugin for DockerPlugin {
    #[tracing::instrument(skip_all)]
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

        let info = if let Some(ref platform) = self.config.platform {
            match self
                .registry_client
                .get_platform_manifest_digest(&ir.registry, &ir.repository, tag, platform)
                .await
                .context_to()?
            {
                Some(info) => info,
                None => {
                    return Err(
                        uptrakit_plugin_infrastructure_core::PluginError::PluginInternal(
                            crate::error::DockerError::PlatformNotAvailable {
                                platform: platform.clone(),
                                image: ir.image.clone(),
                                tag: tag.to_string(),
                            }
                            .to_string(),
                        )
                        .into(),
                    );
                }
            }
        } else {
            self.registry_client
                .get_manifest_info(&ir.registry, &ir.repository, tag)
                .await
                .context_to()?
        };

        let release_url = ir.web_url(&info.digest);
        let release = {
            let mut r =
                UpstreamRelease::new(Version::new(&info.digest), tag.to_string(), false, "");
            r.release_url = release_url;
            r.published_at = info.created_at;
            r
        };

        tracing::debug!(
            digest = %info.digest,
            tag = %tag,
            image = %ir.image,
            platform = ?self.config.platform,
            "fetched Docker release (digest mode)"
        );
        Ok(vec![release])
    }
}
