use serde::{Deserialize, Serialize};
use std::fmt;
use time::OffsetDateTime;

use crate::version::Version;

/// Supported provider types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    GithubReleases,
    ProxmoxHelperScripts,
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GithubReleases => write!(f, "github_releases"),
            Self::ProxmoxHelperScripts => write!(f, "proxmox_helper_scripts"),
        }
    }
}

/// A downloadable asset attached to a release.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    /// Asset filename.
    pub name: String,
    /// Direct download URL.
    pub download_url: String,
    /// File size in bytes, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// MIME content type, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
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
    fn provider_type_serialization_roundtrip() {
        let gh = ProviderType::GithubReleases;
        let json = serde_json::to_string(&gh).expect("serialize");
        assert_eq!(json, r#""github_releases""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, gh);
    }

    #[test]
    fn provider_type_proxmox_serialization() {
        let phs = ProviderType::ProxmoxHelperScripts;
        let json = serde_json::to_string(&phs).expect("serialize");
        assert_eq!(json, r#""proxmox_helper_scripts""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, phs);
    }

    #[test]
    fn provider_type_display() {
        assert_eq!(ProviderType::GithubReleases.to_string(), "github_releases");
        assert_eq!(
            ProviderType::ProxmoxHelperScripts.to_string(),
            "proxmox_helper_scripts"
        );
    }

    #[test]
    fn release_asset_serialization_roundtrip() {
        let asset = ReleaseAsset {
            name: "app-linux-amd64.tar.gz".to_string(),
            download_url: "https://example.com/download".to_string(),
            size: Some(12345),
            content_type: Some("application/gzip".to_string()),
        };
        let json = serde_json::to_string(&asset).expect("serialize");
        let deserialized: ReleaseAsset = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, asset);
    }

    #[test]
    fn release_asset_optional_fields_omitted() {
        let asset = ReleaseAsset {
            name: "app.zip".to_string(),
            download_url: "https://example.com/app.zip".to_string(),
            size: None,
            content_type: None,
        };
        let json = serde_json::to_string(&asset).expect("serialize");
        assert!(!json.contains("size"));
        assert!(!json.contains("content_type"));
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
}
