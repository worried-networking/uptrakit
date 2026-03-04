use serde::{Deserialize, Serialize};

use crate::types::UpstreamRelease;

/// A single item in a batch fetch-releases request.
///
/// Represents one package whose upstream releases should be fetched within a
/// batch operation (e.g., one of 50 APT packages queried via a single
/// `apt-cache madison` invocation).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchFetchItem {
    /// Plugin-specific identifier for the package (e.g., APT package name).
    pub package_identifier: String,
}

/// Result of fetching releases for a single package within a batch operation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BatchFetchResult {
    /// Plugin-specific identifier for the package.
    pub package_identifier: String,
    /// Available upstream releases for the package (may be empty if the package
    /// is not found in any configured repository or registry).
    pub releases: Vec<UpstreamRelease>,
    /// Error message if the fetch failed for this specific package.
    ///
    /// `None` indicates success (even if `releases` is also empty).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::version::Version;

    #[test]
    fn batch_fetch_item_serialization_roundtrip() {
        let item = BatchFetchItem {
            package_identifier: "nginx".to_string(),
        };
        let json = serde_json::to_string(&item).expect("serialize");
        let deserialized: BatchFetchItem = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized, item);
    }

    #[test]
    fn batch_fetch_result_with_releases_roundtrip() {
        let result = BatchFetchResult {
            package_identifier: "nginx".to_string(),
            releases: vec![UpstreamRelease {
                version: Version::new("1.24.0"),
                tag: "1.24.0".to_string(),
                is_prerelease: false,
                release_url: String::new(),
                release_notes: None,
                published_at: None,
                assets: vec![],
                category: None,
                attestation_status: None,
            }],
            error: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        let deserialized: BatchFetchResult = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.package_identifier, result.package_identifier);
        assert_eq!(deserialized.releases.len(), 1);
        assert!(deserialized.error.is_none());
    }

    #[test]
    fn batch_fetch_result_empty_releases_omits_error() {
        let result = BatchFetchResult {
            package_identifier: "unknown-pkg".to_string(),
            releases: vec![],
            error: None,
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(!json.contains("error"));
    }

    #[test]
    fn batch_fetch_result_error_has_empty_releases() {
        let result = BatchFetchResult {
            package_identifier: "broken-pkg".to_string(),
            releases: vec![],
            error: Some("apt-cache madison failed".to_string()),
        };
        assert!(result.releases.is_empty());
        assert!(result.error.is_some());
    }
}
