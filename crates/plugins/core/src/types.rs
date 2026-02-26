use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use uptrakit_shared_types::{DiscoveredSoftware, PluginType, ReleaseAsset, ReleaseInfo};

use crate::version::Version;

/// Capabilities that a plugin may support.
///
/// # Design note
///
/// This is a closed, centralized enum rather than a trait-based capability system.
/// All plugins in this project are first-party and registered exclusively through
/// `uptrakit-plugin-registry` (see AGENTS.md: "adding a new plugin only through
/// the registry is an acceptable tradeoff"). `#[non_exhaustive]` allows adding new
/// variants in future releases without requiring downstream match-arm updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    /// Plugin can discover locally installed software.
    DiscoverLocalSoftware,
    /// Plugin can refresh/sync the local package index from remote sources.
    RefreshPackageIndex,
    /// Plugin can determine whether it is applicable to the current host.
    DetectHostCompatibility,
    /// Plugin can run logic before an update is applied.
    PreUpdateHook,
    /// Plugin can run logic after an update is applied.
    PostUpdateHook,
    /// Plugin's `fetch_releases()` does not require any local system state
    /// (no package index, no filesystem access, no local commands) and can
    /// be called from the controller process directly rather than through
    /// an agent.
    ///
    /// All `fetch_releases` calls default to agent-side. This capability is
    /// an explicit opt-in to controller-side execution. The user can override
    /// via `execution_site` on the plugin assignment.
    ControllerSideFetchReleases,
}

/// Metadata for an upstream software release.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpstreamRelease {
    /// Parsed version (with optional semver).
    pub version: Version,
    /// Original tag name from the source.
    pub tag: String,
    /// Whether this is a pre-release.
    pub is_prerelease: bool,
    /// URL to the release page.
    pub release_url: String,
    /// Release notes / changelog text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    /// When the release was published.
    #[serde(default, with = "crate::serde_helpers::optional_rfc3339")]
    pub published_at: Option<OffsetDateTime>,
    /// Downloadable assets.
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_capability_new_variants_exist() {
        // Ensure the new variants compile and can be compared.
        assert_eq!(
            PluginCapability::DetectHostCompatibility,
            PluginCapability::DetectHostCompatibility
        );
        assert_eq!(PluginCapability::PreUpdateHook, PluginCapability::PreUpdateHook);
        assert_eq!(PluginCapability::PostUpdateHook, PluginCapability::PostUpdateHook);
        // They should be distinct from the original two.
        assert_ne!(
            PluginCapability::DetectHostCompatibility,
            PluginCapability::DiscoverLocalSoftware
        );
        assert_ne!(
            PluginCapability::PreUpdateHook,
            PluginCapability::RefreshPackageIndex
        );
        assert_eq!(
            PluginCapability::ControllerSideFetchReleases,
            PluginCapability::ControllerSideFetchReleases
        );
        assert_ne!(
            PluginCapability::ControllerSideFetchReleases,
            PluginCapability::RefreshPackageIndex
        );
    }

    #[test]
    fn upstream_release_serialization_roundtrip() {
        let release = UpstreamRelease {
            version: Version::new("1.0.0"),
            tag: "v1.0.0".to_string(),
            is_prerelease: false,
            release_url: "https://github.com/owner/repo/releases/tag/v1.0.0".to_string(),
            release_notes: Some("Initial release".to_string()),
            published_at: Some(
                OffsetDateTime::from_unix_timestamp(1706400000).expect("valid timestamp"),
            ),
            assets: vec![ReleaseAsset {
                name: "app.tar.gz".to_string(),
                download_url: "https://example.com/app.tar.gz".to_string(),
                size: Some(1024),
                content_type: None,
            }],
        };
        let json = serde_json::to_string(&release).expect("serialize");
        let deserialized: UpstreamRelease = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.version, release.version);
        assert_eq!(deserialized.tag, release.tag);
        assert_eq!(deserialized.is_prerelease, release.is_prerelease);
        assert_eq!(deserialized.assets.len(), 1);
    }

    #[test]
    fn upstream_release_minimal() {
        let release = UpstreamRelease {
            version: Version::new("0.1.0"),
            tag: "0.1.0".to_string(),
            is_prerelease: true,
            release_url: "https://example.com".to_string(),
            release_notes: None,
            published_at: None,
            assets: vec![],
        };
        let json = serde_json::to_string(&release).expect("serialize");
        assert!(!json.contains("release_notes"));
        let deserialized: UpstreamRelease = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, release);
    }

    #[test]
    fn discovered_software_serialization_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "prometheus".to_string(),
            name: "Prometheus".to_string(),
            installed_version: "2.53.0".to_string(),
            extra: Some(serde_json::json!({"install_path": "/usr/local/bin/prometheus"})),
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn discovered_software_optional_extra_omitted() {
        let sw = DiscoveredSoftware {
            package_identifier: "grafana".to_string(),
            name: "Grafana".to_string(),
            installed_version: "10.0.0".to_string(),
            extra: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(!json.contains("extra"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }
}
