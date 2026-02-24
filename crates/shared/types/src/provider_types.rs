use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Supported provider types.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for provider type strings received
/// from a newer peer that this binary does not yet know about.  Serde
/// deserialization is infallible: an unknown string such as `"apt"` becomes
/// `Other("apt")` rather than a parse error, allowing older agents and web-API
/// clients to survive rolling upgrades without dropping entire messages.
///
/// `FromStr` retains its original error behaviour for *known-type* contexts
/// (registry validation, URL parameters, database columns) where a caller
/// explicitly needs to distinguish known variants from unknown ones.
///
/// The registry's dispatch table still returns
/// [`RegistryError::UnknownProviderType`] for `Other(_)` — you cannot create
/// or validate a provider whose type the binary does not implement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub enum ProviderType {
    GithubReleases,
    ProxmoxHelperScripts,
    Docker,
    Homebrew,
    Apt,
    /// An unknown provider type received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    /// Registry operations (create, validate, mask) will return
    /// `UnknownProviderType` for this variant.
    Other(String),
}

impl ProviderType {
    /// Returns the snake_case string representation of this provider type.
    ///
    /// For [`ProviderType::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::GithubReleases => "github_releases",
            Self::ProxmoxHelperScripts => "proxmox_helper_scripts",
            Self::Docker => "docker",
            Self::Homebrew => "homebrew",
            Self::Apt => "apt",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Error returned when parsing a string that does not match any *known*
/// [`ProviderType`] variant.
///
/// Note: serde deserialization is *infallible* — unknown strings are mapped to
/// [`ProviderType::Other`] rather than returning this error.  `ParseProviderTypeError`
/// is only returned from the [`FromStr`] implementation, which is used in
/// contexts where the caller must distinguish known from unknown provider types
/// (registry validation, URL query parameters, etc.).
#[derive(Debug, Error)]
pub enum ParseProviderTypeError {
    /// The input string does not match any known provider type.
    #[error("invalid provider type value")]
    Invalid,
}

impl FromStr for ProviderType {
    type Err = ParseProviderTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github_releases" => Ok(Self::GithubReleases),
            "proxmox_helper_scripts" => Ok(Self::ProxmoxHelperScripts),
            "docker" => Ok(Self::Docker),
            "homebrew" => Ok(Self::Homebrew),
            "apt" => Ok(Self::Apt),
            _ => Err(ParseProviderTypeError::Invalid),
        }
    }
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Serde: infallible string-based serialization ─────────────────────────────
//
// Custom Serialize/Deserialize are implemented manually rather than via derive
// so that unknown strings deserialize to `Other(String)` rather than failing.
// This makes rolling upgrades wire-safe: a message containing a new provider
// type from a newer server can be fully parsed by an older client without
// dropping the enclosing struct.

impl From<String> for ProviderType {
    /// Converts a snake_case string to a provider type.
    ///
    /// Unknown strings map to [`ProviderType::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "github_releases" => Self::GithubReleases,
            "proxmox_helper_scripts" => Self::ProxmoxHelperScripts,
            "docker" => Self::Docker,
            "homebrew" => Self::Homebrew,
            "apt" => Self::Apt,
            _ => Self::Other(s),
        }
    }
}

impl From<ProviderType> for String {
    fn from(pt: ProviderType) -> String {
        match pt {
            ProviderType::GithubReleases => "github_releases".to_string(),
            ProviderType::ProxmoxHelperScripts => "proxmox_helper_scripts".to_string(),
            ProviderType::Docker => "docker".to_string(),
            ProviderType::Homebrew => "homebrew".to_string(),
            ProviderType::Apt => "apt".to_string(),
            ProviderType::Other(s) => s,
        }
    }
}

impl Serialize for ProviderType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProviderType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize as a plain string, then convert via From<String>.
        // Unknown strings become Other(s) — this conversion is infallible.
        String::deserialize(deserializer).map(ProviderType::from)
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
    fn provider_type_docker_serialization() {
        let dr = ProviderType::Docker;
        let json = serde_json::to_string(&dr).expect("serialize");
        assert_eq!(json, r#""docker""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, dr);
    }

    /// Old `"docker_registry"` wire strings from pre-migration data must
    /// deserialize to `Other("docker_registry")` rather than failing.
    #[test]
    fn provider_type_docker_registry_legacy_deserializes_to_other() {
        let deserialized: ProviderType =
            serde_json::from_str(r#""docker_registry""#).expect("deserialize legacy");
        assert_eq!(
            deserialized,
            ProviderType::Other("docker_registry".to_string())
        );
    }

    #[test]
    fn provider_type_homebrew_serialization() {
        let hb = ProviderType::Homebrew;
        let json = serde_json::to_string(&hb).expect("serialize");
        assert_eq!(json, r#""homebrew""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, hb);
    }

    /// Unknown strings from a newer peer must deserialize to `Other(String)`
    /// rather than failing.  This is the core forward-compatibility guarantee.
    #[test]
    fn provider_type_unknown_deserializes_to_other() {
        let deserialized: ProviderType =
            serde_json::from_str(r#""winget""#).expect("deserialize unknown");
        assert_eq!(deserialized, ProviderType::Other("winget".to_string()));

        let deserialized: ProviderType =
            serde_json::from_str(r#""flatpak""#).expect("deserialize unknown");
        assert_eq!(deserialized, ProviderType::Other("flatpak".to_string()));
    }

    /// `"apt"` deserializes to the known `Apt` variant, not `Other`.
    #[test]
    fn provider_type_apt_deserializes_to_apt_variant() {
        let deserialized: ProviderType =
            serde_json::from_str(r#""apt""#).expect("deserialize apt");
        assert_eq!(deserialized, ProviderType::Apt);
    }

    /// `ProviderType::Apt` serializes to `"apt"`.
    #[test]
    fn provider_type_apt_serialization() {
        let apt = ProviderType::Apt;
        let json = serde_json::to_string(&apt).expect("serialize");
        assert_eq!(json, r#""apt""#);

        let deserialized: ProviderType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, apt);
    }

    /// `Other(String)` must serialize back to its inner string.
    #[test]
    fn provider_type_other_serializes_to_inner_string() {
        let pt = ProviderType::Other("flatpak".to_string());
        let json = serde_json::to_string(&pt).expect("serialize");
        assert_eq!(json, r#""flatpak""#);
    }

    /// Full serde roundtrip for `Other`: deserialize then re-serialize produces
    /// the original JSON string unchanged.
    #[test]
    fn provider_type_other_roundtrip() {
        let original = r#""snap""#;
        let deserialized: ProviderType = serde_json::from_str(original).expect("deserialize");
        assert_eq!(deserialized, ProviderType::Other("snap".to_string()));
        let reserialized = serde_json::to_string(&deserialized).expect("serialize");
        assert_eq!(reserialized, original);
    }

    /// `From<String>` maps known strings to known variants and unknown strings
    /// to `Other`.
    #[test]
    fn provider_type_from_string() {
        assert_eq!(
            ProviderType::from("github_releases".to_string()),
            ProviderType::GithubReleases
        );
        assert_eq!(
            ProviderType::from("docker".to_string()),
            ProviderType::Docker
        );
        assert_eq!(
            ProviderType::from("apt".to_string()),
            ProviderType::Apt
        );
        assert_eq!(
            ProviderType::from("winget".to_string()),
            ProviderType::Other("winget".to_string())
        );
        // Old wire string maps to Other — not to Docker
        assert_eq!(
            ProviderType::from("docker_registry".to_string()),
            ProviderType::Other("docker_registry".to_string())
        );
    }

    #[test]
    fn provider_type_display() {
        assert_eq!(ProviderType::GithubReleases.to_string(), "github_releases");
        assert_eq!(
            ProviderType::ProxmoxHelperScripts.to_string(),
            "proxmox_helper_scripts"
        );
        assert_eq!(ProviderType::Docker.to_string(), "docker");
        assert_eq!(ProviderType::Homebrew.to_string(), "homebrew");
        assert_eq!(ProviderType::Apt.to_string(), "apt");
        assert_eq!(
            ProviderType::Other("custom_type".to_string()).to_string(),
            "custom_type"
        );
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
            "docker".parse::<ProviderType>().ok(),
            Some(ProviderType::Docker)
        );
        assert_eq!(
            "homebrew".parse::<ProviderType>().ok(),
            Some(ProviderType::Homebrew)
        );
        assert_eq!(
            "apt".parse::<ProviderType>().ok(),
            Some(ProviderType::Apt)
        );
        // Old wire string must be rejected by FromStr (it becomes Other via serde)
        assert!("docker_registry".parse::<ProviderType>().is_err());
    }

    /// `FromStr` still rejects unknown strings to preserve the registry's
    /// ability to distinguish known from unknown types in validation contexts.
    #[test]
    fn provider_type_from_str_invalid_returns_err() {
        assert!("unknown".parse::<ProviderType>().is_err());
        assert!("".parse::<ProviderType>().is_err());
        assert!("GITHUB_RELEASES".parse::<ProviderType>().is_err());
        assert!("GithubReleases".parse::<ProviderType>().is_err());
    }

    #[test]
    fn provider_type_from_str_error_display() {
        let err = "bad_value".parse::<ProviderType>().unwrap_err();
        assert_eq!(err.to_string(), "invalid provider type value");
    }

    /// Known variants round-trip through `FromStr`.
    #[test]
    fn provider_type_display_round_trips_through_from_str() {
        let variants = [
            ProviderType::GithubReleases,
            ProviderType::ProxmoxHelperScripts,
            ProviderType::Docker,
            ProviderType::Homebrew,
            ProviderType::Apt,
        ];
        for pt in &variants {
            let s = pt.to_string();
            let parsed: ProviderType = s
                .parse()
                .expect("from_str should succeed for Display output of known variants");
            assert_eq!(&parsed, pt);
        }
    }

    #[test]
    fn provider_type_as_str_matches_display() {
        let variants = [
            ProviderType::GithubReleases,
            ProviderType::ProxmoxHelperScripts,
            ProviderType::Docker,
            ProviderType::Homebrew,
            ProviderType::Apt,
            ProviderType::Other("my_provider".to_string()),
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
