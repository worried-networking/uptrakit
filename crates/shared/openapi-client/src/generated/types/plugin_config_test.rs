// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Request and response types for plugin configuration testing (dry-run).
use crate::generated::types::validation::{Validate, ValidationError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
/// Request body for `POST /api/v1/plugin-configs/test`.
///
/// Tests a plugin configuration without saving it. The user can submit an
/// ad-hoc plugin type + config JSON for testing before creating a plugin
/// config record.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
impl TestPluginConfigRequest {
    /// Create a new test request with the required fields.
    ///
    /// Optional fields default to `None` and can be set with the builder
    /// methods [`with_plugin_config_id`], [`with_host_id`], [`with_test_kind`],
    /// and [`with_package_identifier`].
    ///
    /// [`with_plugin_config_id`]: Self::with_plugin_config_id
    /// [`with_host_id`]: Self::with_host_id
    /// [`with_test_kind`]: Self::with_test_kind
    /// [`with_package_identifier`]: Self::with_package_identifier
    pub fn new(plugin_type: String, config: serde_json::Value) -> Self {
        Self {
            plugin_type,
            config,
            plugin_config_id: None,
            host_id: None,
            test_kind: None,
            package_identifier: None,
        }
    }
    /// Set the saved config ID to merge with.
    #[must_use]
    pub fn with_plugin_config_id(mut self, id: Option<uuid::Uuid>) -> Self {
        self.plugin_config_id = id;
        self
    }
    /// Set the host ID for agent-side tests.
    #[must_use]
    pub fn with_host_id(mut self, id: Option<uuid::Uuid>) -> Self {
        self.host_id = id;
        self
    }
    /// Set the test kind.
    #[must_use]
    pub fn with_test_kind(mut self, kind: Option<String>) -> Self {
        self.test_kind = kind;
        self
    }
    /// Set the package identifier.
    #[must_use]
    pub fn with_package_identifier(mut self, identifier: Option<String>) -> Self {
        self.package_identifier = identifier;
        self
    }
}
/// Response for plugin config test.
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
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
impl TestPluginConfigResponse {
    /// Create a new response with the required fields.
    pub fn new(success: bool, test_kind: String, duration_ms: u64) -> Self {
        Self {
            success,
            test_kind,
            output: None,
            error: None,
            detected_version: None,
            duration_ms,
        }
    }
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
