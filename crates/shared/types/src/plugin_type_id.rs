use std::borrow::Cow;
use std::fmt;

use serde::{Deserialize, Serialize};

/// Opaque plugin type identifier — validated at the catalog boundary.
///
/// Uses `Cow<'static, str>` so well-known constants are zero-allocation borrows
/// while DB/wire values are owned strings. Both are the same type.
///
/// This replaces `PluginType` enum. Instead of matching on variants, code looks up
/// the `PluginTypeId` in the `PluginCatalog` to get a `PluginDescriptor`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginTypeId(Cow<'static, str>);

impl PluginTypeId {
    /// Const constructor for well-known identifiers. Zero allocation.
    pub const fn from_static(s: &'static str) -> Self {
        Self(Cow::Borrowed(s))
    }

    /// Runtime constructor for DB/wire values. Allocates.
    pub fn new(s: impl Into<String>) -> Self {
        Self(Cow::Owned(s.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginTypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for PluginTypeId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for PluginTypeId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl AsRef<str> for PluginTypeId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for PluginTypeId {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for PluginTypeId {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// Well-known plugin type identifiers as typed constants.
/// Use these directly in catalog lookups — no wrapping needed.
pub mod plugin_ids {
    use super::PluginTypeId;

    pub const RELEASES_GITHUB: PluginTypeId = PluginTypeId::from_static("releases_github");
    pub const RELEASES_GITLAB: PluginTypeId = PluginTypeId::from_static("releases_gitlab");
    pub const RELEASES_FORGEJO: PluginTypeId = PluginTypeId::from_static("releases_forgejo");
    pub const RELEASES_DOCKER: PluginTypeId = PluginTypeId::from_static("releases_docker");
    pub const DISCOVERY_PROXMOX_HELPER_SCRIPTS: PluginTypeId =
        PluginTypeId::from_static("discovery_proxmox_helper_scripts");
    pub const PACKAGE_MANAGER_APT: PluginTypeId = PluginTypeId::from_static("package_manager_apt");
    pub const PACKAGE_MANAGER_HOMEBREW: PluginTypeId =
        PluginTypeId::from_static("package_manager_homebrew");
    pub const PACKAGE_MANAGER_DNF: PluginTypeId = PluginTypeId::from_static("package_manager_dnf");
    pub const PACKAGE_MANAGER_NPM: PluginTypeId = PluginTypeId::from_static("package_manager_npm");
    pub const PACKAGE_MANAGER_MAS: PluginTypeId = PluginTypeId::from_static("package_manager_mas");
    pub const PACKAGE_MANAGER_PACMAN: PluginTypeId =
        PluginTypeId::from_static("package_manager_pacman");
    pub const PACKAGE_MANAGER_PKG: PluginTypeId = PluginTypeId::from_static("package_manager_pkg");
    pub const PACKAGE_MANAGER_APK: PluginTypeId = PluginTypeId::from_static("package_manager_apk");
    pub const PACKAGE_MANAGER_SNAP: PluginTypeId =
        PluginTypeId::from_static("package_manager_snap");
    pub const PACKAGE_MANAGER_CARGO: PluginTypeId =
        PluginTypeId::from_static("package_manager_cargo");
    pub const GENERIC_SHELL: PluginTypeId = PluginTypeId::from_static("generic_shell");
    pub const HOOK_SHELL: PluginTypeId = PluginTypeId::from_static("hook_shell");
    pub const HOOK_SYSTEMD: PluginTypeId = PluginTypeId::from_static("hook_systemd");
    pub const INFRASTRUCTURE_PROXMOX: PluginTypeId =
        PluginTypeId::from_static("infrastructure_proxmox");
    pub const WEBHOOK: PluginTypeId = PluginTypeId::from_static("webhook");
    pub const TELEGRAM: PluginTypeId = PluginTypeId::from_static("telegram");
    pub const EMAIL: PluginTypeId = PluginTypeId::from_static("email");
    pub const ENHANCEMENT_DASHBOARD_ICONS: PluginTypeId =
        PluginTypeId::from_static("enhancement_dashboard_icons");

    /// All well-known plugin type IDs. Must include every constant above.
    /// Tests verify bidirectional consistency with `all_descriptors()`.
    pub const ALL: &[PluginTypeId] = &[
        RELEASES_GITHUB,
        RELEASES_GITLAB,
        RELEASES_FORGEJO,
        RELEASES_DOCKER,
        DISCOVERY_PROXMOX_HELPER_SCRIPTS,
        PACKAGE_MANAGER_APT,
        PACKAGE_MANAGER_HOMEBREW,
        PACKAGE_MANAGER_DNF,
        PACKAGE_MANAGER_NPM,
        PACKAGE_MANAGER_MAS,
        PACKAGE_MANAGER_PACMAN,
        PACKAGE_MANAGER_PKG,
        PACKAGE_MANAGER_APK,
        PACKAGE_MANAGER_SNAP,
        PACKAGE_MANAGER_CARGO,
        GENERIC_SHELL,
        HOOK_SHELL,
        HOOK_SYSTEMD,
        INFRASTRUCTURE_PROXMOX,
        WEBHOOK,
        TELEGRAM,
        EMAIL,
        ENHANCEMENT_DASHBOARD_ICONS,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_static_is_zero_alloc() {
        let id = PluginTypeId::from_static("package_manager_apt");
        assert_eq!(id.as_str(), "package_manager_apt");
        // Cow::Borrowed — no heap allocation
        assert!(matches!(id.0, Cow::Borrowed(_)));
    }

    #[test]
    fn new_allocates() {
        let id = PluginTypeId::new("custom_plugin");
        assert_eq!(id.as_str(), "custom_plugin");
        assert!(matches!(id.0, Cow::Owned(_)));
    }

    #[test]
    fn from_string() {
        let id = PluginTypeId::from("custom".to_string());
        assert_eq!(id.as_str(), "custom");
    }

    #[test]
    fn display() {
        let id = PluginTypeId::from_static("releases_github");
        assert_eq!(id.to_string(), "releases_github");
    }

    #[test]
    fn serde_roundtrip() {
        let id = PluginTypeId::from_static("package_manager_apt");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, r#""package_manager_apt""#);
        let de: PluginTypeId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de, id);
    }

    #[test]
    fn serde_transparent_accepts_any_string() {
        let de: PluginTypeId = serde_json::from_str(r#""unknown_plugin""#).expect("deserialize");
        assert_eq!(de.as_str(), "unknown_plugin");
    }

    #[test]
    fn equality_static_vs_owned() {
        let static_id = PluginTypeId::from_static("releases_github");
        let owned_id = PluginTypeId::new("releases_github");
        assert_eq!(static_id, owned_id);
    }

    #[test]
    fn equality_with_str() {
        let id = PluginTypeId::from_static("releases_github");
        assert_eq!(id, "releases_github");
        assert_eq!(id, *"releases_github");
    }

    #[test]
    fn all_constants_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for id in plugin_ids::ALL {
            assert!(
                seen.insert(id.as_str()),
                "duplicate plugin_ids constant: {}",
                id.as_str()
            );
        }
    }

    #[test]
    fn all_constants_count() {
        // Update this if you add a new well-known constant.
        assert_eq!(plugin_ids::ALL.len(), 23);
    }

    #[test]
    fn hash_equality() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(PluginTypeId::from_static("releases_github"), "found");
        // Lookup with an owned key matches the static key.
        assert_eq!(
            map.get(&PluginTypeId::new("releases_github")),
            Some(&"found")
        );
    }
}
