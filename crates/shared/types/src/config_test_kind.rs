use serde::{Deserialize, Serialize};

// WIRE TYPE — used in TestPluginConfigPayload (uptrakit-wire); must follow the
// Other(String) catch-all pattern before new variants are added (see coding-standards.md).
/// The kind of configuration test to perform on the agent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTestKind {
    /// Execute `detect_installed_version()` and return output + detected version.
    VersionDetection,
    /// Validate update_command syntax (sh -n check, do NOT execute).
    UpdateCommandValidation,
    /// Execute pre-update hook with mock context.
    PreUpdateHook,
    /// Execute post-update hook with mock context.
    PostUpdateHook,
    /// Test connectivity for controller-side plugins (`fetch_releases`).
    Connectivity,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_detection_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::VersionDetection).unwrap();
        assert_eq!(json, r#""version_detection""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::VersionDetection);
    }

    #[test]
    fn update_command_validation_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::UpdateCommandValidation).unwrap();
        assert_eq!(json, r#""update_command_validation""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::UpdateCommandValidation);
    }

    #[test]
    fn pre_update_hook_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::PreUpdateHook).unwrap();
        assert_eq!(json, r#""pre_update_hook""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::PreUpdateHook);
    }

    #[test]
    fn post_update_hook_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::PostUpdateHook).unwrap();
        assert_eq!(json, r#""post_update_hook""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::PostUpdateHook);
    }

    #[test]
    fn connectivity_roundtrip() {
        let json = serde_json::to_string(&ConfigTestKind::Connectivity).unwrap();
        assert_eq!(json, r#""connectivity""#);
        let back: ConfigTestKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ConfigTestKind::Connectivity);
    }
}
