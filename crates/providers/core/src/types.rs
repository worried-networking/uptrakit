use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

pub use uptrakit_shared_types::{ProviderType, ReleaseAsset, ReleaseInfo};

use crate::version::Version;

/// Capabilities that a provider may support.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    /// Provider can discover locally installed software.
    DiscoverLocalSoftware,
    /// Provider can refresh/sync the local package index from remote sources.
    RefreshPackageIndex,
}

/// A piece of software discovered on the local system by a provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredSoftware {
    /// Provider-specific identifier for this software (e.g., package name, app slug).
    pub package_identifier: String,
    /// Human-readable display name.
    pub name: String,
    /// Currently installed version, if detected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<Version>,
    /// Additional provider-specific metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
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
            installed_version: Some(Version::new("2.53.0")),
            extra: Some(serde_json::json!({"install_path": "/usr/local/bin/prometheus"})),
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn discovered_software_optional_fields_omitted() {
        let sw = DiscoveredSoftware {
            package_identifier: "grafana".to_string(),
            name: "Grafana".to_string(),
            installed_version: None,
            extra: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(!json.contains("installed_version"));
        assert!(!json.contains("extra"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn discovered_software_equality() {
        let a = DiscoveredSoftware {
            package_identifier: "node-exporter".to_string(),
            name: "Node Exporter".to_string(),
            installed_version: Some(Version::new("1.8.0")),
            extra: None,
        };
        let b = a.clone();
        assert_eq!(a, b);

        let c = DiscoveredSoftware {
            package_identifier: "node-exporter".to_string(),
            name: "Node Exporter".to_string(),
            installed_version: Some(Version::new("1.9.0")),
            extra: None,
        };
        assert_ne!(a, c);
    }

    #[test]
    fn provider_capability_serialization_roundtrip() {
        let cap = ProviderCapability::DiscoverLocalSoftware;
        let json = serde_json::to_string(&cap).expect("serialize");
        assert_eq!(json, r#""discover_local_software""#);

        let deserialized: ProviderCapability = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, cap);
    }

    #[test]
    fn provider_capability_refresh_package_index_serialization_roundtrip() {
        let cap = ProviderCapability::RefreshPackageIndex;
        let json = serde_json::to_string(&cap).expect("serialize");
        assert_eq!(json, r#""refresh_package_index""#);

        let deserialized: ProviderCapability = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, cap);
    }

    #[test]
    fn provider_capability_is_copy() {
        let cap = ProviderCapability::DiscoverLocalSoftware;
        let cap2 = cap; // Copy, not move
        assert_eq!(cap, cap2);

        let cap3 = ProviderCapability::RefreshPackageIndex;
        let cap4 = cap3; // Copy, not move
        assert_eq!(cap3, cap4);
    }
}
