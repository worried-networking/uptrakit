use crate::generated::shared_types::host_feature::HostFeature;
use crate::generated::shared_types::os_family::OsFamily;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
/// Runtime description of a host's execution environment.
///
/// `os_family` is derived from the existing `host.os_type` DB string.
/// `features` are **agent-reported** — the agent probes the host at bootstrap
/// and reports detected features. No heuristic inference from OS type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HostCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_family: Option<OsFamily>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// Agent-probed feature flags. Empty = agent hasn't reported (legacy agent).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub features: BTreeSet<HostFeature>,
}
impl HostCapabilities {
    /// Single canonical constructor from raw host fields + feature strings.
    ///
    /// Used at both:
    /// 1. **DB read boundary** — `web-api-queries` extracts fields from `host::Model`
    ///    and passes `host.host_features` parsed as `Vec<String>` (or empty).
    /// 2. **Agent side** — agent probes features locally / over SSH, yielding
    ///    `Vec<String>`, and passes them alongside `host_info` fields.
    ///
    /// All feature strings are accepted — unknown features from newer agents
    /// are stored losslessly. They won't match any well-known constant in
    /// `HostRequirements::required_features`, so they don't affect validation
    /// but are preserved for forward-compatibility.
    pub fn new(
        os_type: Option<&str>,
        os_version: Option<&str>,
        architecture: Option<&str>,
        feature_strings: &[String],
    ) -> Self {
        Self {
            os_family: os_type.and_then(OsFamily::from_os_type),
            os_version: os_version.map(String::from),
            architecture: architecture.map(String::from),
            features: feature_strings
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| HostFeature::new(s.clone()))
                .collect(),
        }
    }
    /// Convenience for DB read path where features are JSON-encoded.
    ///
    /// Parses `features_json` (a JSON `["posix_shell","systemd"]` string)
    /// into `Vec<String>` and delegates to `Self::new()`. If `features_json`
    /// is `None` or unparseable, passes an empty slice (legacy agent).
    pub fn from_json_features(
        os_type: Option<&str>,
        os_version: Option<&str>,
        architecture: Option<&str>,
        features_json: Option<&str>,
    ) -> Self {
        let strings: Vec<String> = features_json
            .and_then(|json| serde_json::from_str(json).ok())
            .unwrap_or_default();
        Self::new(os_type, os_version, architecture, &strings)
    }
    pub fn has_feature(&self, feature: impl std::borrow::Borrow<HostFeature>) -> bool {
        self.features.contains(feature.borrow())
    }
}
