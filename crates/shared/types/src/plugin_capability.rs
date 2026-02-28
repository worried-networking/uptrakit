use serde::{Deserialize, Serialize};

/// Capabilities that a plugin may support.
///
/// # Design note
///
/// This is a closed, centralized enum rather than a trait-based capability system.
/// All plugins in this project are first-party and registered exclusively through
/// `uptrakit-plugin-registry` (see AGENTS.md: "adding a new plugin only through
/// the registry is an acceptable tradeoff"). `#[non_exhaustive]` allows adding new
/// variants in future releases without requiring downstream match-arm updates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum PluginCapability {
    /// Plugin can discover locally installed software.
    DiscoverLocalSoftware,
    /// Plugin can refresh/sync the local package index from remote sources.
    RefreshPackageIndex,
    /// Plugin can determine whether it is applicable to the current host.
    DetectHostCompatibility,
    /// Plugin can run logic before an update is applied.
    PreUpdateHook,
    /// Plugin can run logic after an update is applied.
    PostUpdateHook,
    /// Plugin's `fetch_releases()` does not require any local system state
    /// (no package index, no filesystem access, no local commands) and can
    /// be called from the controller process directly rather than through
    /// an agent.
    ///
    /// All `fetch_releases` calls default to agent-side. This capability is
    /// an explicit opt-in to controller-side execution. The user can override
    /// via `execution_site` on the plugin assignment.
    ControllerSideFetchReleases,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_capability_serialization_roundtrip() {
        let cap = PluginCapability::DiscoverLocalSoftware;
        let json = serde_json::to_string(&cap).expect("serialize");
        assert_eq!(json, r#""discover_local_software""#);
        let de: PluginCapability = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(de, cap);
    }

    #[test]
    fn plugin_capability_all_variants_snake_case() {
        let cases = [
            (PluginCapability::DiscoverLocalSoftware, "discover_local_software"),
            (PluginCapability::RefreshPackageIndex, "refresh_package_index"),
            (
                PluginCapability::DetectHostCompatibility,
                "detect_host_compatibility",
            ),
            (PluginCapability::PreUpdateHook, "pre_update_hook"),
            (PluginCapability::PostUpdateHook, "post_update_hook"),
            (
                PluginCapability::ControllerSideFetchReleases,
                "controller_side_fetch_releases",
            ),
        ];
        for (cap, expected) in cases {
            let json = serde_json::to_string(&cap).expect("serialize");
            assert_eq!(json, format!("\"{expected}\""));
        }
    }
}
