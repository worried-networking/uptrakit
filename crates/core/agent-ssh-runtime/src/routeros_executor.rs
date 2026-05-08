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

/// Substrings that indicate a RouterOS command failed.
///
/// RouterOS' SSH server **always returns exit code 0** even on command errors
/// (forum t=153623), so output parsing is the only reliable detection. Markers
/// are anchored on `line.contains(marker)`. Each entry is sourced to MikroTik
/// docs, the official forum, or output reproduced against a real router; do
/// not add markers without a confirmed source — false positives abort
/// bootstrap mid-way. See [`classify_ros_bootstrap_result`].
const ROS_ERROR_MARKERS: &[&str] = &[
    // `failure: <reason>` — runtime errors. MikroTik Scripting docs example:
    // `failure: dns name does not exist`. Existing in-tree precedent in
    // `bootstrap.rs::detect_host_os` already keys on `not enough permissions`,
    // which appears both as `failure: not enough permissions` and standalone.
    "failure:",
    "not enough permissions",
    // Enum-parameter rejection. Reproduced by user against this Mikrotik:
    // `input does not match any value of policy ...`. Forum t=74415.
    "input does not match any value",
    // Parser/syntax errors. MikroTik Scripting docs: `bad command name this
    // (line 1 column 1)`, `syntax error (line 1 column 7)`.
    "bad command name",
    "syntax error",
    // Script-level errors emitted by `/import` and similar; docs example:
    // `Script Error: bad command name ...`.
    "Script Error:",
];

/// Classify a RouterOS bootstrap command result as success or failure.
///
/// **Scope**: this function is for the three bootstrap helpers
/// (`create_group`, `create_user`, `import_ssh_key`) only. The read-only
/// [`RouterOsExecutor`] trait methods parse legitimate output that may
/// coincidentally contain marker substrings (for instance `/log print` event
/// names) — they intentionally use the lenient `exec_raw` path.
fn classify_ros_bootstrap_result(
    cmd: &str,
    result: &crate::ssh_transport::RemoteCommandResult,
) -> crate::error::Result<()> {
    let combined = format!("{}{}", result.stdout, result.stderr);
    if let Some(line) = combined
        .lines()
        .find(|line| ROS_ERROR_MARKERS.iter().any(|m| line.contains(m)))
    {
        rootcause::bail!(crate::error::Error::SshCommand(format!(
            "RouterOS command failed: `{cmd}`: {}",
            line.trim()
        )));
    }
    if result.exit_code != 0 {
        rootcause::bail!(crate::error::Error::SshCommand(format!(
            "RouterOS command failed: `{cmd}` (exit_code={}); output: {}",
            result.exit_code,
            combined.trim()
        )));
    }
    Ok(())
}

impl RouterOsSshExecutor {
    pub(crate) async fn create_group(&self, policy_str: &str) -> crate::error::Result<()> {
        let cmd = format!("/user group add name=uptrakit policy={policy_str}");
        let result = self
            .inner
            .session()
            .exec_command(&cmd)
            .await
            .map_err(|e| rootcause::report!(crate::error::Error::SshCommand(e.to_string())))?;
        classify_ros_bootstrap_result(&cmd, &result)
    }

    pub(crate) async fn create_user(&self) -> crate::error::Result<()> {
        let cmd = r#"/user add name=uptrakit group=uptrakit password="""#;
        let result = self
            .inner
            .session()
            .exec_command(cmd)
            .await
            .map_err(|e| rootcause::report!(crate::error::Error::SshCommand(e.to_string())))?;
        classify_ros_bootstrap_result(cmd, &result)
    }

    pub(crate) async fn import_ssh_key(&self, remote_path: &str) -> crate::error::Result<()> {
        let cmd = format!("/user ssh-keys import public-key-file={remote_path} user=uptrakit");
        let result = self
            .inner
            .session()
            .exec_command(&cmd)
            .await
            .map_err(|e| rootcause::report!(crate::error::Error::SshCommand(e.to_string())))?;
        classify_ros_bootstrap_result(&cmd, &result)
    }
}

/// Parse a `key: value` line from RouterOS CLI output.
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

    // ── classify_ros_bootstrap_result tests ─────────────────────────────

    fn make_result(
        stdout: &str,
        stderr: &str,
        exit_code: u32,
    ) -> crate::ssh_transport::RemoteCommandResult {
        crate::ssh_transport::RemoteCommandResult {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    #[test]
    fn classify_ok_on_clean_zero_exit() {
        let result = make_result("", "", 0);
        classify_ros_bootstrap_result("/user add name=uptrakit", &result)
            .expect("clean output + exit 0 must classify as Ok");
    }

    #[test]
    fn classify_known_bootstrap_success_outputs() {
        // /user group add and /user add emit nothing on success.
        classify_ros_bootstrap_result(
            "/user group add name=uptrakit policy=ssh,read",
            &make_result("", "", 0),
        )
        .expect("/user group add success is silent");
        classify_ros_bootstrap_result(
            r#"/user add name=uptrakit group=uptrakit password="""#,
            &make_result("", "", 0),
        )
        .expect("/user add success is silent");
        // /user ssh-keys import on success emits a key listing block; assert
        // a representative form classifies as Ok (no marker collision).
        let import_success = "Flags: D - DISABLED\n0   user=uptrakit key-owner=\"\" bits=256\n";
        classify_ros_bootstrap_result(
            "/user ssh-keys import public-key-file=uptrakit-bootstrap.pub user=uptrakit",
            &make_result(import_success, "", 0),
        )
        .expect("/user ssh-keys import success listing must classify as Ok");
    }

    #[test]
    fn classify_marker_input_does_not_match() {
        let stdout = "input does not match any value of policy (/user/group/add (policy); line 1)";
        let result = make_result(stdout, "", 0);
        let err = classify_ros_bootstrap_result(
            "/user group add name=uptrakit policy=read,test,update",
            &result,
        )
        .expect_err("invalid enum value must surface as error");
        let msg = format!("{err:?}");
        assert!(msg.contains("input does not match any value"), "msg: {msg}");
        assert!(msg.contains("/user group add"), "msg: {msg}");
    }

    #[test]
    fn classify_marker_failure_prefix() {
        let result = make_result("", "failure: not enough permissions (9)\n", 0);
        let err = classify_ros_bootstrap_result("/user add name=uptrakit", &result)
            .expect_err("`failure:` line must surface as error");
        assert!(format!("{err:?}").contains("failure: not enough permissions"));
    }

    #[test]
    fn classify_marker_not_enough_permissions_standalone() {
        let result = make_result("not enough permissions (9)\n", "", 0);
        let err = classify_ros_bootstrap_result("/user group add", &result)
            .expect_err("standalone permissions error must surface");
        assert!(format!("{err:?}").contains("not enough permissions"));
    }

    #[test]
    fn classify_marker_bad_command_name() {
        let result = make_result("bad command name this (line 1 column 1)\n", "", 0);
        let err = classify_ros_bootstrap_result("this is not valid", &result)
            .expect_err("bad command name must surface");
        assert!(format!("{err:?}").contains("bad command name"));
    }

    #[test]
    fn classify_marker_syntax_error() {
        let result = make_result("syntax error (line 1 column 7)\n", "", 0);
        let err = classify_ros_bootstrap_result("/user add =", &result)
            .expect_err("syntax error must surface");
        assert!(format!("{err:?}").contains("syntax error"));
    }

    #[test]
    fn classify_nonzero_exit_no_marker_still_fails() {
        // Cannot happen on real ROS today (exit_code is always 0) but kept
        // as a defence in case a future ROS version starts returning real
        // exit codes — we'd prefer to fail rather than mask the signal.
        let result = make_result("", "", 1);
        let err = classify_ros_bootstrap_result("/user add", &result)
            .expect_err("non-zero exit without marker must still error");
        let msg = format!("{err:?}");
        assert!(msg.contains("exit_code=1"), "msg: {msg}");
    }

    #[test]
    fn classify_max_exit_treated_as_failure() {
        // u32::MAX is the "no exit status received" sentinel from
        // `exec_command_streaming` — a bootstrap command where the server
        // never sent ExitStatus is suspicious enough to fail.
        let result = make_result("", "", u32::MAX);
        classify_ros_bootstrap_result("/user add", &result)
            .expect_err("u32::MAX exit code must classify as failure");
    }
}
