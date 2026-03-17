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
    ReleasesGitlab,
    ReleasesForgejo,
    DiscoveryProxmoxHelperScripts,
    ReleasesDocker,
    PackageManagerHomebrew,
    PackageManagerApt,
    PackageManagerDnf,
    PackageManagerNpm,
    PackageManagerMas,
    PackageManagerPacman,
    PackageManagerPkg,
    PackageManagerApk,
    PackageManagerSnap,
    PackageManagerCargo,
    GenericShell,
    InfrastructureProxmox,
    /// Update lifecycle hook: stop/start a systemd service around updates.
    HookSystemd,
    /// Update lifecycle hook: run arbitrary shell commands before/after updates.
    HookShell,
    /// Enhancement: automatic icon assignment from Dashboard Icons.
    EnhancementDashboardIcons,
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
            Self::ReleasesGitlab => "releases_gitlab",
            Self::ReleasesForgejo => "releases_forgejo",
            Self::DiscoveryProxmoxHelperScripts => "discovery_proxmox_helper_scripts",
            Self::ReleasesDocker => "releases_docker",
            Self::PackageManagerHomebrew => "package_manager_homebrew",
            Self::PackageManagerApt => "package_manager_apt",
            Self::PackageManagerDnf => "package_manager_dnf",
            Self::PackageManagerNpm => "package_manager_npm",
            Self::PackageManagerMas => "package_manager_mas",
            Self::PackageManagerPacman => "package_manager_pacman",
            Self::PackageManagerPkg => "package_manager_pkg",
            Self::PackageManagerApk => "package_manager_apk",
            Self::PackageManagerSnap => "package_manager_snap",
            Self::PackageManagerCargo => "package_manager_cargo",
            Self::GenericShell => "generic_shell",
            Self::InfrastructureProxmox => "infrastructure_proxmox",
            Self::HookSystemd => "hook_systemd",
            Self::HookShell => "hook_shell",
            Self::EnhancementDashboardIcons => "enhancement_dashboard_icons",
            Self::Other(s) => s.as_str(),
        }
    }

    /// Returns `true` if this plugin type is a package manager.
    ///
    /// Package managers use tenant-level `plugin_type_settings` rather than
    /// per-config `plugin_configs` rows, and their `host_software_item_plugin`
    /// rows carry `plugin_config_id = NULL`.
    pub fn is_package_manager(&self) -> bool {
        matches!(
            self,
            Self::PackageManagerHomebrew
                | Self::PackageManagerApt
                | Self::PackageManagerDnf
                | Self::PackageManagerNpm
                | Self::PackageManagerMas
                | Self::PackageManagerPacman
                | Self::PackageManagerPkg
                | Self::PackageManagerApk
                | Self::PackageManagerSnap
                | Self::PackageManagerCargo
        )
    }

    /// Returns a human-readable display name for this plugin type.
    ///
    /// For [`PluginType::Other`], returns the raw wire string as-is.
    pub fn display_name(&self) -> &str {
        match self {
            Self::ReleasesGithub => "GitHub Releases",
            Self::ReleasesGitlab => "GitLab Releases",
            Self::ReleasesForgejo => "Forgejo Releases",
            Self::ReleasesDocker => "Docker",
            Self::DiscoveryProxmoxHelperScripts => "Proxmox Helper Scripts",
            Self::PackageManagerHomebrew => "Homebrew",
            Self::PackageManagerApt => "APT",
            Self::PackageManagerDnf => "DNF",
            Self::PackageManagerNpm => "npm",
            Self::PackageManagerMas => "Mac App Store",
            Self::PackageManagerPacman => "Pacman",
            Self::PackageManagerPkg => "BSD pkg",
            Self::PackageManagerApk => "APK",
            Self::PackageManagerSnap => "Snap",
            Self::PackageManagerCargo => "cargo install",
            Self::GenericShell => "Shell",
            Self::InfrastructureProxmox => "Proxmox VE",
            Self::HookSystemd => "Systemd Hook",
            Self::HookShell => "Shell Hook",
            Self::EnhancementDashboardIcons => "Dashboard Icons",
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
            "releases_gitlab" => Ok(Self::ReleasesGitlab),
            "releases_forgejo" => Ok(Self::ReleasesForgejo),
            "discovery_proxmox_helper_scripts" => Ok(Self::DiscoveryProxmoxHelperScripts),
            "releases_docker" => Ok(Self::ReleasesDocker),
            "package_manager_homebrew" => Ok(Self::PackageManagerHomebrew),
            "package_manager_apt" => Ok(Self::PackageManagerApt),
            "package_manager_dnf" => Ok(Self::PackageManagerDnf),
            "package_manager_npm" => Ok(Self::PackageManagerNpm),
            "package_manager_mas" => Ok(Self::PackageManagerMas),
            "package_manager_pacman" => Ok(Self::PackageManagerPacman),
            "package_manager_pkg" => Ok(Self::PackageManagerPkg),
            "package_manager_apk" => Ok(Self::PackageManagerApk),
            "package_manager_snap" => Ok(Self::PackageManagerSnap),
            "package_manager_cargo" => Ok(Self::PackageManagerCargo),
            "generic_shell" => Ok(Self::GenericShell),
            "infrastructure_proxmox" => Ok(Self::InfrastructureProxmox),
            "hook_systemd" => Ok(Self::HookSystemd),
            "hook_shell" => Ok(Self::HookShell),
            "enhancement_dashboard_icons" => Ok(Self::EnhancementDashboardIcons),
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
            "releases_gitlab" => Self::ReleasesGitlab,
            "releases_forgejo" => Self::ReleasesForgejo,
            "discovery_proxmox_helper_scripts" => Self::DiscoveryProxmoxHelperScripts,
            "releases_docker" => Self::ReleasesDocker,
            "package_manager_homebrew" => Self::PackageManagerHomebrew,
            "package_manager_apt" => Self::PackageManagerApt,
            "package_manager_dnf" => Self::PackageManagerDnf,
            "package_manager_npm" => Self::PackageManagerNpm,
            "package_manager_mas" => Self::PackageManagerMas,
            "package_manager_pacman" => Self::PackageManagerPacman,
            "package_manager_pkg" => Self::PackageManagerPkg,
            "package_manager_apk" => Self::PackageManagerApk,
            "package_manager_snap" => Self::PackageManagerSnap,
            "package_manager_cargo" => Self::PackageManagerCargo,
            "generic_shell" => Self::GenericShell,
            "infrastructure_proxmox" => Self::InfrastructureProxmox,
            "hook_systemd" => Self::HookSystemd,
            "hook_shell" => Self::HookShell,
            "enhancement_dashboard_icons" => Self::EnhancementDashboardIcons,
            _ => Self::Other(s),
        }
    }
}

impl From<PluginType> for String {
    fn from(pt: PluginType) -> String {
        match pt {
            PluginType::ReleasesGithub => "releases_github".to_string(),
            PluginType::ReleasesGitlab => "releases_gitlab".to_string(),
            PluginType::ReleasesForgejo => "releases_forgejo".to_string(),
            PluginType::DiscoveryProxmoxHelperScripts => {
                "discovery_proxmox_helper_scripts".to_string()
            }
            PluginType::ReleasesDocker => "releases_docker".to_string(),
            PluginType::PackageManagerHomebrew => "package_manager_homebrew".to_string(),
            PluginType::PackageManagerApt => "package_manager_apt".to_string(),
            PluginType::PackageManagerDnf => "package_manager_dnf".to_string(),
            PluginType::PackageManagerNpm => "package_manager_npm".to_string(),
            PluginType::PackageManagerMas => "package_manager_mas".to_string(),
            PluginType::PackageManagerPacman => "package_manager_pacman".to_string(),
            PluginType::PackageManagerPkg => "package_manager_pkg".to_string(),
            PluginType::PackageManagerApk => "package_manager_apk".to_string(),
            PluginType::PackageManagerSnap => "package_manager_snap".to_string(),
            PluginType::PackageManagerCargo => "package_manager_cargo".to_string(),
            PluginType::GenericShell => "generic_shell".to_string(),
            PluginType::InfrastructureProxmox => "infrastructure_proxmox".to_string(),
            PluginType::HookSystemd => "hook_systemd".to_string(),
            PluginType::HookShell => "hook_shell".to_string(),
            PluginType::EnhancementDashboardIcons => "enhancement_dashboard_icons".to_string(),
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

/// Attestation status for a GitHub release as determined by the GitHub Attestations API.
///
/// Used by the controller to record results of `actions/attest`-based Sigstore
/// provenance checks and by the agent to enforce the `require_attestation` policy.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all variant for status strings received from a
/// newer peer that this binary does not yet know about. Serde deserialization
/// is infallible: an unknown string becomes `Other(s)` rather than a parse error,
/// allowing older agents and web-API clients to survive rolling upgrades.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[non_exhaustive]
pub enum AttestationStatus {
    /// At least one release asset was verified via the GitHub Attestations API.
    Verified,
    /// GitHub Attestations API returned no attestations for the release asset digest.
    NotFound,
    /// Attestation check was not performed (no checksums file found or check disabled).
    Unverified,
    /// An unknown attestation status received from a newer peer.
    Other(String),
}

impl From<String> for AttestationStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "Verified" => Self::Verified,
            "NotFound" => Self::NotFound,
            "Unverified" => Self::Unverified,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for AttestationStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let s = match self {
            Self::Verified => "Verified",
            Self::NotFound => "NotFound",
            Self::Unverified => "Unverified",
            Self::Other(s) => s.as_str(),
        };
        serializer.serialize_str(s)
    }
}

impl<'de> Deserialize<'de> for AttestationStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(AttestationStatus::from)
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
    /// SHA-256 digest from the release checksums file, if available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256_digest: Option<String>,
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
    /// Attestation status as determined by the GitHub Attestations API.
    ///
    /// Set by the controller from the most recent `fetch_releases` run.
    /// `None` means the check was never performed or the source is not GitHub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attestation_status: Option<AttestationStatus>,
    /// When `true`, the agent must abort the update if attestation is not `Verified`.
    ///
    /// Copied from `GitHubConfig.require_attestation` by the controller at
    /// trigger time.
    #[serde(default)]
    pub require_attestation: bool,
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
    fn plugin_type_gitlab_serialization_roundtrip() {
        let gl = PluginType::ReleasesGitlab;
        let json = serde_json::to_string(&gl).expect("serialize");
        assert_eq!(json, r#""releases_gitlab""#);
        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, gl);
    }

    #[test]
    fn plugin_type_forgejo_serialization_roundtrip() {
        let cb = PluginType::ReleasesForgejo;
        let json = serde_json::to_string(&cb).expect("serialize");
        assert_eq!(json, r#""releases_forgejo""#);
        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, cb);
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

    /// `PluginType::PackageManagerApk` serializes to `"package_manager_apk"`.
    #[test]
    fn plugin_type_apk_serialization() {
        let apk = PluginType::PackageManagerApk;
        let json = serde_json::to_string(&apk).expect("serialize");
        assert_eq!(json, r#""package_manager_apk""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, apk);
    }

    #[test]
    fn plugin_type_pacman_serialization() {
        let pacman = PluginType::PackageManagerPacman;
        let json = serde_json::to_string(&pacman).expect("serialize");
        assert_eq!(json, r#""package_manager_pacman""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, pacman);
    }

    #[test]
    fn plugin_type_shell_serialization() {
        let shell = PluginType::GenericShell;
        let json = serde_json::to_string(&shell).expect("serialize");
        assert_eq!(json, r#""generic_shell""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, shell);
    }

    #[test]
    fn plugin_type_pkg_serialization() {
        let pkg = PluginType::PackageManagerPkg;
        let json = serde_json::to_string(&pkg).expect("serialize");
        assert_eq!(json, r#""package_manager_pkg""#);
        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, pkg);
    }

    #[test]
    fn plugin_type_infrastructure_proxmox_serialization() {
        let pve = PluginType::InfrastructureProxmox;
        let json = serde_json::to_string(&pve).expect("serialize");
        assert_eq!(json, r#""infrastructure_proxmox""#);

        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, pve);
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
            PluginType::from("releases_gitlab".to_string()),
            PluginType::ReleasesGitlab
        );
        assert_eq!(
            PluginType::from("releases_forgejo".to_string()),
            PluginType::ReleasesForgejo
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
            PluginType::from("package_manager_mas".to_string()),
            PluginType::PackageManagerMas
        );
        assert_eq!(
            PluginType::from("package_manager_pacman".to_string()),
            PluginType::PackageManagerPacman
        );
        assert_eq!(
            PluginType::from("package_manager_pkg".to_string()),
            PluginType::PackageManagerPkg
        );
        assert_eq!(
            PluginType::from("package_manager_apk".to_string()),
            PluginType::PackageManagerApk
        );
        assert_eq!(
            PluginType::from("package_manager_snap".to_string()),
            PluginType::PackageManagerSnap
        );
        assert_eq!(
            PluginType::from("package_manager_cargo".to_string()),
            PluginType::PackageManagerCargo
        );
        assert_eq!(
            PluginType::from("generic_shell".to_string()),
            PluginType::GenericShell
        );
        assert_eq!(
            PluginType::from("infrastructure_proxmox".to_string()),
            PluginType::InfrastructureProxmox
        );
        assert_eq!(
            PluginType::from("hook_systemd".to_string()),
            PluginType::HookSystemd
        );
        assert_eq!(
            PluginType::from("hook_shell".to_string()),
            PluginType::HookShell
        );
        assert_eq!(
            PluginType::from("enhancement_dashboard_icons".to_string()),
            PluginType::EnhancementDashboardIcons
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
        assert_eq!(PluginType::ReleasesGitlab.to_string(), "releases_gitlab");
        assert_eq!(PluginType::ReleasesForgejo.to_string(), "releases_forgejo");
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
        assert_eq!(
            PluginType::PackageManagerMas.to_string(),
            "package_manager_mas"
        );
        assert_eq!(
            PluginType::PackageManagerPacman.to_string(),
            "package_manager_pacman"
        );
        assert_eq!(
            PluginType::PackageManagerPkg.to_string(),
            "package_manager_pkg"
        );
        assert_eq!(
            PluginType::PackageManagerApk.to_string(),
            "package_manager_apk"
        );
        assert_eq!(
            PluginType::PackageManagerSnap.to_string(),
            "package_manager_snap"
        );
        assert_eq!(
            PluginType::PackageManagerCargo.to_string(),
            "package_manager_cargo"
        );
        assert_eq!(PluginType::GenericShell.to_string(), "generic_shell");
        assert_eq!(
            PluginType::InfrastructureProxmox.to_string(),
            "infrastructure_proxmox"
        );
        assert_eq!(PluginType::HookSystemd.to_string(), "hook_systemd");
        assert_eq!(PluginType::HookShell.to_string(), "hook_shell");
        assert_eq!(
            PluginType::EnhancementDashboardIcons.to_string(),
            "enhancement_dashboard_icons"
        );
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
            "package_manager_mas".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerMas)
        );
        assert_eq!(
            "package_manager_pacman".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerPacman)
        );
        assert_eq!(
            "package_manager_pkg".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerPkg)
        );
        assert_eq!(
            "package_manager_apk".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerApk)
        );
        assert_eq!(
            "package_manager_snap".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerSnap)
        );
        assert_eq!(
            "package_manager_cargo".parse::<PluginType>().ok(),
            Some(PluginType::PackageManagerCargo)
        );
        assert_eq!(
            "releases_gitlab".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesGitlab)
        );
        assert_eq!(
            "releases_forgejo".parse::<PluginType>().ok(),
            Some(PluginType::ReleasesForgejo)
        );
        assert_eq!(
            "generic_shell".parse::<PluginType>().ok(),
            Some(PluginType::GenericShell)
        );
        assert_eq!(
            "infrastructure_proxmox".parse::<PluginType>().ok(),
            Some(PluginType::InfrastructureProxmox)
        );
        assert_eq!(
            "hook_systemd".parse::<PluginType>().ok(),
            Some(PluginType::HookSystemd)
        );
        assert_eq!(
            "hook_shell".parse::<PluginType>().ok(),
            Some(PluginType::HookShell)
        );
        assert_eq!(
            "enhancement_dashboard_icons".parse::<PluginType>().ok(),
            Some(PluginType::EnhancementDashboardIcons)
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
        assert_eq!(PluginType::ReleasesGitlab.display_name(), "GitLab Releases");
        assert_eq!(
            PluginType::ReleasesForgejo.display_name(),
            "Forgejo Releases"
        );
        assert_eq!(PluginType::ReleasesDocker.display_name(), "Docker");
        assert_eq!(
            PluginType::DiscoveryProxmoxHelperScripts.display_name(),
            "Proxmox Helper Scripts"
        );
        assert_eq!(
            PluginType::PackageManagerHomebrew.display_name(),
            "Homebrew"
        );
        assert_eq!(PluginType::PackageManagerApt.display_name(), "APT");
        assert_eq!(PluginType::PackageManagerNpm.display_name(), "npm");
        assert_eq!(
            PluginType::PackageManagerMas.display_name(),
            "Mac App Store"
        );
        assert_eq!(PluginType::PackageManagerPacman.display_name(), "Pacman");
        assert_eq!(PluginType::PackageManagerPkg.display_name(), "BSD pkg");
        assert_eq!(PluginType::PackageManagerApk.display_name(), "APK");
        assert_eq!(PluginType::PackageManagerSnap.display_name(), "Snap");
        assert_eq!(
            PluginType::PackageManagerCargo.display_name(),
            "cargo install"
        );
        assert_eq!(PluginType::GenericShell.display_name(), "Shell");
        assert_eq!(
            PluginType::InfrastructureProxmox.display_name(),
            "Proxmox VE"
        );
    }

    #[test]
    fn plugin_type_enhancement_dashboard_icons_serialization() {
        let di = PluginType::EnhancementDashboardIcons;
        let json = serde_json::to_string(&di).expect("serialize");
        assert_eq!(json, r#""enhancement_dashboard_icons""#);
        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, di);
    }

    #[test]
    fn display_name_other_returns_raw_string() {
        let pt = PluginType::Other("custom_plugin".to_string());
        assert_eq!(pt.display_name(), "custom_plugin");
    }

    #[test]
    fn plugin_type_snap_serialization() {
        let snap = PluginType::PackageManagerSnap;
        let json = serde_json::to_string(&snap).expect("serialize");
        assert_eq!(json, r#""package_manager_snap""#);
        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, snap);
    }

    #[test]
    fn plugin_type_snap_deserializes_to_snap_variant() {
        let deserialized: PluginType =
            serde_json::from_str(r#""package_manager_snap""#).expect("deserialize snap");
        assert_eq!(deserialized, PluginType::PackageManagerSnap);
    }

    #[test]
    fn plugin_type_cargo_serialization() {
        let cargo = PluginType::PackageManagerCargo;
        let json = serde_json::to_string(&cargo).expect("serialize");
        assert_eq!(json, r#""package_manager_cargo""#);
        let deserialized: PluginType = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, cargo);
    }

    #[test]
    fn plugin_type_cargo_deserializes_to_cargo_variant() {
        let deserialized: PluginType =
            serde_json::from_str(r#""package_manager_cargo""#).expect("deserialize cargo");
        assert_eq!(deserialized, PluginType::PackageManagerCargo);
    }

    #[test]
    fn plugin_type_display_round_trips_through_from_str() {
        let variants = [
            PluginType::ReleasesGithub,
            PluginType::ReleasesGitlab,
            PluginType::ReleasesForgejo,
            PluginType::DiscoveryProxmoxHelperScripts,
            PluginType::ReleasesDocker,
            PluginType::PackageManagerHomebrew,
            PluginType::PackageManagerApt,
            PluginType::PackageManagerNpm,
            PluginType::PackageManagerMas,
            PluginType::PackageManagerPacman,
            PluginType::PackageManagerPkg,
            PluginType::PackageManagerApk,
            PluginType::PackageManagerSnap,
            PluginType::PackageManagerCargo,
            PluginType::GenericShell,
            PluginType::InfrastructureProxmox,
            PluginType::HookSystemd,
            PluginType::HookShell,
            PluginType::EnhancementDashboardIcons,
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
            PluginType::ReleasesGitlab,
            PluginType::ReleasesForgejo,
            PluginType::DiscoveryProxmoxHelperScripts,
            PluginType::ReleasesDocker,
            PluginType::PackageManagerHomebrew,
            PluginType::PackageManagerApt,
            PluginType::PackageManagerNpm,
            PluginType::PackageManagerMas,
            PluginType::PackageManagerPacman,
            PluginType::PackageManagerPkg,
            PluginType::PackageManagerApk,
            PluginType::PackageManagerSnap,
            PluginType::PackageManagerCargo,
            PluginType::GenericShell,
            PluginType::InfrastructureProxmox,
            PluginType::HookSystemd,
            PluginType::HookShell,
            PluginType::EnhancementDashboardIcons,
            PluginType::Other("my_plugin".to_string()),
        ];
        for pt in &variants {
            assert_eq!(pt.as_str(), pt.to_string());
        }
    }

    #[test]
    fn attestation_status_roundtrip() {
        for (status, expected) in [
            (AttestationStatus::Verified, r#""Verified""#),
            (AttestationStatus::NotFound, r#""NotFound""#),
            (AttestationStatus::Unverified, r#""Unverified""#),
        ] {
            let json = serde_json::to_string(&status).expect("serialize");
            assert_eq!(json, expected);
            let deserialized: AttestationStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(deserialized, status);
        }
    }

    #[test]
    fn attestation_status_unknown_deserializes_to_other() {
        let deserialized: AttestationStatus =
            serde_json::from_str(r#""Pending""#).expect("deserialize unknown");
        assert_eq!(
            deserialized,
            AttestationStatus::Other("Pending".to_string())
        );
    }

    #[test]
    fn attestation_status_other_serializes_to_inner_string() {
        let status = AttestationStatus::Other("Pending".to_string());
        let json = serde_json::to_string(&status).expect("serialize");
        assert_eq!(json, r#""Pending""#);
    }

    #[test]
    fn release_asset_serialization_roundtrip() {
        let asset = ReleaseAsset {
            name: "app-linux-amd64.tar.gz".to_string(),
            download_url: "https://example.com/download".to_string(),
            size: Some(12345),
            content_type: Some("application/gzip".to_string()),
            sha256_digest: Some("a".repeat(64)),
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
            sha256_digest: None,
        };
        let json = serde_json::to_string(&asset).expect("serialize");
        assert!(!json.contains("size"));
        assert!(!json.contains("content_type"));
        assert!(!json.contains("sha256_digest"));
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
                sha256_digest: None,
            }],
            attestation_status: Some(AttestationStatus::Verified),
            require_attestation: true,
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
            attestation_status: None,
            require_attestation: false,
        };
        let json = serde_json::to_string(&info).expect("serialize");
        assert!(!json.contains("assets"));
        assert!(!json.contains("attestation_status"));
        let deserialized: ReleaseInfo = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, info);
    }

    #[test]
    fn release_info_defaults_on_deserialize() {
        // Old wire messages without attestation fields must deserialize cleanly.
        let json = r#"{"tag":"v1.0.0","release_url":"https://example.com"}"#;
        let info: ReleaseInfo = serde_json::from_str(json).expect("deserialize");
        assert_eq!(info.tag, "v1.0.0");
        assert!(info.attestation_status.is_none());
        assert!(!info.require_attestation);
    }
}
