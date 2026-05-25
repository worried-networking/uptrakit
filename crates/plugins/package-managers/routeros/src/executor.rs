//! RouterOS update executor — routes between install (download+reboot) and
//! download-only paths based on config and runtime policy.

use std::sync::Arc;

use rootcause::prelude::*;

use uptrakit_plugin_infrastructure_core::RouterOsExecutor;

use crate::channel::RouterOsChannel;
use crate::error::{Result, RouterOsError};

/// Drives a RouterOS update: triggers the background update check, waits for
/// the router to finish the check, then either downloads-and-installs (reboot)
/// or downloads only.
pub(crate) struct RouterOsUpdateExecutor {
    /// The typed RouterOS CLI interface.
    pub(crate) exec: Arc<dyn RouterOsExecutor>,
    /// Whether the plugin config requests auto-reboot.
    pub(crate) reboot: bool,
    /// Hard gate from the host runtime — reboot is impossible without it.
    pub(crate) allow_reboot: bool,
}

impl RouterOsUpdateExecutor {
    /// Run the update sequence.
    ///
    /// 1. If `channel` is `Some`, set it on the router before checking.
    /// 2. Trigger the background update check (`check-for-updates`).
    /// 3. Wait 10 seconds for the router to complete the check.
    /// 4. Either issue `package install` (if `reboot && allow_reboot`)
    ///    or `package download`.
    pub(crate) async fn run_update(&self, channel: Option<&RouterOsChannel>) -> Result<()> {
        if let Some(ch) = channel {
            self.exec
                .set_update_channel(ch.as_str())
                .await
                .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;
        }

        self.exec
            .check_for_updates()
            .await
            .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;

        // Give the router time to complete the background check.
        // RouterOS processes the check asynchronously; callers must wait before
        // reading `latest-version` or issuing install/download commands.
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;

        if self.reboot && self.allow_reboot {
            self.exec
                .package_install()
                .await
                .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;
        } else {
            self.exec
                .package_download()
                .await
                .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use parking_lot::Mutex;

    use uptrakit_plugin_infrastructure_core::{PluginError, RouterOsExecutor};

    use super::*;

    // ── mock executor ─────────────────────────────────────────────────────────

    /// Which method was called last on the mock.
    #[derive(Debug, Clone, PartialEq)]
    enum LastCall {
        None,
        CheckForUpdates,
        PackageInstall,
        PackageDownload,
    }

    struct MockExec {
        last_call: Mutex<LastCall>,
    }

    impl MockExec {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                last_call: Mutex::new(LastCall::None),
            })
        }

        fn last(&self) -> LastCall {
            self.last_call.lock().clone()
        }
    }

    #[async_trait::async_trait]
    impl RouterOsExecutor for MockExec {
        async fn resource_print(&self) -> std::result::Result<String, PluginError> {
            Ok(String::new())
        }

        async fn routerboard_print(&self) -> std::result::Result<String, PluginError> {
            Ok(String::new())
        }

        async fn license_print(&self) -> std::result::Result<String, PluginError> {
            Ok(String::new())
        }

        async fn set_update_channel(&self, _channel: &str) -> std::result::Result<(), PluginError> {
            Ok(())
        }

        async fn check_for_updates(&self) -> std::result::Result<(), PluginError> {
            *self.last_call.lock() = LastCall::CheckForUpdates;
            Ok(())
        }

        async fn package_update_print(&self) -> std::result::Result<String, PluginError> {
            Ok(String::new())
        }

        async fn package_install(&self) -> std::result::Result<(), PluginError> {
            *self.last_call.lock() = LastCall::PackageInstall;
            Ok(())
        }

        async fn package_download(&self) -> std::result::Result<(), PluginError> {
            *self.last_call.lock() = LastCall::PackageDownload;
            Ok(())
        }
    }

    // ── tests ─────────────────────────────────────────────────────────────────

    #[tokio::test(start_paused = true)]
    async fn run_update_calls_check_then_download_when_reboot_false() {
        let mock = MockExec::new();
        let executor = RouterOsUpdateExecutor {
            exec: Arc::clone(&mock) as Arc<dyn RouterOsExecutor>,
            reboot: false,
            allow_reboot: true,
        };
        executor
            .run_update(None)
            .await
            .expect("update should succeed");
        assert_eq!(mock.last(), LastCall::PackageDownload);
    }

    #[tokio::test(start_paused = true)]
    async fn run_update_calls_check_then_download_when_allow_reboot_false() {
        let mock = MockExec::new();
        let executor = RouterOsUpdateExecutor {
            exec: Arc::clone(&mock) as Arc<dyn RouterOsExecutor>,
            reboot: true,
            allow_reboot: false,
        };
        executor
            .run_update(None)
            .await
            .expect("update should succeed");
        // allow_reboot=false: must download even though reboot=true
        assert_eq!(mock.last(), LastCall::PackageDownload);
    }

    #[tokio::test(start_paused = true)]
    async fn run_update_calls_install_when_reboot_and_allow_reboot_both_true() {
        let mock = MockExec::new();
        let executor = RouterOsUpdateExecutor {
            exec: Arc::clone(&mock) as Arc<dyn RouterOsExecutor>,
            reboot: true,
            allow_reboot: true,
        };
        executor
            .run_update(None)
            .await
            .expect("update should succeed");
        assert_eq!(mock.last(), LastCall::PackageInstall);
    }

    #[tokio::test(start_paused = true)]
    async fn run_update_calls_download_when_both_false() {
        let mock = MockExec::new();
        let executor = RouterOsUpdateExecutor {
            exec: Arc::clone(&mock) as Arc<dyn RouterOsExecutor>,
            reboot: false,
            allow_reboot: false,
        };
        executor
            .run_update(None)
            .await
            .expect("update should succeed");
        assert_eq!(mock.last(), LastCall::PackageDownload);
    }
}
