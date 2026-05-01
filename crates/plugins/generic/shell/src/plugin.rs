use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::command::{
    CommandExecutor, CommandSpec, send_output, shell_escape,
};
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, ConfigTestKind, ExecuteUpdateResult, HostRequirements, HostRuntime,
    OutputStreamType, PluginError, PluginFamily, ReleaseInfo, Result, UpdateOutputSender, Version,
    declare_plugin,
};

use crate::config::ShellConfig;

/// Shell plugin implementation.
///
/// Provides generic agent-side operations using user-supplied shell commands:
/// - `detect_installed_version`: reads the installed version via `version_command`.
/// - `execute_update`: runs `update_command` to perform an update.
///
/// Both operations are independently optional; each falls back to a well-typed
/// error when its corresponding config field is absent.
pub struct ShellPlugin {
    config: ShellConfig,
    executor: Arc<dyn CommandExecutor>,
}

impl ShellPlugin {
    /// Create a new `ShellPlugin` from the given configuration.
    ///
    /// The executor is extracted from the host runtime. Config validation is the
    /// caller's responsibility (the registry calls `validate()` before constructing).
    pub fn new(
        config: ShellConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, String> {
        let executor = runtime.executor();
        Ok(Self { config, executor })
    }
}

// ── PluginMeta + PluginDescriptor ─────────────────────────────────────────

declare_plugin!(ShellPlugin, ShellConfig, "generic_shell", {
    display_name: "Generic Shell",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::POSIX,
    config_test: [ConfigTestKind::VersionDetection, ConfigTestKind::UpdateCommandValidation],
    roles: [VersionDetector, UpdateExecutor],
});

// ── Role trait implementations ────────────────────────────────────────────

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for ShellPlugin {
    #[tracing::instrument(skip_all)]
    async fn detect_installed_version(&self, package_identifier: &str) -> Result<Option<Version>> {
        let Some(ref cmd_template) = self.config.version_command else {
            return Ok(None);
        };

        let cmd = cmd_template.replace("{package_identifier}", &shell_escape(package_identifier));

        let output = self
            .executor
            .execute_quiet(&CommandSpec::shell(&cmd))
            .await
            .map_err(|e| {
                report!(PluginError::PluginInternal(format!(
                    "version_command failed: {e}"
                )))
            })?;

        let version = output
            .output
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .map(|l| Version::new(l.to_string()));

        Ok(version)
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for ShellPlugin {
    #[tracing::instrument(skip_all)]
    async fn execute_update(
        &self,
        package_identifier: &str,
        to_version: &str,
        release_info: Option<&ReleaseInfo>,
        output_tx: &UpdateOutputSender,
    ) -> Result<ExecuteUpdateResult> {
        let Some(ref cmd_template) = self.config.update_command else {
            bail!(PluginError::Configuration(
                "execute_update is not configured: update_command is absent".to_string()
            ));
        };

        let tag = release_info.map(|r| r.tag.as_str()).unwrap_or(to_version);

        let cmd = cmd_template
            .replace("{version}", &shell_escape(to_version))
            .replace("{tag}", &shell_escape(tag))
            .replace("{package_identifier}", &shell_escape(package_identifier));

        send_output(
            output_tx,
            &format!("Running: {cmd}"),
            OutputStreamType::Stdout,
        )
        .await;

        tracing::debug!(command = %cmd, "running shell update command");

        let cmd_output = self
            .executor
            .execute(&CommandSpec::shell(&cmd), output_tx)
            .await
            .map_err(|e| {
                report!(PluginError::InstallFailed(format!(
                    "update_command failed: {e}"
                )))
            })?;

        Ok(ExecuteUpdateResult::new(cmd_output.output, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, LocalCommandExecutor, PluginCapability, PluginMeta, ReleaseInfo,
        StandardHostRuntime, UpdateExecutor, VersionDetector, mpsc,
    };

    fn test_runtime() -> Arc<dyn HostRuntime> {
        let executor = Arc::new(LocalCommandExecutor) as Arc<dyn CommandExecutor>;
        let caps = HostCapabilities::default();
        Arc::new(StandardHostRuntime::new(executor, caps)) as Arc<dyn HostRuntime>
    }

    fn make_plugin(version_command: Option<&str>, update_command: Option<&str>) -> ShellPlugin {
        ShellPlugin::new(
            ShellConfig {
                version_command: version_command.map(String::from),
                update_command: update_command.map(String::from),
                prefer_interactive: false,
            },
            test_runtime(),
        )
        .expect("plugin creation")
    }

    // ── descriptor tests ─────────────────────────────────────────────────────

    #[test]
    fn plugin_type_id() {
        let plugin = make_plugin(Some("echo 1"), None);
        assert_eq!(plugin.plugin_type_id().as_str(), "generic_shell");
    }

    #[test]
    fn descriptor_capabilities() {
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::VersionDetection)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::UpdateExecution)
        );
        assert!(
            DESCRIPTOR
                .capabilities
                .contains(&PluginCapability::ConfigTest)
        );
    }

    #[test]
    fn descriptor_has_expected_roles() {
        assert!(DESCRIPTOR.roles.version_detector.is_some());
        assert!(DESCRIPTOR.roles.update_executor.is_some());
        assert!(DESCRIPTOR.roles.discoverer.is_none());
        assert!(DESCRIPTOR.roles.release_fetcher.is_none());
    }

    // ── detect_installed_version tests ────────────────────────────────────────

    #[tokio::test]
    async fn detect_version_returns_none_when_no_command() {
        let plugin = make_plugin(None, Some("dummy"));
        let result = plugin.detect_installed_version("pkg").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_version_returns_first_line() {
        let plugin = make_plugin(Some("echo '3.14.1'"), None);
        let result = plugin
            .detect_installed_version("any-pkg")
            .await
            .expect("should succeed");
        assert_eq!(result.as_ref().map(|v| v.as_str()), Some("3.14.1"));
    }

    #[tokio::test]
    async fn detect_version_placeholder_replaced() {
        let plugin = make_plugin(Some("echo '{package_identifier}'"), None);
        let result = plugin
            .detect_installed_version("booklore")
            .await
            .expect("should succeed");
        // Shell-escaped placeholder is expanded; output contains the identifier.
        assert!(result.is_some());
        assert!(result.unwrap().as_str().contains("booklore"));
    }

    #[tokio::test]
    async fn detect_version_empty_output_returns_none() {
        let plugin = make_plugin(Some("true"), None);
        let result = plugin
            .detect_installed_version("pkg")
            .await
            .expect("should succeed");
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn detect_version_command_failure_propagates_error() {
        let plugin = make_plugin(Some("false"), None);
        let result = plugin.detect_installed_version("pkg").await;
        assert!(result.is_err());
    }

    // ── execute_update tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn execute_update_returns_error_when_no_command() {
        let plugin = make_plugin(Some("echo hi"), None);
        let (tx, _rx) = mpsc::channel(100);
        let result = plugin.execute_update("pkg", "1.0.0", None, &tx).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_update_runs_with_version_replaced() {
        // Use a command that echoes the version arg so we can verify replacement.
        let plugin = make_plugin(None, Some("echo {version}"));
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("mypkg", "2.3.4", None, &tx)
            .await
            .expect("should succeed");
        assert!(result.output.contains("2.3.4"));
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_tag_falls_back_to_version_when_no_release_info() {
        // {tag} should fall back to {version} when release_info is None.
        let plugin = make_plugin(None, Some("echo {tag}"));
        let (tx, mut rx) = mpsc::channel(100);
        let result = plugin
            .execute_update("mypkg", "1.0.0", None, &tx)
            .await
            .expect("should succeed");
        assert!(result.output.contains("1.0.0"));
        rx.close();
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn execute_update_tag_uses_release_info_tag() {
        let plugin = make_plugin(None, Some("echo {tag}"));
        let (tx, mut rx) = mpsc::channel(100);
        let release_info = ReleaseInfo {
            tag: "v1.2.3".to_string(),
            release_url: "https://example.com".to_string(),
            assets: vec![],
            attestation_status: None,
            require_attestation: false,
        };
        let result = plugin
            .execute_update("mypkg", "1.2.3", Some(&release_info), &tx)
            .await
            .expect("should succeed");
        assert!(result.output.contains("v1.2.3"));
        rx.close();
        while rx.recv().await.is_some() {}
    }
}
