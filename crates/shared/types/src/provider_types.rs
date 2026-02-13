use std::fmt;

use serde::{Deserialize, Serialize};

/// Supported provider types.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    GithubReleases,
    ProxmoxHelperScripts,
    DockerRegistry,
    Homebrew,
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GithubReleases => write!(f, "github_releases"),
            Self::ProxmoxHelperScripts => write!(f, "proxmox_helper_scripts"),
            Self::DockerRegistry => write!(f, "docker_registry"),
            Self::Homebrew => write!(f, "homebrew"),
        }
    }
}

/// A downloadable asset attached to a release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// Simplified release info for update execution context.
///
/// Contains the minimal release metadata needed by providers to execute updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
    fn provider_type_docker_registry_serialization() {
        let dr = ProviderType::DockerRegistry;
        let json = serde_json::to_string(&dr).expect("serialize");
        assert_eq!(json, r#""docker_registry""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, dr);
    }

    #[test]
    fn provider_type_homebrew_serialization() {
        let hb = ProviderType::Homebrew;
        let json = serde_json::to_string(&hb).expect("serialize");
        assert_eq!(json, r#""homebrew""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, hb);
    }

    #[test]
    fn provider_type_display() {
        assert_eq!(ProviderType::GithubReleases.to_string(), "github_releases");
        assert_eq!(
            ProviderType::ProxmoxHelperScripts.to_string(),
            "proxmox_helper_scripts"
        );
        assert_eq!(ProviderType::DockerRegistry.to_string(), "docker_registry");
        assert_eq!(ProviderType::Homebrew.to_string(), "homebrew");
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
    fn release_info_serialization_roundtrip() {
        let info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com/release".to_string(),
            assets: vec![ReleaseAsset {
                name: "app.tar.gz".to_string(),
                download_url: "https://example.com/app.tar.gz".to_string(),
                size: Some(1024),
                content_type: None,
            }],
        };
        let json = serde_json::to_string(&info).expect("serialize");
        let deserialized: ReleaseInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, info);
    }

    #[test]
    fn release_info_empty_assets_omitted() {
        let info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com/release".to_string(),
            assets: vec![],
        };
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(!json.contains("assets"));
        let deserialized: ReleaseInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, info);
    }
}
