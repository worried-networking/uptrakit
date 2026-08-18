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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
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

impl std::str::FromStr for PluginTypeId {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

/// Derives the notification plugin's type ID from its `channel_type`
/// string (channel identity stays `"email"`/`"telegram"`/`"webhook"` —
/// a separate, runtime-validated concept; plugin type IDs are namespaced).
///
/// Deliberately defined outside the [`plugin_ids`] module: consumer code
/// (web-api, controller-core, surface-proxy, …) must not reference
/// `plugin_ids::` directly (enforced by `ci/check_plugin_semantic_boundary.py`),
/// but deriving a notification plugin's identity from its channel type is a
/// cross-cutting concern those crates legitimately need.
pub fn notification_plugin_type(channel_type: &str) -> PluginTypeId {
    PluginTypeId::new(format!("notifications.{channel_type}"))
}

/// Well-known plugin type identifiers as typed constants.
/// Use these directly in catalog lookups — no wrapping needed.
pub mod plugin_ids {
    use super::PluginTypeId;

    pub const RELEASES_GITHUB: PluginTypeId = PluginTypeId::from_static("releases.github");
    pub const RELEASES_GITLAB: PluginTypeId = PluginTypeId::from_static("releases.gitlab");
    pub const RELEASES_FORGEJO: PluginTypeId = PluginTypeId::from_static("releases.forgejo");
    pub const RELEASES_DOCKER: PluginTypeId = PluginTypeId::from_static("releases.docker");
    pub const DISCOVERY_PROXMOX_HELPER_SCRIPTS: PluginTypeId =
        PluginTypeId::from_static("discovery.proxmox-helper-scripts");
    pub const DISCOVERY_UPTRAKIT_SELF_UPDATE: PluginTypeId =
        PluginTypeId::from_static("discovery.uptrakit-self-update");
    pub const PACKAGE_MANAGER_APT: PluginTypeId = PluginTypeId::from_static("package-manager.apt");
    pub const PACKAGE_MANAGER_HOMEBREW: PluginTypeId =
        PluginTypeId::from_static("package-manager.homebrew");
    pub const PACKAGE_MANAGER_DNF: PluginTypeId = PluginTypeId::from_static("package-manager.dnf");
    pub const PACKAGE_MANAGER_NPM: PluginTypeId = PluginTypeId::from_static("package-manager.npm");
    pub const PACKAGE_MANAGER_MAS: PluginTypeId = PluginTypeId::from_static("package-manager.mas");
    pub const PACKAGE_MANAGER_PACMAN: PluginTypeId =
        PluginTypeId::from_static("package-manager.pacman");
    pub const PACKAGE_MANAGER_PKG: PluginTypeId = PluginTypeId::from_static("package-manager.pkg");
    pub const PACKAGE_MANAGER_APK: PluginTypeId = PluginTypeId::from_static("package-manager.apk");
    pub const PACKAGE_MANAGER_SNAP: PluginTypeId =
        PluginTypeId::from_static("package-manager.snap");
    pub const PACKAGE_MANAGER_CARGO: PluginTypeId =
        PluginTypeId::from_static("package-manager.cargo");
    pub const PACKAGE_MANAGER_ROUTEROS: PluginTypeId =
        PluginTypeId::from_static("package-manager.routeros");
    pub const PACKAGE_MANAGER_SKILLS: PluginTypeId =
        PluginTypeId::from_static("package-manager.skills");
    // TODO(uv-plan-3): not yet in the registry's all_descriptors() — the uv
    // series' final plan activates it (no CI gate asserts ALL ⊆ descriptors,
    // so this breadcrumb is the only in-code signal until then).
    pub const PACKAGE_MANAGER_UV: PluginTypeId = PluginTypeId::from_static("package-manager.uv");
    pub const GENERIC_SHELL: PluginTypeId = PluginTypeId::from_static("generic.shell");
    pub const HOOK_SHELL: PluginTypeId = PluginTypeId::from_static("hook.shell");
    pub const HOOK_SYSTEMD: PluginTypeId = PluginTypeId::from_static("hook.systemd");
    pub const INFRASTRUCTURE_PROXMOX: PluginTypeId =
        PluginTypeId::from_static("infrastructure.proxmox");
    pub const WEBHOOK: PluginTypeId = PluginTypeId::from_static("notifications.webhook");
    pub const TELEGRAM: PluginTypeId = PluginTypeId::from_static("notifications.telegram");
    pub const EMAIL: PluginTypeId = PluginTypeId::from_static("notifications.email");
    pub const ENHANCEMENT_DASHBOARD_ICONS: PluginTypeId =
        PluginTypeId::from_static("enhancement.dashboard-icons");
    #[cfg(feature = "test-support")]
    pub const TEST_FETCH_FAIL: PluginTypeId = PluginTypeId::from_static("test.fetch-fail");
    #[cfg(feature = "test-support")]
    pub const TEST_PER_ITEM_FAIL: PluginTypeId = PluginTypeId::from_static("test.per-item-fail");
    #[cfg(feature = "test-support")]
    pub const TEST_CTX_CAPTURE: PluginTypeId = PluginTypeId::from_static("test.ctx-capture");
    #[cfg(feature = "test-support")]
    pub const TEST_ENRICHER_ECHO: PluginTypeId = PluginTypeId::from_static("test.enricher-echo");
    #[cfg(feature = "test-support")]
    pub const TEST_ENRICHER_MISS: PluginTypeId = PluginTypeId::from_static("test.enricher-miss");
    #[cfg(feature = "test-support")]
    pub const TEST_LIFECYCLE_HOOK: PluginTypeId = PluginTypeId::from_static("test.lifecycle-hook");

    /// All well-known plugin type IDs. Must include every constant above.
    /// Tests verify bidirectional consistency with `all_descriptors()`.
    pub const ALL: &[PluginTypeId] = &[
        RELEASES_GITHUB,
        RELEASES_GITLAB,
        RELEASES_FORGEJO,
        RELEASES_DOCKER,
        DISCOVERY_PROXMOX_HELPER_SCRIPTS,
        DISCOVERY_UPTRAKIT_SELF_UPDATE,
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
        PACKAGE_MANAGER_ROUTEROS,
        PACKAGE_MANAGER_SKILLS,
        PACKAGE_MANAGER_UV,
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
        let id = PluginTypeId::from_static("package-manager.apt");
        assert_eq!(id.as_str(), "package-manager.apt");
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
        let id = PluginTypeId::from_static("releases.github");
        assert_eq!(id.to_string(), "releases.github");
    }

    #[test]
    fn serde_roundtrip() {
        let id = PluginTypeId::from_static("package-manager.apt");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, r#""package-manager.apt""#);
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
        let static_id = PluginTypeId::from_static("releases.github");
        let owned_id = PluginTypeId::new("releases.github");
        assert_eq!(static_id, owned_id);
    }

    #[test]
    fn equality_with_str() {
        let id = PluginTypeId::from_static("releases.github");
        assert_eq!(id, "releases.github");
        assert_eq!(id, *"releases.github");
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
    fn package_manager_skills_constant_is_correct() {
        assert_eq!(
            plugin_ids::PACKAGE_MANAGER_SKILLS.as_str(),
            "package-manager.skills"
        );
    }

    #[test]
    fn all_constants_count() {
        // Update this if you add a new well-known constant.
        assert_eq!(plugin_ids::ALL.len(), 27);
    }

    #[test]
    fn hash_equality() {
        use std::collections::HashMap;
        let mut map = HashMap::new();
        map.insert(PluginTypeId::from_static("releases.github"), "found");
        // Lookup with an owned key matches the static key.
        assert_eq!(
            map.get(&PluginTypeId::new("releases.github")),
            Some(&"found")
        );
    }

    #[test]
    fn notification_plugin_type_derives_namespaced_id() {
        assert_eq!(notification_plugin_type("email"), plugin_ids::EMAIL);
        assert_eq!(notification_plugin_type("telegram"), plugin_ids::TELEGRAM);
        assert_eq!(notification_plugin_type("webhook"), plugin_ids::WEBHOOK);
    }
}
