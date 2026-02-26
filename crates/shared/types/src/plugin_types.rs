use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Supported plugin types.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for plugin type strings received
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
/// [`RegistryError::UnknownPluginType`] for `Other(_)` — you cannot create
/// or validate a plugin whose type the binary does not implement.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub enum PluginType {
    GithubReleases,
    ProxmoxHelperScripts,
    Docker,
    Homebrew,
    Apt,
    /// An unknown plugin type received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    /// Registry operations (create, validate, mask) will return
    /// `UnknownPluginType` for this variant.
    Other(String),
}

impl PluginType {
    /// Returns the snake_case string representation of this plugin type.
    ///
    /// For [`PluginType::Other`], returns the inner string as-is.
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
/// [`PluginType`] variant.
///
/// Note: serde deserialization is *infallible* — unknown strings are mapped to
/// [`PluginType::Other`] rather than returning this error.  `ParsePluginTypeError`
/// is only returned from the [`FromStr`] implementation, which is used in
/// contexts where the caller must distinguish known from unknown plugin types
/// (registry validation, URL query parameters, etc.).
#[derive(Debug, Error)]
pub enum ParsePluginTypeError {
    /// The input string does not match any known plugin type.
    #[error("invalid plugin type value")]
    Invalid,
}

impl FromStr for PluginType {
    type Err = ParsePluginTypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github_releases" => Ok(Self::GithubReleases),
            "proxmox_helper_scripts" => Ok(Self::ProxmoxHelperScripts),
            "docker" => Ok(Self::Docker),
            "homebrew" => Ok(Self::Homebrew),
            "apt" => Ok(Self::Apt),
            _ => Err(ParsePluginTypeError::Invalid),
        }
    }
}

impl fmt::Display for PluginType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Serde: infallible string-based serialization ─────────────────────────────
//
// Custom Serialize/Deserialize are implemented manually rather than via derive
// so that unknown strings deserialize to `Other(String)` rather than failing.
// This makes rolling upgrades wire-safe: a message containing a new plugin
// type from a newer server can be fully parsed by an older client without
// dropping the enclosing struct.

impl From<String> for PluginType {
    /// Converts a snake_case string to a plugin type.
    ///
    /// Unknown strings map to [`PluginType::Other`] rather than failing.
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

impl From<PluginType> for String {
    fn from(pt: PluginType) -> String {
        match pt {
            PluginType::GithubReleases => "github_releases".to_string(),
            PluginType::ProxmoxHelperScripts => "proxmox_helper_scripts".to_string(),
            PluginType::Docker => "docker".to_string(),
            PluginType::Homebrew => "homebrew".to_string(),
            PluginType::Apt => "apt".to_string(),
            PluginType::Other(s) => s,
        }
    }
}

impl Serialize for PluginType {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PluginType {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Deserialize as a plain string, then convert via From<String>.
        // Unknown strings become Other(s) — this conversion is infallible.
        String::deserialize(deserializer).map(PluginType::from)
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
/// Contains the minimal release metadata needed by plugins to execute updates.
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
    fn plugin_type_serialization_roundtrip() {
        let gh = PluginType::GithubReleases;
        let json = serde_json::to_string(&gh).expect("serialize");
        assert_eq!(json, r#""github_releases""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, gh);
    }

    #[test]
    fn plugin_type_proxmox_serialization() {
        let phs = PluginType::ProxmoxHelperScripts;
        let json = serde_json::to_string(&phs).expect("serialize");
        assert_eq!(json, r#""proxmox_helper_scripts""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, phs);
    }

    #[test]
    fn plugin_type_docker_serialization() {
        let dr = PluginType::Docker;
        let json = serde_json::to_string(&dr).expect("serialize");
        assert_eq!(json, r#""docker""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, dr);
    }

    /// Old `"docker_registry"` wire strings from pre-migration data must
    /// deserialize to `Other("docker_registry")` rather than failing.
    #[test]
    fn plugin_type_docker_registry_legacy_deserializes_to_other() {
        let deserialized: PluginType =
            serde_json::from_str(r#""docker_registry""#).expect("deserialize legacy");
        assert_eq!(
            deserialized,
            PluginType::Other("docker_registry".to_string())
        );
    }

    #[test]
    fn plugin_type_homebrew_serialization() {
        let hb = PluginType::Homebrew;
        let json = serde_json::to_string(&hb).expect("serialize");
        assert_eq!(json, r#""homebrew""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, hb);
    }

    /// Unknown strings from a newer peer must deserialize to `Other(String)`
    /// rather than failing.  This is the core forward-compatibility guarantee.
    #[test]
    fn plugin_type_unknown_deserializes_to_other() {
        let deserialized: PluginType =
            serde_json::from_str(r#""winget""#).expect("deserialize unknown");
        assert_eq!(deserialized, PluginType::Other("winget".to_string()));

        let deserialized: PluginType =
            serde_json::from_str(r#""flatpak""#).expect("deserialize unknown");
        assert_eq!(deserialized, PluginType::Other("flatpak".to_string()));
    }

    /// `"apt"` deserializes to the known `Apt` variant, not `Other`.
    #[test]
    fn plugin_type_apt_deserializes_to_apt_variant() {
        let deserialized: PluginType =
            serde_json::from_str(r#""apt""#).expect("deserialize apt");
        assert_eq!(deserialized, PluginType::Apt);
    }

    /// `PluginType::Apt` serializes to `"apt"`.
    #[test]
    fn plugin_type_apt_serialization() {
        let apt = PluginType::Apt;
        let json = serde_json::to_string(&apt).expect("serialize");
        assert_eq!(json, r#""apt""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, apt);
    }

    /// `Other(String)` must serialize back to its inner string.
    #[test]
    fn plugin_type_other_serializes_to_inner_string() {
        let pt = PluginType::Other("flatpak".to_string());
        let json = serde_json::to_string(&pt).expect("serialize");
        assert_eq!(json, r#""flatpak""#);
    }

    /// Full serde roundtrip for `Other`: deserialize then re-serialize produces
    /// the original JSON string unchanged.
    #[test]
    fn plugin_type_other_roundtrip() {
        let original = r#""snap""#;
        let deserialized: PluginType = serde_json::from_str(original).expect("deserialize");
        assert_eq!(deserialized, PluginType::Other("snap".to_string()));
        let reserialized = serde_json::to_string(&deserialized).expect("serialize");
        assert_eq!(reserialized, original);
    }

    /// `From<String>` maps known strings to known variants and unknown strings
    /// to `Other`.
    #[test]
    fn plugin_type_from_string() {
        assert_eq!(
            PluginType::from("github_releases".to_string()),
            PluginType::GithubReleases
        );
        assert_eq!(
            PluginType::from("docker".to_string()),
            PluginType::Docker
        );
        assert_eq!(
            PluginType::from("apt".to_string()),
            PluginType::Apt
        );
        assert_eq!(
            PluginType::from("winget".to_string()),
            PluginType::Other("winget".to_string())
        );
        // Old wire string maps to Other — not to Docker
        assert_eq!(
            PluginType::from("docker_registry".to_string()),
            PluginType::Other("docker_registry".to_string())
        );
    }

    #[test]
    fn plugin_type_display() {
        assert_eq!(PluginType::GithubReleases.to_string(), "github_releases");
        assert_eq!(
            PluginType::ProxmoxHelperScripts.to_string(),
            "proxmox_helper_scripts"
        );
        assert_eq!(PluginType::Docker.to_string(), "docker");
        assert_eq!(PluginType::Homebrew.to_string(), "homebrew");
        assert_eq!(PluginType::Apt.to_string(), "apt");
        assert_eq!(
            PluginType::Other("custom_type".to_string()).to_string(),
            "custom_type"
        );
    }

    #[test]
    fn plugin_type_from_str_valid() {
        assert_eq!(
            "github_releases".parse::<PluginType>().ok(),
            Some(PluginType::GithubReleases)
        );
        assert_eq!(
            "proxmox_helper_scripts".parse::<PluginType>().ok(),
            Some(PluginType::ProxmoxHelperScripts)
        );
        assert_eq!(
            "docker".parse::<PluginType>().ok(),
            Some(PluginType::Docker)
        );
        assert_eq!(
            "homebrew".parse::<PluginType>().ok(),
            Some(PluginType::Homebrew)
        );
        assert_eq!(
            "apt".parse::<PluginType>().ok(),
            Some(PluginType::Apt)
        );
        // Old wire string must be rejected by FromStr (it becomes Other via serde)
        assert!("docker_registry".parse::<PluginType>().is_err());
    }

    /// `FromStr` still rejects unknown strings to preserve the registry's
    /// ability to distinguish known from unknown types in validation contexts.
    #[test]
    fn plugin_type_from_str_invalid_returns_err() {
        assert!("unknown".parse::<PluginType>().is_err());
        assert!("".parse::<PluginType>().is_err());
        assert!("GITHUB_RELEASES".parse::<PluginType>().is_err());
        assert!("GithubReleases".parse::<PluginType>().is_err());
    }

    #[test]
    fn plugin_type_from_str_error_display() {
        let err = "bad_value".parse::<PluginType>().unwrap_err();
        assert_eq!(err.to_string(), "invalid plugin type value");
    }

    /// Known variants round-trip through `FromStr`.
    #[test]
    fn plugin_type_display_round_trips_through_from_str() {
        let variants = [
            PluginType::GithubReleases,
            PluginType::ProxmoxHelperScripts,
            PluginType::Docker,
            PluginType::Homebrew,
            PluginType::Apt,
        ];
        for pt in &variants {
            let s = pt.to_string();
            let parsed: PluginType = s
                .parse()
                .expect("from_str should succeed for Display output of known variants");
            assert_eq!(&parsed, pt);
        }
    }

    #[test]
    fn plugin_type_as_str_matches_display() {
        let variants = [
            PluginType::GithubReleases,
            PluginType::ProxmoxHelperScripts,
            PluginType::Docker,
            PluginType::Homebrew,
            PluginType::Apt,
            PluginType::Other("my_plugin".to_string()),
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
