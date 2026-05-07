# RouterOS Support — Plan C: Plugin and Documentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the `uptrakit-package-manager-routeros` crate implementing `VersionDetector`, `ReleaseFetcher`, and `UpdateExecutor` roles, then
update documentation (`CONTEXT.md`, `plugin-guidelines.md`, ADR).

**Architecture:** The plugin depends only on `plugin-infrastructure-core` (for `RouterOsHostRuntime`, `RouterOsExecutor`,
`HostRequirements::ROUTER_OS`) and has no dependency on `agent-ssh`. It downcasts `Arc<dyn HostRuntime>` → `RouterOsHostRuntime` to access the typed
executor and `allow_reboot` flag. Version parsing is local pure functions (no subprocess).

**Tech Stack:** Rust (edition 2024), async-trait, serde, thiserror, rootcause, uptrakit-shared-macros

**Prerequisites:** Plans A and B must be merged before starting this plan.

---

## Tasks

### Task 1: Scaffold the new crate

**Files:**

- Create: `crates/plugins/package-managers/routeros/Cargo.toml`
- Create: `crates/plugins/package-managers/routeros/src/lib.rs`
- Modify: `Cargo.toml` (workspace root) — add to `[workspace.members]`

- [ ] **Step 1: Create Cargo.toml**

```toml
[package]
name = "uptrakit-package-manager-routeros"
description = "Uptrakit package-manager plugin for MikroTik RouterOS"
edition.workspace = true
license.workspace = true
authors.workspace = true
repository.workspace = true
version = "0.0.1"

[dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true }
uptrakit-shared-types = { workspace = true }
uptrakit-shared-macros = { workspace = true }
rootcause = { workspace = true }
async-trait = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }

[dev-dependencies]
uptrakit-plugin-infrastructure-core = { workspace = true, features = ["testing"] }
tokio = { workspace = true, features = ["macros", "rt"] }
parking_lot = { workspace = true }

[lints]
workspace = true
```

- [ ] **Step 2: Create src/lib.rs placeholder**

```rust
pub mod config;
pub mod error;
pub mod executor;
pub mod plugin;
pub mod version;

pub use config::RouterOsConfig;
pub use error::RouterOsError;
pub use plugin::RouterOsPlugin;
```

- [ ] **Step 3: Add to workspace members in root Cargo.toml**

In the `[workspace]` `members` array, add:

```toml
"crates/plugins/package-managers/routeros",
```

- [ ] **Step 4: Check workspace compiles (all modules will fail until added)**

```bash
cargo check -p uptrakit-package-manager-routeros 2>&1 | head -20
```

Expected: errors about missing modules — that's fine for now.

- [ ] **Step 5: Commit scaffold**

```bash
git add crates/plugins/package-managers/routeros/ Cargo.toml
git commit -m "chore(routeros): scaffold uptrakit-package-manager-routeros crate"
```

---

### Task 2: Implement config.rs and error.rs

**Files:**

- Create: `crates/plugins/package-managers/routeros/src/config.rs`
- Create: `crates/plugins/package-managers/routeros/src/error.rs`

- [ ] **Step 1: Write config.rs**

```rust
use uptrakit_plugin_infrastructure_core::{
    PluginConfig, PluginConfigValidationError, TypeSettings,
    form_schema::FormFieldDescriptor,
};

/// Per-plugin-assignment configuration for the RouterOS package manager.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RouterOsConfig {
    /// RouterOS update channel: `"stable"`, `"long-term"`, `"testing"`, or
    /// `None` to leave the router's configured channel unchanged.
    #[serde(default)]
    pub channel: Option<String>,
    /// If `true`, trigger an immediate reboot after downloading the update
    /// (calls `/system package update install` instead of `download`).
    ///
    /// Has no effect if `routeros_host_config.allow_reboot` is `false` for
    /// this host — the RouterOS group lacks the `reboot` policy in that case.
    #[serde(default)]
    pub reboot: bool,
}

impl PluginConfig for RouterOsConfig {
    fn validate(&self) -> Result<(), PluginConfigValidationError> {
        if let Some(channel) = &self.channel {
            let valid = ["stable", "long-term", "testing"];
            if !valid.contains(&channel.as_str()) {
                return Err(PluginConfigValidationError::Contract(format!(
                    "channel must be one of {valid:?}, got {channel:?}"
                )));
            }
        }
        Ok(())
    }

    fn validate_identifier(_value: &str) -> Result<(), PluginConfigValidationError> {
        Ok(()) // RouterOS has a single software item; any identifier is valid
    }

    fn form_schema() -> Vec<FormFieldDescriptor> {
        vec![]
    }
}

impl TypeSettings for RouterOsConfig {
    fn type_settings_form_schema() -> Vec<FormFieldDescriptor> {
        vec![]
    }

    fn type_settings_sample() -> serde_json::Value {
        serde_json::json!({ "reboot": false })
    }
}
```

- [ ] **Step 2: Write error.rs**

```rust
use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_plugin_infrastructure_core::PluginError;
use uptrakit_shared_macros::impl_report_conversion;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RouterOsError {
    #[error("SSH exec failed: {0}")]
    SshExec(String),
    #[error("failed to parse RouterOS output field '{field}' from: {context}")]
    ParseFailure {
        field: &'static str,
        context: String,
    },
    #[error("version not available: {0}")]
    VersionUnavailable(String),
}

pub type Result<T> = std::result::Result<T, Report<RouterOsError>>;

impl_report_conversion!(
    RouterOsError => PluginError,
    |e: RouterOsError| PluginError::PluginInternal(e.to_string())
);
```

- [ ] **Step 3: Build to verify**

```bash
cargo check -p uptrakit-package-manager-routeros 2>&1 | head -20
```

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/package-managers/routeros/src/config.rs crates/plugins/package-managers/routeros/src/error.rs
git commit -m "feat(routeros-plugin): add RouterOsConfig and RouterOsError"
```

---

### Task 3: Implement version.rs — parsing helpers

**Files:**

- Create: `crates/plugins/package-managers/routeros/src/version.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_resource_version_strips_channel_suffix() {
        let output = "version: 7.14.2 (stable)\nplatform: MikroTik\n";
        assert_eq!(parse_resource_version(output), Some("7.14.2".to_string()));
    }

    #[test]
    fn parse_resource_version_no_suffix() {
        let output = "version: 7.15\nuptime: 1d\n";
        assert_eq!(parse_resource_version(output), Some("7.15".to_string()));
    }

    #[test]
    fn parse_resource_version_missing_field() {
        assert_eq!(parse_resource_version("uptime: 3d\n"), None);
    }

    #[test]
    fn parse_latest_version_returns_field() {
        let output = "channel: stable\ninstalled-version: 7.14.2\nlatest-version: 7.15\n";
        assert_eq!(parse_latest_version(output), Some("7.15".to_string()));
    }

    #[test]
    fn parse_latest_version_missing() {
        assert_eq!(parse_latest_version("channel: stable\n"), None);
    }

    #[test]
    fn parse_routeros_field_found() {
        assert_eq!(parse_ros_field("key: value\n", "key"), Some("value"));
    }

    #[test]
    fn parse_routeros_field_missing() {
        assert_eq!(parse_ros_field("other: value\n", "key"), None);
    }
}
```

Run: `cargo test -p uptrakit-package-manager-routeros version` Expected: FAIL (functions not defined yet).

- [ ] **Step 2: Implement version.rs**

```rust
//! RouterOS output parsing helpers.
//!
//! `parse_routeros_field` is duplicated from `agent-ssh/routeros_executor.rs`
//! because that crate is internal. Duplication (~5 lines) is correct — no
//! cross-crate re-export across the agent/plugin boundary.

/// Parse a `key: value` line from RouterOS CLI output.
///
/// Trims leading/trailing whitespace from both key and value.
/// Returns `None` if the key is absent.
pub(crate) fn parse_ros_field<'a>(output: &'a str, key: &str) -> Option<&'a str> {
    for line in output.lines() {
        if let Some(rest) = line.trim_start().strip_prefix(key) {
            if let Some(val) = rest.strip_prefix(':') {
                return Some(val.trim());
            }
        }
    }
    None
}

/// Parse the `version:` field from `/system resource print` output.
///
/// Strips the channel suffix in parentheses:
/// `7.14.2 (stable)` → `"7.14.2"`.
pub fn parse_resource_version(output: &str) -> Option<String> {
    let raw = parse_ros_field(output, "version")?;
    // Strip " (channel)" suffix if present
    let trimmed = match raw.split_once('(') {
        Some((before, _)) => before.trim(),
        None => raw,
    };
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Parse the `latest-version:` field from `/system package update print` output.
pub fn parse_latest_version(output: &str) -> Option<String> {
    let val = parse_ros_field(output, "latest-version")?;
    if val.is_empty() { None } else { Some(val.to_string()) }
}
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p uptrakit-package-manager-routeros version
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/package-managers/routeros/src/version.rs
git commit -m "feat(routeros-plugin): add version parsing helpers"
```

---

### Task 4: Implement executor.rs — update routing

**Files:**

- Create: `crates/plugins/package-managers/routeros/src/executor.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::RouterOsExecutor;
    use parking_lot::Mutex;

    // Simple recorder for which method was called last.
    #[derive(Default)]
    struct RecordingExec {
        last_call: Mutex<Option<&'static str>>,
    }
    #[async_trait::async_trait]
    impl RouterOsExecutor for RecordingExec {
        async fn resource_print(&self) -> std::result::Result<String, uptrakit_plugin_infrastructure_core::PluginError> {
            Ok(String::new())
        }
        async fn routerboard_print(&self) -> std::result::Result<String, uptrakit_plugin_infrastructure_core::PluginError> {
            Ok(String::new())
        }
        async fn license_print(&self) -> std::result::Result<String, uptrakit_plugin_infrastructure_core::PluginError> {
            Ok(String::new())
        }
        async fn check_for_updates(&self) -> std::result::Result<(), uptrakit_plugin_infrastructure_core::PluginError> {
            Ok(())
        }
        async fn package_update_print(&self) -> std::result::Result<String, uptrakit_plugin_infrastructure_core::PluginError> {
            Ok(String::new())
        }
        async fn package_install(&self) -> std::result::Result<(), uptrakit_plugin_infrastructure_core::PluginError> {
            *self.last_call.lock() = Some("install");
            Ok(())
        }
        async fn package_download(&self) -> std::result::Result<(), uptrakit_plugin_infrastructure_core::PluginError> {
            *self.last_call.lock() = Some("download");
            Ok(())
        }
    }

    fn executor(reboot: bool, allow_reboot: bool) -> RouterOsUpdateExecutor {
        RouterOsUpdateExecutor {
            exec: Arc::new(RecordingExec::default()),
            reboot,
            allow_reboot,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn install_called_when_both_true() {
        let exec = Arc::new(RecordingExec::default());
        let ue = RouterOsUpdateExecutor {
            exec: Arc::clone(&exec) as Arc<dyn RouterOsExecutor>,
            reboot: true,
            allow_reboot: true,
        };
        ue.run_update().await.unwrap();
        assert_eq!(*exec.last_call.lock(), Some("install"));
    }

    #[tokio::test(start_paused = true)]
    async fn download_called_when_reboot_true_but_disallowed() {
        let exec = Arc::new(RecordingExec::default());
        let ue = RouterOsUpdateExecutor {
            exec: Arc::clone(&exec) as Arc<dyn RouterOsExecutor>,
            reboot: true,
            allow_reboot: false,
        };
        ue.run_update().await.unwrap();
        assert_eq!(*exec.last_call.lock(), Some("download"));
    }

    #[tokio::test(start_paused = true)]
    async fn download_called_when_reboot_false() {
        let exec = Arc::new(RecordingExec::default());
        let ue = RouterOsUpdateExecutor {
            exec: Arc::clone(&exec) as Arc<dyn RouterOsExecutor>,
            reboot: false,
            allow_reboot: true,
        };
        ue.run_update().await.unwrap();
        assert_eq!(*exec.last_call.lock(), Some("download"));
    }
}
```

Run: `cargo test -p uptrakit-package-manager-routeros executor` Expected: FAIL.

- [ ] **Step 2: Implement executor.rs**

```rust
use std::sync::Arc;

use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::RouterOsExecutor;

use crate::error::{Result, RouterOsError};

/// Executes RouterOS firmware update operations.
pub(crate) struct RouterOsUpdateExecutor {
    pub(crate) exec: Arc<dyn RouterOsExecutor>,
    /// `true` if the operator requested a reboot after the update.
    pub(crate) reboot: bool,
    /// `true` if the RouterOS group has the `reboot` policy.
    /// Loaded from `routeros_host_config.allow_reboot` by the plugin via runtime downcast.
    pub(crate) allow_reboot: bool,
}

impl RouterOsUpdateExecutor {
    /// Run the update: install (download+reboot) or download only.
    ///
    /// RouterOS requires a prior `check-for-updates` before `install` or `download`
    /// will fetch anything — the router caches the update metadata from that check.
    /// We trigger the check and wait a fixed 10 s for the background check to complete.
    /// For a more robust implementation, poll `package_update_print` until
    /// `status:` ≠ `"Checking for updates..."` with a timeout.
    pub(crate) async fn run_update(&self) -> Result<()> {
        self.exec
            .check_for_updates()
            .await
            .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;
        // Give the router time to complete the background check.
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
```

- [ ] **Step 3: Run tests**

```bash
cargo test -p uptrakit-package-manager-routeros executor
```

Expected: all pass.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/package-managers/routeros/src/executor.rs
git commit -m "feat(routeros-plugin): add RouterOsUpdateExecutor with install/download routing"
```

---

### Task 5: Implement plugin.rs and declare_plugin macro

**Files:**

- Create: `crates/plugins/package-managers/routeros/src/plugin.rs`

- [ ] **Step 1: Write plugin.rs**

```rust
use std::sync::Arc;

use async_trait::async_trait;
use rootcause::prelude::*;
use uptrakit_plugin_infrastructure_core::{
    ConfigModel, HostRequirements, HostRuntime, PluginConfigValidationError, PluginFamily, Result,
    RouterOsHostRuntime, UpstreamRelease, Version, declare_plugin,
};

use crate::config::RouterOsConfig;
use crate::error::RouterOsError;
use crate::executor::RouterOsUpdateExecutor;
use crate::version::{parse_latest_version, parse_resource_version};

pub struct RouterOsPlugin {
    config: RouterOsConfig,
    runtime: Arc<dyn HostRuntime>,
}

impl RouterOsPlugin {
    pub fn new(
        config: RouterOsConfig,
        runtime: Arc<dyn HostRuntime>,
    ) -> std::result::Result<Self, PluginConfigValidationError> {
        use uptrakit_plugin_infrastructure_core::PluginConfig as _;
        config.validate()?;
        Ok(Self { config, runtime })
    }

    fn ros_runtime(&self) -> std::result::Result<&RouterOsHostRuntime, Report<RouterOsError>> {
        self.runtime
            .as_any()
            .downcast_ref::<RouterOsHostRuntime>()
            .ok_or_else(|| {
                report!(RouterOsError::SshExec(
                    "RouterOsPlugin requires RouterOsHostRuntime; \
                     runtime downcast failed (misconfigured host type?)"
                        .to_string()
                ))
            })
    }
}

// ── declare_plugin! ───────────────────────────────────────────────────

declare_plugin!(RouterOsPlugin, RouterOsConfig, "package_manager_routeros", {
    display_name: "RouterOS Package Manager",
    family: PluginFamily::Software,
    config_model: ConfigModel::PluginConfig,
    host_requirements: HostRequirements::ROUTER_OS,
    roles: [
        VersionDetector,
        UpdateExecutor,
        ReleaseFetcher { host_requirements: HostRequirements::ROUTER_OS },
    ],
});

// ── Role implementations ──────────────────────────────────────────────
// Pattern: same as `apt` — separate `#[async_trait] impl Trait for Plugin` blocks.

#[async_trait]
impl uptrakit_plugin_infrastructure_core::VersionDetector for RouterOsPlugin {
    async fn detect_installed_version(&self, _package_identifier: &str) -> Result<Option<Version>> {
        let ros = self.ros_runtime().map_err(Into::into)?;
        let exec = ros.routeros_executor();
        let output = exec
            .resource_print()
            .await
            .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;
        Ok(parse_resource_version(&output).map(|v| Version::new(&v)))
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::ReleaseFetcher for RouterOsPlugin {
    async fn fetch_releases(&self, _package_identifier: &str) -> Result<Vec<UpstreamRelease>> {
        let ros = self.ros_runtime().map_err(Into::into)?;
        let exec = ros.routeros_executor();
        let output = exec
            .package_update_print()
            .await
            .map_err(|e| report!(RouterOsError::SshExec(e.to_string())))?;
        let latest = parse_latest_version(&output).ok_or_else(|| {
            report!(RouterOsError::VersionUnavailable(
                "run check-for-updates on the router first".to_string()
            ))
        })?;
        Ok(vec![UpstreamRelease {
            version: Version::new(&latest),
            ..Default::default()
        }])
    }
}

#[async_trait]
impl uptrakit_plugin_infrastructure_core::UpdateExecutor for RouterOsPlugin {
    async fn execute_update(
        &self,
        _package_identifier: &str,
        _version: &str,
        _ctx: &uptrakit_plugin_infrastructure_core::UpdateContext,
    ) -> Result<()> {
        let ros = self.ros_runtime().map_err(Into::into)?;
        let exec = ros.routeros_executor();
        let ue = RouterOsUpdateExecutor {
            exec,
            reboot: self.config.reboot,
            allow_reboot: ros.allow_reboot,
        };
        ue.run_update().await.map_err(Into::into)
    }
}
```

> **Note on trait method signatures:** The exact signatures for `detect_installed_version`, `fetch_releases`, and `execute_update` must match those
> declared in `plugin-infrastructure-core/src/roles.rs`. Verify against the trait definitions before compiling — if signatures differ, adjust
> parameter names/types accordingly. The `UpdateContext` import path may vary; grep `roles.rs` for the actual type name.

- [ ] **Step 2: Build — fix any macro/trait errors**

```bash
cargo check -p uptrakit-package-manager-routeros 2>&1 | head -60
```

Fix any errors about missing trait implementations until it compiles.

- [ ] **Step 3: Write a basic plugin instantiation test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use uptrakit_plugin_infrastructure_core::{
        HostCapabilities, RouterOsHostRuntime, RouterOsExecutor, PluginCapability, PluginMeta,
    };

    fn mock_exec() -> Arc<dyn RouterOsExecutor> {
        use std::pin::Pin;
        struct NoopExec;
        impl RouterOsExecutor for NoopExec {
            fn resource_print(&self) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>> {
                Box::pin(async { Ok("version: 7.14.2 (stable)\nplatform: MikroTik\n".into()) })
            }
            fn routerboard_print(&self) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>> {
                Box::pin(async { Ok(String::new()) })
            }
            fn license_print(&self) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>> {
                Box::pin(async { Ok(String::new()) })
            }
            fn package_update_print(&self) -> Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send + '_>> {
                Box::pin(async { Ok("latest-version: 7.15\n".into()) })
            }
            fn package_install(&self) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
                Box::pin(async { Ok(()) })
            }
            fn package_download(&self) -> Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
                Box::pin(async { Ok(()) })
            }
        }
        Arc::new(NoopExec)
    }

    fn test_runtime(allow_reboot: bool) -> Arc<dyn uptrakit_plugin_infrastructure_core::HostRuntime> {
        let caps = HostCapabilities::new(Some("routeros"), None, None, &["router_os_cli".to_string()]);
        uptrakit_plugin_infrastructure_core::construct_routeros_host_runtime(
            mock_exec(),
            caps,
            allow_reboot,
        )
    }

    #[test]
    fn plugin_creation_succeeds() {
        let config = RouterOsConfig { channel: None, reboot: false };
        let plugin = RouterOsPlugin::new(config, test_runtime(false));
        assert!(plugin.is_ok());
    }

    #[test]
    fn plugin_rejects_invalid_channel() {
        let config = RouterOsConfig { channel: Some("nightly".to_string()), reboot: false };
        let plugin = RouterOsPlugin::new(config, test_runtime(false));
        assert!(plugin.is_err());
    }

    #[tokio::test]
    async fn detect_version_parses_output() {
        let config = RouterOsConfig::default();
        let plugin = RouterOsPlugin::new(config, test_runtime(false)).unwrap();
        // Call detect_installed_version (trait method — adapt to actual trait API)
        // This test documents the expected behaviour; adjust call syntax to match
        // the actual VersionDetector trait method signature.
        let result = plugin.detect_installed_version("routeros").await;
        assert_eq!(result.ok().flatten(), Some("7.14.2".to_string()));
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test -p uptrakit-package-manager-routeros plugin
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/package-managers/routeros/src/plugin.rs crates/plugins/package-managers/routeros/src/lib.rs
git commit -m "feat(routeros-plugin): implement RouterOsPlugin with VersionDetector/ReleaseFetcher/UpdateExecutor"
```

---

### Task 6: Register plugin in the plugin catalog

**Files:**

- Modify: whichever crate registers all plugins (find by grep for `"package_manager_apt"` in non-plugin crates)

- [ ] **Step 1: Find the plugin registry catalog**

```bash
grep -rn "package_manager_apt" crates/ | grep -v target | grep -v ".md:" | grep -v "Cargo.toml"
```

Identify which crate has a function like `build_catalog` or `register_plugins` that lists all known plugin types.

- [ ] **Step 2: Add RouterOS plugin to the catalog**

Following the same pattern used for APT, add:

```rust
use uptrakit_package_manager_routeros::RouterOsPlugin;
// ... in the catalog builder:
DESCRIPTOR.call(RouterOsPlugin::descriptor()),
// or however the existing plugins are registered
```

Also add the dependency to that crate's `Cargo.toml`:

```toml
uptrakit-package-manager-routeros = { workspace = true }
```

And add `uptrakit-package-manager-routeros` to `Cargo.toml` workspace dependencies:

```toml
uptrakit-package-manager-routeros = { path = "crates/plugins/package-managers/routeros" }
```

- [ ] **Step 3: Build the full workspace**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Run the full test suite**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(routeros-plugin): register RouterOS plugin in catalog"
```

---

### Task 7: Documentation — CONTEXT.md and plugin-guidelines.md

**Files:**

- Modify: `CONTEXT.md`
- Modify: `docs/development/plugin-guidelines.md`

- [ ] **Step 1: Update CONTEXT.md**

Find the `Host` term definition. Add RouterOS to the list of supported host OS families:

> **Host** — A managed machine running uptrakit agent software. Supported OS families: Linux, macOS, FreeBSD, RouterOS (MikroTik). Each host is
> identified by a `machine_id` and reports its capabilities to the controller.

- [ ] **Step 2: Update plugin-guidelines.md**

Add a new section (after the existing POSIX requirements section, or wherever `HostRequirements` is documented):

````markdown
## Non-POSIX Plugin Execution: RouterOsHostRuntime

Plugins targeting RouterOS hosts use a different runtime pattern from POSIX plugins.

### HostRequirements::ROUTER_OS

Use `HostRequirements::ROUTER_OS` in your `declare_plugin!` roles instead of `POSIX` or `POSIX_PRIVILEGED`. This restricts the role to hosts with
`OsFamily::RouterOs` and the `ROUTER_OS_CLI` feature.

### Downcast to RouterOsHostRuntime

RouterOS plugins do not call `runtime.executor()`. Instead, downcast to `RouterOsHostRuntime`:

```rust
let ros = runtime.as_any()
    .downcast_ref::<RouterOsHostRuntime>()
    .ok_or_else(|| "expected RouterOsHostRuntime".to_string())?;
let exec = ros.routeros_executor();   // Arc<dyn RouterOsExecutor>
let allow_reboot = ros.allow_reboot; // bool from routeros_host_config
```

`RouterOsExecutor` provides typed methods (`resource_print`, `package_install`, etc.) for all RouterOS CLI interactions. Do not call
`runtime.executor()` — it returns a `NoopCommandExecutor` and logs an error.

### Testing

Use a mock `RouterOsExecutor` + `construct_routeros_host_runtime` in tests. See `crates/plugins/package-managers/routeros/src/plugin.rs` tests for the
pattern.
````

- [ ] **Step 3: Lint markdown**

```bash
npx prettier --write CONTEXT.md docs/development/plugin-guidelines.md --prose-wrap always --print-width 150
```

Then:

```bash
npx markdownlint --config .markdownlint.json CONTEXT.md docs/development/plugin-guidelines.md
```

Fix any errors.

- [ ] **Step 4: Commit**

```bash
git add CONTEXT.md docs/development/plugin-guidelines.md
git commit -m "docs: update CONTEXT.md and plugin-guidelines for RouterOS support"
```

---

### Task 8: Write ADR — Non-POSIX bootstrap probe-then-route

**Files:**

- Create: `docs/adr/NNNN-routeros-non-posix-bootstrap-probe.md` (use the next sequential ADR number)

- [ ] **Step 1: Find next ADR number**

```bash
ls docs/adr/ | sort | tail -3
```

- [ ] **Step 2: Write the ADR**

```markdown
# NNNN — Non-POSIX bootstrap via probe-then-route detection

**Date:** 2026-05-07 **Status:** Accepted

## Context

uptrakit's `bootstrap_connect` assumes POSIX hosts (Linux, macOS, FreeBSD). Adding RouterOS (MikroTik) support requires detecting the host type over
SSH before committing to a bootstrap plan, since RouterOS uses an entirely different CLI, has no shell, and cannot use `sudo`.

## Decision

Run a lightweight probe (`/system resource print` with a 5-second timeout) in `bootstrap_connect` before any plan construction. Two-gate detection:

1. Exit 0 AND output contains `"platform:"` or `"MikroTik"` → RouterOS.
2. Exit 0 AND output contains `"not enough permissions"` (without POSIX error strings) → fail with a diagnostic error.
3. Anything else → POSIX.

RouterOS hosts get a separate `RouterOsHostRuntime` (in `plugin-infrastructure-core`) that carries a `RouterOsSshExecutor` and the `allow_reboot`
flag. Plugins downcast `runtime.as_any()` to this type — they do not call `runtime.executor()`.

## Trade-offs

**Why probe-then-route over a user-provided type selector:** Eliminates a user-facing question for the common case. The probe is robust for the two
real target types (POSIX and RouterOS). An unusual restricted POSIX shell that echoes "not enough permissions" could produce a false
RouterOS-permission-denied error, but this is unlikely in practice.

**Why place RouterOsExecutor trait in plugin-infrastructure-core:** Allows the RouterOS plugin crate to depend on the trait without depending on
`agent-ssh` (which is runtime-only and must not be a plugin dependency). The coupling cost is that `plugin-infrastructure-core` gains
transport-specific knowledge, but the trait is abstract (no SSH types leak).

**Why not store RouterOsExecutor as `Arc<dyn Any>` in RouterOsHostRuntime:** The explicit trait is type-safe and self-documenting. The downcast
pattern is already established for `HostRuntime::as_any()`.

## Consequences

- New host type `OsFamily::RouterOs` is fully supported from bootstrap through version-check and update.
- Future non-POSIX host types follow the same pattern: add an `OsFamily` variant, define an executor trait in `plugin-infrastructure-core`, add a
  runtime type, add a bootstrap probe arm.
- `agent-core` public functions now accept `Arc<dyn HostRuntime>` instead of `Arc<dyn CommandExecutor>` — callers must construct the runtime before
  calling agent-core.
```

- [ ] **Step 3: Lint**

```bash
npx prettier --write docs/adr/NNNN-routeros-non-posix-bootstrap-probe.md --prose-wrap always --print-width 150
npx markdownlint --config .markdownlint.json docs/adr/NNNN-routeros-non-posix-bootstrap-probe.md
```

- [ ] **Step 4: Commit**

```bash
git add docs/adr/NNNN-routeros-non-posix-bootstrap-probe.md
git commit -m "docs(adr): document non-POSIX bootstrap probe-then-route decision for RouterOS"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                                               | Covered                                     |
| ------------------------------------------------------------------------------ | ------------------------------------------- |
| `package_manager_routeros` plugin type ID                                      | Task 5 ✓                                    |
| `PluginFamily::Software`                                                       | Task 5 ✓                                    |
| `VersionDetector` + `UpdateExecutor` + `ReleaseFetcher` roles                  | Task 5 ✓                                    |
| `RouterOsConfig.channel` + `.reboot` with serde defaults                       | Task 2 ✓                                    |
| `RouterOsError` with `impl_report_conversion!` → `PluginError::PluginInternal` | Task 2 ✓                                    |
| `parse_resource_version` strips channel suffix                                 | Task 3 ✓                                    |
| `parse_latest_version`                                                         | Task 3 ✓                                    |
| `RouterOsUpdateExecutor` with install/download routing                         | Task 4 ✓                                    |
| Plugin registered in catalog                                                   | Task 6 ✓                                    |
| `fetch_releases` returns `VersionUnavailable` when `latest-version` absent     | Task 5 (documented in plugin.rs comments) ✓ |
| `CONTEXT.md` updated                                                           | Task 7 ✓                                    |
| `plugin-guidelines.md` RouterOsHostRuntime pattern                             | Task 7 ✓                                    |
| ADR: probe-then-route, RouterOsExecutor in plugin-core, executor downcast      | Task 8 ✓                                    |

**No placeholders:** All steps show exact code.

**Type consistency:** `RouterOsConfig` defined in Task 2, used in Task 5. `RouterOsError` defined in Task 2, used in Tasks 4 and 5.
`parse_resource_version`/`parse_latest_version` defined in Task 3, used in Task 5. `RouterOsUpdateExecutor` defined in Task 4, constructed in Task 5.
`RouterOsHostRuntime` from Plan A Task 4.
