//! Agent-side plugin configuration test execution.
//!
//! Called when the controller sends a `TestPluginConfig` message. Executes
//! the requested test kind and returns a `TestPluginConfigResultPayload`.

use std::sync::Arc;
use std::time::Instant;

use uptrakit_command::{CommandExecutor, CommandSpec};
use uptrakit_internal_wire::{
    ConfigTestKind, ServiceMessage, TestPluginConfigPayload, TestPluginConfigResultPayload,
};
use uptrakit_plugin_infrastructure_core::{HostCapabilities, construct_host_runtime};
use uptrakit_plugin_infrastructure_registry::get_descriptor;

/// Build a result payload with the common fields populated.
///
/// Since `TestPluginConfigResultPayload` is `#[non_exhaustive]`, external
/// crates must use `::new()` and then set optional fields individually.
fn make_result(request_id: &str, success: bool, start: Instant) -> TestPluginConfigResultPayload {
    TestPluginConfigResultPayload::new(
        request_id.to_string(),
        success,
        start.elapsed().as_millis() as u64,
    )
}

/// Execute a plugin configuration test and return the result as a
/// [`ServiceMessage`].
///
/// This function never panics -- all errors are captured in the result payload.
pub async fn run_config_test(
    payload: TestPluginConfigPayload,
    executor: Arc<dyn CommandExecutor>,
) -> ServiceMessage {
    ServiceMessage::TestPluginConfigResult(handle_config_test(payload, executor).await)
}

/// Execute a plugin configuration test and return the result payload.
///
/// This function never panics -- all errors are captured in the result payload.
async fn handle_config_test(
    payload: TestPluginConfigPayload,
    executor: Arc<dyn CommandExecutor>,
) -> TestPluginConfigResultPayload {
    let start = Instant::now();

    match payload.test_kind {
        ConfigTestKind::VersionDetection => {
            handle_version_detection(
                &payload.request_id,
                &payload.plugin_type,
                &payload.config,
                payload.package_identifier.as_deref(),
                executor,
                start,
            )
            .await
        }
        ConfigTestKind::UpdateCommandValidation => {
            handle_update_command_validation(
                &payload.request_id,
                &payload.plugin_type,
                &payload.config,
                executor,
                start,
            )
            .await
        }
        // PreUpdateHook, PostUpdateHook, Connectivity, and any future
        // variants added to ConfigTestKind are not yet implemented.
        _ => {
            tracing::warn!(
                request_id = %payload.request_id,
                test_kind = ?payload.test_kind,
                "unsupported config test kind on the agent"
            );
            let mut result = make_result(&payload.request_id, false, start);
            result.error = Some(format!(
                "test kind {:?} is not yet supported on the agent",
                payload.test_kind
            ));
            result
        }
    }
}

/// Run `detect_installed_version` through the plugin registry.
async fn handle_version_detection(
    request_id: &str,
    plugin_type_str: &str,
    config: &serde_json::Value,
    package_identifier: Option<&str>,
    executor: Arc<dyn CommandExecutor>,
    start: Instant,
) -> TestPluginConfigResultPayload {
    let runtime = construct_host_runtime(executor, HostCapabilities::default());

    let desc = match get_descriptor(plugin_type_str) {
        Some(d) => d,
        None => {
            let mut result = make_result(request_id, false, start);
            result.error = Some(format!(
                "failed to create plugin: unknown plugin type '{plugin_type_str}'"
            ));
            return result;
        }
    };

    let slot = match desc.roles.version_detector.as_ref() {
        Some(s) => s,
        None => {
            let mut result = make_result(request_id, false, start);
            result.error = Some(format!(
                "plugin '{plugin_type_str}' does not support version detection"
            ));
            return result;
        }
    };

    let detector = match (slot.create)(config, runtime) {
        Ok(d) => d,
        Err(e) => {
            let mut result = make_result(request_id, false, start);
            result.error = Some(format!("failed to create plugin: {e}"));
            return result;
        }
    };

    let pkg_id = package_identifier.unwrap_or_default();
    match detector.detect_installed_version(pkg_id).await {
        Ok(Some(version)) => {
            let version_str = version.to_string();
            tracing::debug!(
                request_id = %request_id,
                detected_version = %version_str,
                "config test: version detected"
            );
            let mut result = make_result(request_id, true, start);
            result.output = Some(format!("detected version: {version_str}"));
            result.detected_version = Some(version_str);
            result
        }
        Ok(None) => {
            let mut result = make_result(request_id, true, start);
            result.output = Some("no version detected (package may not be installed)".to_string());
            result
        }
        Err(e) => {
            let mut result = make_result(request_id, false, start);
            result.error = Some(format!("version detection failed: {e}"));
            result
        }
    }
}

/// Validate update command syntax without executing it.
///
/// For shell plugins, extracts `update_command` from the config JSON and runs
/// `sh -n -c "<command>"` to syntax-check without side effects.
async fn handle_update_command_validation(
    request_id: &str,
    plugin_type_str: &str,
    config: &serde_json::Value,
    executor: Arc<dyn CommandExecutor>,
    start: Instant,
) -> TestPluginConfigResultPayload {
    // Extract update_command from the config JSON. This field is used by the
    // Shell plugin and any plugin config that carries an update command.
    let update_command = config
        .get("update_command")
        .and_then(serde_json::Value::as_str);

    let command = match update_command {
        Some(cmd) if !cmd.is_empty() => cmd,
        _ => {
            let mut result = make_result(request_id, false, start);
            result.error = Some(format!(
                "plugin type '{plugin_type_str}' config has no update_command field"
            ));
            return result;
        }
    };

    // Use `sh -n` to syntax-check the command without executing it.
    // -n: read commands but do not execute them (syntax check only).
    let spec = CommandSpec::exec(
        "sh",
        ["-n".to_string(), "-c".to_string(), command.to_string()],
    );

    match executor.execute_quiet(&spec).await {
        Ok(output) if output.exit_code == 0 => {
            let mut result = make_result(request_id, true, start);
            result.output = Some("update command syntax is valid".to_string());
            result
        }
        Ok(output) => {
            let mut result = make_result(request_id, false, start);
            result.output = Some(output.output);
            result.error = Some(format!(
                "update command syntax check failed (exit code {})",
                output.exit_code
            ));
            result
        }
        Err(e) => {
            let mut result = make_result(request_id, false, start);
            result.error = Some(format!("failed to run syntax check: {e}"));
            result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_command::NoopCommandExecutor;

    fn noop_executor() -> Arc<dyn CommandExecutor> {
        Arc::new(NoopCommandExecutor)
    }

    fn make_payload(
        request_id: &str,
        test_kind: ConfigTestKind,
        plugin_type: &str,
        config: serde_json::Value,
        package_identifier: Option<String>,
    ) -> TestPluginConfigPayload {
        let mut p = TestPluginConfigPayload::new(
            request_id.to_string(),
            "host-1".to_string(),
            test_kind,
            plugin_type.to_string(),
            config,
        );
        p.package_identifier = package_identifier;
        p
    }

    #[tokio::test]
    async fn unsupported_test_kind_returns_error() {
        let payload = make_payload(
            "test-001",
            ConfigTestKind::Connectivity,
            "generic_shell",
            serde_json::json!({}),
            None,
        );

        let result = handle_config_test(payload, noop_executor()).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not yet supported"))
        );
        assert_eq!(result.request_id, "test-001");
        assert!(result.duration_ms < 5000);
    }

    #[tokio::test]
    async fn version_detection_unknown_plugin_type_returns_error() {
        let payload = make_payload(
            "test-002",
            ConfigTestKind::VersionDetection,
            "unknown_plugin_xyz",
            serde_json::json!({}),
            Some("my-pkg".to_string()),
        );

        let result = handle_config_test(payload, noop_executor()).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("failed to create plugin"))
        );
        assert_eq!(result.request_id, "test-002");
    }

    #[tokio::test]
    async fn update_command_validation_missing_field_returns_error() {
        let payload = make_payload(
            "test-003",
            ConfigTestKind::UpdateCommandValidation,
            "generic_shell",
            serde_json::json!({"version_command": "echo 1"}),
            None,
        );

        let result = handle_config_test(payload, noop_executor()).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("no update_command"))
        );
        assert_eq!(result.request_id, "test-003");
    }

    #[tokio::test]
    async fn run_config_test_wraps_in_service_message() {
        let payload = make_payload(
            "test-004",
            ConfigTestKind::Connectivity,
            "generic_shell",
            serde_json::json!({}),
            None,
        );

        let msg = run_config_test(payload, noop_executor()).await;
        match msg {
            ServiceMessage::TestPluginConfigResult(result) => {
                assert_eq!(result.request_id, "test-004");
                assert!(!result.success);
            }
            other => panic!("expected TestPluginConfigResult, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pre_update_hook_returns_unsupported() {
        let payload = make_payload(
            "test-005",
            ConfigTestKind::PreUpdateHook,
            "hook_shell",
            serde_json::json!({}),
            None,
        );

        let result = handle_config_test(payload, noop_executor()).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not yet supported"))
        );
    }

    #[tokio::test]
    async fn post_update_hook_returns_unsupported() {
        let payload = make_payload(
            "test-006",
            ConfigTestKind::PostUpdateHook,
            "hook_shell",
            serde_json::json!({}),
            None,
        );

        let result = handle_config_test(payload, noop_executor()).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("not yet supported"))
        );
    }

    #[tokio::test]
    async fn update_command_validation_empty_command_returns_error() {
        let payload = make_payload(
            "test-007",
            ConfigTestKind::UpdateCommandValidation,
            "generic_shell",
            serde_json::json!({"update_command": ""}),
            None,
        );

        let result = handle_config_test(payload, noop_executor()).await;
        assert!(!result.success);
        assert!(
            result
                .error
                .as_deref()
                .is_some_and(|e| e.contains("no update_command"))
        );
    }
}
