use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::host_feature::HostFeature;
use crate::os_family::OsFamily;

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
    /// Unknown feature strings are silently dropped — they cannot match any
    /// `HostRequirements::required_features` entry, so they don't affect validation.
    /// This is intentional for forward-compatibility: a newer agent can report
    /// features the controller doesn't know about yet.
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
                .filter_map(|s| {
                    serde_json::from_value::<HostFeature>(serde_json::Value::String(s.clone())).ok()
                })
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

    pub fn has_feature(&self, feature: HostFeature) -> bool {
        self.features.contains(&feature)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_parses_known_features() {
        let caps = HostCapabilities::new(
            Some("linux"),
            Some("Ubuntu 24.04"),
            Some("x86_64"),
            &[
                "posix_shell".to_string(),
                "privilege_escalation".to_string(),
                "systemd".to_string(),
            ],
        );
        assert_eq!(caps.os_family, Some(OsFamily::Linux));
        assert_eq!(caps.os_version.as_deref(), Some("Ubuntu 24.04"));
        assert_eq!(caps.architecture.as_deref(), Some("x86_64"));
        assert!(caps.has_feature(HostFeature::PosixShell));
        assert!(caps.has_feature(HostFeature::PrivilegeEscalation));
        assert!(caps.has_feature(HostFeature::Systemd));
        assert!(!caps.has_feature(HostFeature::RouterOsCli));
    }

    #[test]
    fn unknown_features_silently_dropped() {
        let caps = HostCapabilities::new(
            Some("linux"),
            None,
            None,
            &["posix_shell".to_string(), "unknown_feature".to_string()],
        );
        assert_eq!(caps.features.len(), 1);
        assert!(caps.has_feature(HostFeature::PosixShell));
    }

    #[test]
    fn empty_features_for_legacy_agent() {
        let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
        assert!(caps.features.is_empty());
    }

    #[test]
    fn from_json_features_parses_array() {
        let caps = HostCapabilities::from_json_features(
            Some("macos"),
            None,
            Some("aarch64"),
            Some(r#"["posix_shell","privilege_escalation"]"#),
        );
        assert_eq!(caps.os_family, Some(OsFamily::MacOs));
        assert_eq!(caps.features.len(), 2);
    }

    #[test]
    fn from_json_features_handles_none() {
        let caps = HostCapabilities::from_json_features(Some("linux"), None, None, None);
        assert!(caps.features.is_empty());
    }

    #[test]
    fn from_json_features_handles_invalid_json() {
        let caps =
            HostCapabilities::from_json_features(Some("linux"), None, None, Some("not-json"));
        assert!(caps.features.is_empty());
    }

    #[test]
    fn unknown_os_type_yields_none() {
        let caps = HostCapabilities::new(Some("haiku"), None, None, &[]);
        assert_eq!(caps.os_family, None);
    }

    #[test]
    fn default_has_no_features() {
        let caps = HostCapabilities::default();
        assert!(caps.os_family.is_none());
        assert!(caps.features.is_empty());
    }
}
