//! RouterOS-specific SSH executor.

use std::sync::Arc;
use std::time::Duration;

use uptrakit_plugin_infrastructure_registry::{PluginError, RouterOsExecutor};

use crate::ssh_executor::SshCommandExecutor;
use crate::ssh_transport::SshSession;

const ROS_CMD_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct RouterOsSshExecutor {
    inner: SshCommandExecutor,
}

impl RouterOsSshExecutor {
    #[expect(
        dead_code,
        reason = "called from RouterOS host runtime construction in Plan B Task 5"
    )]
    pub(crate) fn new(session: Arc<SshSession>) -> Self {
        Self {
            inner: SshCommandExecutor::new(session),
        }
    }
}

#[async_trait::async_trait]
impl RouterOsExecutor for RouterOsSshExecutor {
    async fn resource_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system resource print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn routerboard_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system routerboard print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn license_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system license print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn check_for_updates(&self) -> std::result::Result<(), PluginError> {
        self.inner
            .exec_raw(
                "/system package update check-for-updates",
                Some(ROS_CMD_TIMEOUT),
            )
            .await
            .map(|_| ())
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn package_update_print(&self) -> std::result::Result<String, PluginError> {
        self.inner
            .exec_raw("/system package update print", Some(ROS_CMD_TIMEOUT))
            .await
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn package_install(&self) -> std::result::Result<(), PluginError> {
        self.inner
            .exec_raw("/system package update install", Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }

    async fn package_download(&self) -> std::result::Result<(), PluginError> {
        self.inner
            .exec_raw("/system package update download", Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| PluginError::PluginInternal(e.to_string()))
    }
}

// ── Bootstrap helpers (not part of RouterOsExecutor trait) ──────────────

impl RouterOsSshExecutor {
    #[expect(
        dead_code,
        reason = "called from RouterOS bootstrap logic in Plan B Task 5"
    )]
    pub(crate) async fn create_group(&self, policy_str: &str) -> crate::error::Result<()> {
        let cmd = format!("/user group add name=uptrakit policy={policy_str}");
        self.inner
            .exec_raw(&cmd, Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| rootcause::report!(crate::error::Error::SshCommand(e.to_string())))
    }

    #[expect(
        dead_code,
        reason = "called from RouterOS bootstrap logic in Plan B Task 5"
    )]
    pub(crate) async fn create_user(&self) -> crate::error::Result<()> {
        self.inner
            .exec_raw(
                r#"/user add name=uptrakit group=uptrakit password="""#,
                Some(ROS_CMD_TIMEOUT),
            )
            .await
            .map(|_| ())
            .map_err(|e| rootcause::report!(crate::error::Error::SshCommand(e.to_string())))
    }

    #[expect(
        dead_code,
        reason = "called from RouterOS bootstrap logic in Plan B Task 5"
    )]
    pub(crate) async fn import_ssh_key(&self, remote_path: &str) -> crate::error::Result<()> {
        let cmd = format!("/user ssh-keys import public-key-file={remote_path} user=uptrakit");
        self.inner
            .exec_raw(&cmd, Some(ROS_CMD_TIMEOUT))
            .await
            .map(|_| ())
            .map_err(|e| rootcause::report!(crate::error::Error::SshCommand(e.to_string())))
    }
}

/// Parse a `key: value` line from RouterOS CLI output.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by RouterOS plugin discovery logic added in Plan B Task 3+"
    )
)]
pub(crate) fn parse_routeros_field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    for line in output.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(key)
            && let Some(val) = rest.strip_prefix(':')
        {
            return Some(val.trim());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_routeros_field_found() {
        let output = "version: 7.14.2 (stable)\nplatform: MikroTik\n";
        assert_eq!(
            parse_routeros_field(output, "version"),
            Some("7.14.2 (stable)")
        );
    }

    #[test]
    fn parse_routeros_field_missing() {
        assert_eq!(
            parse_routeros_field("platform: MikroTik\n", "version"),
            None
        );
    }

    #[test]
    fn parse_routeros_field_trims_whitespace() {
        assert_eq!(
            parse_routeros_field("  serial-number:  ABC123  \n", "serial-number"),
            Some("ABC123")
        );
    }
}
