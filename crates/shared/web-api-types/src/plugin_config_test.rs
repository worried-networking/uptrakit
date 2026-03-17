//! Request and response types for plugin configuration testing (dry-run).

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

/// Request body for `POST /api/v1/plugin-configs/test`.
///
/// Tests a plugin configuration without saving it. The user can submit an
/// ad-hoc plugin type + config JSON for testing before creating a plugin
/// config record.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TestPluginConfigRequest {
    /// The plugin type to test (e.g. `"generic_shell"`, `"releases_github"`).
    pub plugin_type: String,
    /// The plugin configuration JSON to test.
    pub config: serde_json::Value,
    /// Optional saved config ID. When provided, the saved config is loaded
    /// and the incoming `config` is shallow-merged on top (same merge
    /// semantics as the three-layer config model). This lets the user test
    /// partial edits to an existing config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_config_id: Option<Uuid>,
    /// Host ID to test against (required for agent-side tests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_id: Option<Uuid>,
    /// What to test. If omitted, auto-detected from plugin capabilities:
    /// - `"connectivity"` for controller-side plugins
    /// - `"version_detection"` for agent-side plugins with VersionDetection
    /// - `"update_command_validation"` for agent-side plugins with UpdateExecution
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_kind: Option<String>,
    /// Package identifier for testing (e.g. `"nginx"`, `"owner/repo"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_identifier: Option<String>,
}

/// Response for plugin config test.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TestPluginConfigResponse {
    /// Whether the test passed.
    pub success: bool,
    /// The kind of test that was executed.
    pub test_kind: String,
    /// Command output or connectivity response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Error message if the test failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Detected version (for version detection tests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_version: Option<String>,
    /// Test duration in milliseconds.
    pub duration_ms: u64,
}

impl Validate for TestPluginConfigRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if self.plugin_type.trim().is_empty() {
            return Err(ValidationError {
                field: "plugin_type",
                message: "plugin_type must not be empty".to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_empty_plugin_type() {
        let req = TestPluginConfigRequest {
            plugin_type: "".to_string(),
            config: serde_json::json!({}),
            plugin_config_id: None,
            host_id: None,
            test_kind: None,
            package_identifier: None,
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "plugin_type");
    }

    #[test]
    fn validate_whitespace_plugin_type() {
        let req = TestPluginConfigRequest {
            plugin_type: "  ".to_string(),
            config: serde_json::json!({}),
            plugin_config_id: None,
            host_id: None,
            test_kind: None,
            package_identifier: None,
        };
        assert!(req.validate().is_err());
    }

    #[test]
    fn validate_valid_request() {
        let req = TestPluginConfigRequest {
            plugin_type: "generic_shell".to_string(),
            config: serde_json::json!({"version_command": "nginx -v"}),
            plugin_config_id: None,
            host_id: None,
            test_kind: None,
            package_identifier: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn response_serialization_roundtrip() {
        let resp = TestPluginConfigResponse {
            success: true,
            test_kind: "version_detection".to_string(),
            output: Some("nginx version: 1.24.0".to_string()),
            error: None,
            detected_version: Some("1.24.0".to_string()),
            duration_ms: 150,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let de: TestPluginConfigResponse = serde_json::from_str(&json).unwrap();
        assert!(de.success);
        assert_eq!(de.test_kind, "version_detection");
        assert_eq!(de.detected_version.as_deref(), Some("1.24.0"));
        assert_eq!(de.duration_ms, 150);
    }

    #[test]
    fn request_serialization_roundtrip() {
        let req = TestPluginConfigRequest {
            plugin_type: "releases_github".to_string(),
            config: serde_json::json!({"api_url": "https://api.github.com"}),
            plugin_config_id: None,
            host_id: Some(uuid::Uuid::nil()),
            test_kind: Some("connectivity".to_string()),
            package_identifier: Some("owner/repo".to_string()),
        };
        let json = serde_json::to_string(&req).unwrap();
        let de: TestPluginConfigRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(de.plugin_type, "releases_github");
        assert_eq!(de.host_id, Some(uuid::Uuid::nil()));
        assert_eq!(de.test_kind.as_deref(), Some("connectivity"));
    }

    #[test]
    fn response_omits_none_fields() {
        let resp = TestPluginConfigResponse {
            success: false,
            test_kind: "connectivity".to_string(),
            output: None,
            error: Some("connection refused".to_string()),
            detected_version: None,
            duration_ms: 5000,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("output"));
        assert!(!json.contains("detected_version"));
        assert!(json.contains("error"));
    }
}
