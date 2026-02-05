use async_trait::async_trait;
use regex::Regex;
use rootcause::report;

use uptrakit_provider_core::{Provider, UpstreamRelease, Version};

use crate::config::{DockerRegistryConfig, TrackingMode};
use crate::error::{DockerRegistryError, Result};
use crate::registry::RegistryClient;
use crate::tag::filter_and_sort_tags;

/// Docker Registry provider implementation.
///
/// Tracks container image tags from OCI/Docker registries.
/// Supports two tracking modes:
/// - **SemverTags**: filter tags by pattern, parse as semver, sort descending
/// - **DigestTracking**: track digest changes of a specific tag
pub struct DockerRegistryProvider {
    config: DockerRegistryConfig,
    registry_client: RegistryClient,
    tag_filters: Vec<Regex>,
}

impl DockerRegistryProvider {
    /// Create a new `DockerRegistryProvider` from the given configuration.
    ///
    /// Validates the configuration and pre-compiles tag filter regexes.
    pub fn new(config: DockerRegistryConfig) -> Result<Self> {
        config.validate()?;

        let registry_client = RegistryClient::new(&config)?;

        let tag_filters: Vec<Regex> = config
            .tag_patterns
            .iter()
            .map(|p| {
                Regex::new(p).map_err(|e| {
                    report!(DockerRegistryError::InvalidPattern(format!(
                        "invalid regex '{p}': {e}"
                    )))
                })
            })
            .collect::<Result<_>>()?;

        Ok(Self {
            config,
            registry_client,
            tag_filters,
        })
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
    async fn fetch_releases(&self) -> uptrakit_provider_core::Result<Vec<UpstreamRelease>> {
        match self.config.tracking_mode {
            TrackingMode::SemverTags => {
                let tags = self.registry_client.list_tags().await.map_err(|e| {
                    report!(uptrakit_provider_core::ProviderError::Configuration(
                        format!("failed to list tags: {e}")
                    ))
                })?;

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
                    .map_err(|e| {
                        report!(uptrakit_provider_core::ProviderError::Configuration(
                            format!("failed to get manifest digest: {e}")
                        ))
                    })?;

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrackingMode;

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
        }
    }

    #[test]
    fn provider_creation_succeeds() {
        let config = test_config();
        assert!(DockerRegistryProvider::new(config).is_ok());
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
        };
        assert!(DockerRegistryProvider::new(config).is_err());
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
        };
        assert!(DockerRegistryProvider::new(config).is_err());
    }

    #[test]
    fn tags_to_releases_basic() {
        let config = test_config();
        let provider = DockerRegistryProvider::new(config).expect("valid config");
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
        let mut config = test_config();
        config.tag_strip_prefix = "v".to_string();
        let provider = DockerRegistryProvider::new(config).expect("valid config");
        let tags = vec!["v1.0.0".to_string(), "v2.0.0".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 2);
        assert_eq!(releases[0].version.as_str(), "2.0.0");
        assert_eq!(releases[0].tag, "v2.0.0");
    }

    #[test]
    fn tags_to_releases_no_semver_tags() {
        let config = test_config();
        let provider = DockerRegistryProvider::new(config).expect("valid config");
        let tags = vec!["latest".to_string(), "alpine".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert!(releases.is_empty());
    }

    #[test]
    fn tags_to_releases_release_url() {
        let config = test_config();
        let provider = DockerRegistryProvider::new(config).expect("valid config");
        let tags = vec!["1.25.0".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 1);
        assert!(releases[0].release_url.contains("hub.docker.com"));
        assert!(releases[0].release_url.contains("1.25.0"));
    }

    #[test]
    fn tags_to_releases_prerelease_detection() {
        let mut config = test_config();
        config.include_prereleases = true;
        let provider = DockerRegistryProvider::new(config).expect("valid config");
        let tags = vec!["1.0.0".to_string(), "2.0.0-beta.1".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert_eq!(releases.len(), 2);
        assert!(!releases[1].is_prerelease); // 1.0.0
        assert!(releases[0].is_prerelease); // 2.0.0-beta.1
    }

    #[test]
    fn tags_to_releases_no_release_notes_or_published_at() {
        let config = test_config();
        let provider = DockerRegistryProvider::new(config).expect("valid config");
        let tags = vec!["1.0.0".to_string()];
        let releases = provider.tags_to_releases(tags);
        assert!(releases[0].release_notes.is_none());
        assert!(releases[0].published_at.is_none());
        assert!(releases[0].assets.is_empty());
    }
}
