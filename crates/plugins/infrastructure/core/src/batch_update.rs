use serde::{Deserialize, Serialize};

use crate::types::ReleaseInfo;

/// A single item in a batch update request.
///
/// Represents one package to update within a batch operation (e.g., one
/// of 50 APT packages to upgrade in a single `apt-get upgrade` command).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchUpdateItem {
    /// Plugin-specific identifier for the package (e.g., APT package name).
    pub package_identifier: String,
    /// Target version to install.
    pub to_version: String,
    /// Optional release metadata from the upstream source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_info: Option<ReleaseInfo>,
}

/// Result of updating a single package within a batch operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchUpdateResult {
    /// Plugin-specific identifier for the package.
    pub package_identifier: String,
    /// Whether the update succeeded.
    pub success: bool,
    /// Output from the update operation (may be shared across all packages
    /// in the batch if the package manager uses a single command).
    pub output: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_update_item_serialization_roundtrip() {
        let item = BatchUpdateItem {
            package_identifier: "nginx".to_string(),
            to_version: "1.24.0-2".to_string(),
            release_info: None,
        };
        let json = serde_json::to_string(&item).expect("serialize");
        let deserialized: BatchUpdateItem = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, item);
    }

    #[test]
    fn batch_update_item_release_info_omitted_when_none() {
        let item = BatchUpdateItem {
            package_identifier: "curl".to_string(),
            to_version: "8.5.0".to_string(),
            release_info: None,
        };
        let json = serde_json::to_string(&item).expect("serialize");
        assert!(!json.contains("release_info"));
    }

    #[test]
    fn batch_update_result_serialization_roundtrip() {
        let result = BatchUpdateResult {
            package_identifier: "nginx".to_string(),
            success: true,
            output: "Setting up nginx (1.24.0-2) ...".to_string(),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: BatchUpdateResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, result);
    }

    #[test]
    fn batch_update_result_failure() {
        let result = BatchUpdateResult {
            package_identifier: "broken-pkg".to_string(),
            success: false,
            output: "E: Unable to locate package broken-pkg".to_string(),
        };
        assert!(!result.success);
    }
}
