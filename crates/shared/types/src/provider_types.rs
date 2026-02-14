use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Supported provider types.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    GithubReleases,
    ProxmoxHelperScripts,
    DockerRegistry,
    Homebrew,
}

impl ProviderType {
    /// Returns the snake_case string representation of this provider type.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GithubReleases => "github_releases",
            Self::ProxmoxHelperScripts => "proxmox_helper_scripts",
            Self::DockerRegistry => "docker_registry",
            Self::Homebrew => "homebrew",
        }
    }
}

/// Error returned when parsing an invalid [`ProviderType`] string.
#[derive(Debug, Error)]
#[error("unknown provider type: {0}")]
pub struct ParseProviderTypeError(pub String);

impl FromStr for ProviderType {
    type Err = ParseProviderTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github_releases" => Ok(Self::GithubReleases),
            "proxmox_helper_scripts" => Ok(Self::ProxmoxHelperScripts),
            "docker_registry" => Ok(Self::DockerRegistry),
            "homebrew" => Ok(Self::Homebrew),
            _ => Err(ParseProviderTypeError(s.to_string())),
        }
    }
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
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
    fn provider_type_from_str_valid() {
        assert_eq!(
            "github_releases".parse::<ProviderType>().ok(),
            Some(ProviderType::GithubReleases)
        );
        assert_eq!(
            "proxmox_helper_scripts".parse::<ProviderType>().ok(),
            Some(ProviderType::ProxmoxHelperScripts)
        );
        assert_eq!(
            "docker_registry".parse::<ProviderType>().ok(),
            Some(ProviderType::DockerRegistry)
        );
        assert_eq!(
            "homebrew".parse::<ProviderType>().ok(),
            Some(ProviderType::Homebrew)
        );
    }

    #[test]
    fn provider_type_from_str_invalid_returns_err() {
        assert!("unknown".parse::<ProviderType>().is_err());
        assert!("".parse::<ProviderType>().is_err());
        assert!("GITHUB_RELEASES".parse::<ProviderType>().is_err());
        assert!("GithubReleases".parse::<ProviderType>().is_err());
    }

    #[test]
    fn provider_type_from_str_error_contains_input() {
        let err = "bad_value".parse::<ProviderType>().unwrap_err();
        assert!(err.to_string().contains("bad_value"));
    }

    #[test]
    fn provider_type_display_round_trips_through_from_str() {
        let variants = [
            ProviderType::GithubReleases,
            ProviderType::ProxmoxHelperScripts,
            ProviderType::DockerRegistry,
            ProviderType::Homebrew,
        ];
        for pt in &variants {
            let s = pt.to_string();
            let parsed: ProviderType = s
                .parse()
                .expect("from_str should succeed for Display output");
            assert_eq!(&parsed, pt);
        }
    }

    #[test]
    fn provider_type_as_str_matches_display() {
        let variants = [
            ProviderType::GithubReleases,
            ProviderType::ProxmoxHelperScripts,
            ProviderType::DockerRegistry,
            ProviderType::Homebrew,
        ];
        for pt in &variants {
            assert_eq!(pt.as_str(), pt.to_string());
        }
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
