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
    ReleasesGithub,
    DiscoveryProxmoxHelperScripts,
    ReleasesDocker,
    PackageManagerHomebrew,
    PackageManagerApt,
    PackageManagerNpm,
    GenericShell,
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
            Self::ReleasesGithub => "releases_github",
            Self::DiscoveryProxmoxHelperScripts => "discovery_proxmox_helper_scripts",
            Self::ReleasesDocker => "releases_docker",
            Self::PackageManagerHomebrew => "package_manager_homebrew",
            Self::PackageManagerApt => "package_manager_apt",
            Self::PackageManagerNpm => "package_manager_npm",
            Self::GenericShell => "generic_shell",
            Self::Other(s) => s.as_str(),
        }
    }

    /// Returns a human-readable display name for this plugin type.
    ///
    /// For [`PluginType::Other`], returns the raw wire string as-is.
    pub fn display_name(&self) -> &str {
        match self {
            Self::ReleasesGithub => "GitHub Releases",
            Self::ReleasesDocker => "Docker",
            Self::DiscoveryProxmoxHelperScripts => "Proxmox Helper Scripts",
            Self::PackageManagerHomebrew => "Homebrew",
            Self::PackageManagerApt => "APT",
            Self::PackageManagerNpm => "npm",
            Self::GenericShell => "Shell",
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
            "releases_github" => Ok(Self::ReleasesGithub),
            "discovery_proxmox_helper_scripts" => Ok(Self::DiscoveryProxmoxHelperScripts),
            "releases_docker" => Ok(Self::ReleasesDocker),
            "package_manager_homebrew" => Ok(Self::PackageManagerHomebrew),
            "package_manager_apt" => Ok(Self::PackageManagerApt),
            "package_manager_npm" => Ok(Self::PackageManagerNpm),
            "generic_shell" => Ok(Self::GenericShell),
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
            "releases_github" => Self::ReleasesGithub,
            "discovery_proxmox_helper_scripts" => Self::DiscoveryProxmoxHelperScripts,
            "releases_docker" => Self::ReleasesDocker,
            "package_manager_homebrew" => Self::PackageManagerHomebrew,
            "package_manager_apt" => Self::PackageManagerApt,
            "package_manager_npm" => Self::PackageManagerNpm,
            "generic_shell" => Self::GenericShell,
            _ => Self::Other(s),
        }
    }
}

impl From<PluginType> for String {
    fn from(pt: PluginType) -> String {
        match pt {
            PluginType::ReleasesGithub => "releases_github".to_string(),
            PluginType::DiscoveryProxmoxHelperScripts => {
                "discovery_proxmox_helper_scripts".to_string()
            }
            PluginType::ReleasesDocker => "releases_docker".to_string(),
            PluginType::PackageManagerHomebrew => "package_manager_homebrew".to_string(),
            PluginType::PackageManagerApt => "package_manager_apt".to_string(),
            PluginType::PackageManagerNpm => "package_manager_npm".to_string(),
            PluginType::GenericShell => "generic_shell".to_string(),
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
        let gh = PluginType::ReleasesGithub;
        let json = serde_json::to_string(&gh).expect("serialize");
        assert_eq!(json, r#""releases_github""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, gh);
    }

    #[test]
    fn plugin_type_proxmox_serialization() {
        let phs = PluginType::DiscoveryProxmoxHelperScripts;
        let json = serde_json::to_string(&phs).expect("serialize");
        assert_eq!(json, r#""discovery_proxmox_helper_scripts""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, phs);
    }

    #[test]
    fn plugin_type_docker_serialization() {
        let dr = PluginType::ReleasesDocker;
        let json = serde_json::to_string(&dr).expect("serialize");
        assert_eq!(json, r#""releases_docker""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, dr);
    }

    #[test]
    fn plugin_type_homebrew_serialization() {
        let hb = PluginType::PackageManagerHomebrew;
        let json = serde_json::to_string(&hb).expect("serialize");
        assert_eq!(json, r#""package_manager_homebrew""#);

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

    /// `"package_manager_apt"` deserializes to the known `PackageManagerApt` variant, not `Other`.
    #[test]
    fn plugin_type_apt_deserializes_to_apt_variant() {
        let deserialized: PluginType =
            serde_json::from_str(r#""package_manager_apt""#).expect("deserialize apt");
        assert_eq!(deserialized, PluginType::PackageManagerApt);
    }

    /// `PluginType::PackageManagerApt` serializes to `"package_manager_apt"`.
    #[test]
    fn plugin_type_apt_serialization() {
        let apt = PluginType::PackageManagerApt;
        let json = serde_json::to_string(&apt).expect("serialize");
        assert_eq!(json, r#""package_manager_apt""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, apt);
    }

    #[test]
    fn plugin_type_shell_serialization() {
        let shell = PluginType::GenericShell;
        let json = serde_json::to_string(&shell).expect("serialize");
        assert_eq!(json, r#""generic_shell""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, shell);
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
            PluginType::from("releases_github".to_string()),
            PluginType::ReleasesGithub
        );
        assert_eq!(
            PluginType::from("releases_docker".to_string()),
            PluginType::ReleasesDocker
        );
        assert_eq!(
            PluginType::from("package_manager_apt".to_string()),
            PluginType::PackageManagerApt
        );
        assert_eq!(
            PluginType::from("package_manager_npm".to_string()),
            PluginType::PackageManagerNpm
        );
        assert_eq!(
            PluginType::from("generic_shell".to_string()),
            PluginType::GenericShell
        );
        assert_eq!(
            PluginType::from("winget".to_string()),
            PluginType::Other("winget".to_string())
        );
        // Old wire strings map to Other
        assert_eq!(
            PluginType::from("docker_registry".to_string()),
            PluginType::Other("docker_registry".to_string())
        );
        assert_eq!(
            PluginType::from("github_releases".to_string()),
            PluginType::Other("github_releases".to_string())
        );
    }

    #[test]
    fn plugin_type_display() {
        assert_eq!(PluginType::ReleasesGithub.to_string(), "releases_github");
        assert_eq!(
            PluginType::DiscoveryProxmoxHelperScripts.to_string(),
            "discovery_proxmox_helper_scripts"
        );
        assert_eq!(PluginType::ReleasesDocker.to_string(), "releases_docker");
        assert_eq!(
            PluginType::PackageManagerHomebrew.to_string(),
            "package_manager_homebrew"
        );
        assert_eq!(
            PluginType::PackageManagerApt.to_string(),
            "package_manager_apt"
        );
        assert_eq!(
            PluginType::PackageManagerNpm.to_string(),
            "package_manager_npm"
        );
        assert_eq!(PluginType::GenericShell.to_string(), "generic_shell");
        assert_eq!(
            PluginType::Other("custom_type".to_string()).to_string(),
            "custom_type"
        );
    }

    #[test]
    fn plugin_type_from_str_valid() {
        assert_eq!(
            "releases_github".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesGithub)
        );
        assert_eq!(
            "discovery_proxmox_helper_scripts"
                .parse::<PluginType>()
                .ok(),
            Some(PluginType::DiscoveryProxmoxHelperScripts)
        );
        assert_eq!(
            "releases_docker".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesDocker)
        );
        assert_eq!(
            "package_manager_homebrew".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerHomebrew)
        );
        assert_eq!(
            "package_manager_apt".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerApt)
        );
        assert_eq!(
            "package_manager_npm".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerNpm)
        );
        assert_eq!(
            "generic_shell".parse::<PluginType>().ok(),
            Some(PluginType::GenericShell)
        );
        // Old wire strings must be rejected by FromStr
        assert!("docker_registry".parse::<PluginType>().is_err());
        assert!("github_releases".parse::<PluginType>().is_err());
        assert!("docker".parse::<PluginType>().is_err());
    }

    /// `FromStr` still rejects unknown strings to preserve the registry's
    /// ability to distinguish known from unknown types in validation contexts.
    #[test]
    fn plugin_type_from_str_invalid_returns_err() {
        assert!("unknown".parse::<PluginType>().is_err());
        assert!("".parse::<PluginType>().is_err());
        assert!("RELEASES_GITHUB".parse::<PluginType>().is_err());
        assert!("ReleasesGithub".parse::<PluginType>().is_err());
    }

    #[test]
    fn plugin_type_from_str_error_display() {
        let err = "bad_value".parse::<PluginType>().unwrap_err();
        assert_eq!(err.to_string(), "invalid plugin type value");
    }

    /// Known variants round-trip through `FromStr`.
    #[test]
    fn display_name_known_variants() {
        assert_eq!(PluginType::ReleasesGithub.display_name(), "GitHub Releases");
        assert_eq!(PluginType::ReleasesDocker.display_name(), "Docker");
        assert_eq!(
            PluginType::DiscoveryProxmoxHelperScripts.display_name(),
            "Proxmox Helper Scripts"
        );
        assert_eq!(PluginType::PackageManagerHomebrew.display_name(), "Homebrew");
        assert_eq!(PluginType::PackageManagerApt.display_name(), "APT");
        assert_eq!(PluginType::PackageManagerNpm.display_name(), "npm");
        assert_eq!(PluginType::GenericShell.display_name(), "Shell");
    }

    #[test]
    fn display_name_other_returns_raw_string() {
        let pt = PluginType::Other("custom_plugin".to_string());
        assert_eq!(pt.display_name(), "custom_plugin");
    }

    #[test]
    fn plugin_type_display_round_trips_through_from_str() {
        let variants = [
            PluginType::ReleasesGithub,
            PluginType::DiscoveryProxmoxHelperScripts,
            PluginType::ReleasesDocker,
            PluginType::PackageManagerHomebrew,
            PluginType::PackageManagerApt,
            PluginType::PackageManagerNpm,
            PluginType::GenericShell,
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
            PluginType::ReleasesGithub,
            PluginType::DiscoveryProxmoxHelperScripts,
            PluginType::ReleasesDocker,
            PluginType::PackageManagerHomebrew,
            PluginType::PackageManagerApt,
            PluginType::PackageManagerNpm,
            PluginType::GenericShell,
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
