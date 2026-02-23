use serde::{Deserialize, Serialize};

/// A piece of software discovered on the local system by a provider.
///
/// `installed_version` is required — providers that cannot determine a version
/// must omit the item from results entirely.
///
/// This type is the canonical shared definition used in both the agent/provider
/// layer and the wire protocol. The `uptrakit-provider-core` crate re-exports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DiscoveredSoftware {
    /// Provider-specific identifier for this software (e.g., package name, app slug).
    pub package_identifier: String,
    /// Human-readable display name.
    pub name: String,
    /// Currently installed version (required; providers omit items with unknown versions).
    pub installed_version: String,
    /// Additional provider-specific metadata (e.g. `{"package_type": "formula"}`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialization_roundtrip() {
        let sw = DiscoveredSoftware {
            package_identifier: "wget".to_string(),
            name: "Wget".to_string(),
            installed_version: "1.21.3".to_string(),
            extra: Some(serde_json::json!({"package_type": "formula"})),
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }

    #[test]
    fn optional_extra_omitted() {
        let sw = DiscoveredSoftware {
            package_identifier: "curl".to_string(),
            name: "cURL".to_string(),
            installed_version: "8.4.0".to_string(),
            extra: None,
        };
        let json = serde_json::to_string(&sw).expect("serialize");
        assert!(!json.contains("extra"));
        let deserialized: DiscoveredSoftware = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, sw);
    }
}
