use serde::{Deserialize, Serialize};

use crate::version::Version;

/// A single item in a batch detect-installed-version request.
///
/// Represents one package whose installed version should be detected within a
/// batch operation (e.g., one of 50 APT packages queried via a single
/// `dpkg-query` invocation).
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchDetectItem {
    /// Plugin-specific identifier for the package (e.g., APT package name).
    pub package_identifier: String,
}

impl BatchDetectItem {
    /// Create a new [`BatchDetectItem`] for the given package identifier.
    pub fn new(package_identifier: impl Into<String>) -> Self {
        Self {
            package_identifier: package_identifier.into(),
        }
    }
}

/// Result of detecting the installed version of a single package within a batch
/// operation.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchDetectResult {
    /// Plugin-specific identifier for the package.
    pub package_identifier: String,
    /// Detected installed version, or `None` if the package is not installed.
    ///
    /// When both `installed_version` and `error` are `None`, the package is
    /// confirmed not installed (the query succeeded but found nothing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<Version>,
    /// Error message if detection failed for this specific package.
    ///
    /// `None` indicates success (even if `installed_version` is also `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Human-readable installed version for display when `installed_version`
    /// is opaque (e.g. a Docker SHA256 digest → image publish date).
    /// Propagated from the plugin through the agent wire type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_version: Option<String>,
}

impl BatchDetectResult {
    /// Create a new [`BatchDetectResult`] with explicit fields.
    pub fn new(
        package_identifier: impl Into<String>,
        installed_version: Option<Version>,
        error: Option<String>,
    ) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            installed_version,
            error,
            display_version: None,
        }
    }

    /// Create a successful detect result with the given installed version.
    pub fn found(package_identifier: impl Into<String>, version: Version) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            installed_version: Some(version),
            error: None,
            display_version: None,
        }
    }

    /// Create a result indicating the package is not installed.
    pub fn not_found(package_identifier: impl Into<String>) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            installed_version: None,
            error: None,
            display_version: None,
        }
    }

    /// Create a result indicating an error occurred during detection.
    pub fn error(package_identifier: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            package_identifier: package_identifier.into(),
            installed_version: None,
            error: Some(error.into()),
            display_version: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_detect_item_serialization_roundtrip() {
        let item = BatchDetectItem {
            package_identifier: "nginx".to_string(),
        };
        let json = serde_json::to_string(&item).expect("serialize");
        let deserialized: BatchDetectItem = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, item);
    }

    #[test]
    fn batch_detect_result_installed_serialization_roundtrip() {
        let result = BatchDetectResult {
            package_identifier: "nginx".to_string(),
            installed_version: Some(Version::new("1.24.0")),
            error: None,
            display_version: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: BatchDetectResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, result);
    }

    #[test]
    fn batch_detect_result_not_installed_omits_optional_fields() {
        let result = BatchDetectResult {
            package_identifier: "curl".to_string(),
            installed_version: None,
            error: None,
            display_version: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(!json.contains("installed_version"));
        assert!(!json.contains("error"));
        assert!(!json.contains("display_version"));
    }

    #[test]
    fn batch_detect_result_error_omits_version() {
        let result = BatchDetectResult {
            package_identifier: "broken".to_string(),
            installed_version: None,
            error: Some("dpkg-query failed".to_string()),
            display_version: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(!json.contains("installed_version"));
        assert!(json.contains("dpkg-query failed"));
    }
}
