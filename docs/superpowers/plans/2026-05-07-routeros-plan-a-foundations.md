# RouterOS Support — Plan A: Foundations

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement
> this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish the cross-crate infrastructure that every other RouterOS plan depends on: executor split, SFTP primitives, `RouterOsExecutor`
trait + `RouterOsHostRuntime`, `HostRequirements::ROUTER_OS`, and a refactored `agent-core` API that accepts `Arc<dyn HostRuntime>` instead of
`Arc<dyn CommandExecutor>`.

**Architecture:** The current `agent-core` builds its own `StandardHostRuntime` internally from a raw `CommandExecutor`. This plan inverts that —
callers pass the already-constructed runtime, so `agent-ssh` can inject `RouterOsHostRuntime` later (Plan B). Concurrently, the SSH executor is split
into a POSIX-specific wrapper and a raw base, and the RouterOS runtime types are defined in `plugin-infrastructure-core` so both `agent-ssh` and the
plugin crate can share them without a circular dependency.

**Tech Stack:** Rust (edition 2024), tokio, async-trait, sea-orm, russh 0.60, russh-sftp (new workspace dep), rootcause, thiserror

---

## Tasks

### Task 1: Remove "groundwork" comments from OsFamily::RouterOs and ROUTER_OS_CLI

**Files:**

- Modify: `crates/shared/types/src/os_family.rs`
- Modify: `crates/shared/types/src/host_feature.rs`

- [ ] **Step 1: Update OsFamily::RouterOs doc comment**

In `os_family.rs`, replace:

```rust
/// Groundwork only — no runtime implementation yet.
RouterOs,
```

with:

```rust
/// MikroTik RouterOS. Detected via `/system resource print` during bootstrap.
RouterOs,
```

- [ ] **Step 2: Update ROUTER_OS_CLI doc comment**

In `host_feature.rs`, replace the existing "Groundwork only" doc comment above the `ROUTER_OS_CLI` constant with:

```rust
/// RouterOS CLI available. Set during bootstrap when `/system resource print` succeeds.
pub const ROUTER_OS_CLI: HostFeature = HostFeature::from_static("router_os_cli");
```

- [ ] **Step 3: Verify no tests regress**

```bash
cargo test -p uptrakit-shared-types
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/shared/types/src/os_family.rs crates/shared/types/src/host_feature.rs
git commit -m "chore(types): remove groundwork-only stubs from OsFamily::RouterOs and ROUTER_OS_CLI"
```

---

### Task 2: Split SshCommandExecutor into base + PosixSshCommandExecutor

**Files:**

- Modify: `crates/core/agent-ssh/src/ssh_executor.rs`
- Modify: `crates/core/agent-ssh/src/operations/bootstrap.rs` (call site rename)
- Modify: `crates/core/agent-ssh/src/operations/sync.rs` (call site rename)
- Modify: `crates/core/agent-ssh/src/commands/bootstrap.rs` (call site rename)
- Modify: `crates/core/agent-ssh/src/commands/sync.rs` (call site rename)
- Modify: `crates/core/agent-ssh/src/client.rs` (call site rename)

- [ ] **Step 1: Add the base SshCommandExecutor and rename the existing struct**

At the top of `ssh_executor.rs`, BEFORE the existing `SshCommandExecutor` struct, add the new base struct. Then rename the existing
`SshCommandExecutor` to `PosixSshCommandExecutor`:

```rust
//! SSH-backed [`CommandExecutor`] implementations.
//!
//! [`SshCommandExecutor`] is the raw base: exec channel, no POSIX assumptions.
//! [`PosixSshCommandExecutor`] wraps the base and implements [`CommandExecutor`]
//! for POSIX hosts.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rootcause::prelude::*;
use tokio::sync::mpsc;
use uptrakit_command::{
    CommandError, CommandExecutor, CommandOutput, CommandSpec, StdioTunnel, UpdateOutputLine,
};

use crate::ssh_stdio_tunnel::SshStdioTunnel;
use crate::ssh_transport::{SshExecError, SshSession};

// ── Base executor ─────────────────────────────────────────────────────

/// Raw SSH exec channel. No POSIX assumptions.
///
/// Used by both `PosixSshCommandExecutor` and `RouterOsSshExecutor`.
pub(crate) struct SshCommandExecutor {
    session: Arc<SshSession>,
}

impl SshCommandExecutor {
    pub(crate) fn new(session: Arc<SshSession>) -> Self {
        Self { session }
    }

    /// Execute a pre-formed command string and collect stdout+stderr.
    ///
    /// Returns raw stdout. Does not apply shell quoting or env-var handling.
    pub(crate) async fn exec_raw(
        &self,
        cmd: &str,
        timeout: Option<Duration>,
    ) -> Result<String, SshExecError> {
        self.session.exec_raw(cmd, timeout).await
    }
}

// ── POSIX executor ────────────────────────────────────────────────────

/// Executes POSIX commands on a remote host via an SSH session.
///
/// Wraps [`SshCommandExecutor`] and implements [`CommandExecutor`], applying
/// shell quoting, env-var handling, and sudo context.
pub(crate) struct PosixSshCommandExecutor {
    inner: SshCommandExecutor,
}

impl PosixSshCommandExecutor {
    pub(crate) fn new(session: Arc<SshSession>) -> Self {
        Self {
            inner: SshCommandExecutor::new(session),
        }
    }
}
```

Then rename all existing methods that were on `SshCommandExecutor` (i.e., `run_remote`, the `CommandExecutor` impl, and the test functions) to be on
`PosixSshCommandExecutor`. The `run_remote` helper delegates through `self.inner.exec_raw` or directly calls
`self.inner.session.exec_command_streaming` (same as before — see note below).

> **Note:** The existing `run_remote` on `SshCommandExecutor` calls `self.session.exec_command_streaming`. After the rename,
> `PosixSshCommandExecutor.run_remote` calls `self.inner.session.exec_command_streaming`. Add a `pub(crate) fn session(&self) -> &Arc<SshSession>`
> accessor on `SshCommandExecutor` so `PosixSshCommandExecutor` can reach the session.

- [ ] **Step 2: Add exec_raw to SshSession**

In `crates/core/agent-ssh/src/ssh_transport.rs`, add a new `pub(crate) async fn exec_raw` method that executes a command string without POSIX command
building and returns the raw stdout as `Result<String, SshExecError>`. Define `SshExecError` as:

```rust
/// Error from a raw SSH exec or SFTP operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum SshExecError {
    #[error("SSH exec failed: {0}")]
    Exec(String),
    #[error("SSH exec timed out")]
    TimedOut,
}
```

Implement `exec_raw` on `SshSession`:

```rust
pub(crate) async fn exec_raw(
    &self,
    cmd: &str,
    timeout: Option<Duration>,
) -> Result<String, SshExecError> {
    let fut = self.exec_command(cmd);
    let result = if let Some(dur) = timeout {
        tokio::time::timeout(dur, fut)
            .await
            .map_err(|_| SshExecError::TimedOut)?
            .map_err(|e| SshExecError::Exec(e.to_string()))?
    } else {
        fut.await.map_err(|e| SshExecError::Exec(e.to_string()))?
    };
    let mut out = result.stdout;
    out.push_str(&result.stderr);
    Ok(out)
}
```

- [ ] **Step 3: Update all call sites from SshCommandExecutor::new to PosixSshCommandExecutor::new**

Run:

```bash
grep -rn "SshCommandExecutor::new" crates/core/agent-ssh/src/ | grep -v "ssh_executor.rs"
```

For each hit, replace `SshCommandExecutor::new(...)` with `PosixSshCommandExecutor::new(...)` and add the import
`use crate::ssh_executor::PosixSshCommandExecutor;` if not already present.

The files to update (from the grep): `operations/bootstrap.rs`, `operations/sync.rs`, `commands/bootstrap.rs`, `commands/sync.rs`, `client.rs`.

- [ ] **Step 4: Build to verify no regressions**

```bash
cargo check -p uptrakit-agent-ssh --all-features 2>&1 | head -40
```

Expected: compiles cleanly.

- [ ] **Step 5: Run existing tests**

```bash
cargo test -p uptrakit-agent-ssh
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/core/agent-ssh/src/
git commit -m "refactor(agent-ssh): split SshCommandExecutor into base + PosixSshCommandExecutor"
```

---

### Task 3: Add SFTP primitives (sftp_put / sftp_remove) to SshSession and SshCommandExecutor

**Files:**

- Modify: `Cargo.toml` (workspace root) — add `russh-sftp` workspace dep
- Modify: `crates/core/agent-ssh/Cargo.toml` — add `russh-sftp = { workspace = true }`
- Modify: `crates/core/agent-ssh/src/ssh_transport.rs` — add `sftp_put`, `sftp_remove`
- Modify: `crates/core/agent-ssh/src/ssh_executor.rs` — add `sftp_put`, `sftp_remove` to `SshCommandExecutor`

- [ ] **Step 1: Add russh-sftp to workspace Cargo.toml**

In the `[workspace.dependencies]` section of root `Cargo.toml`, add:

```toml
russh-sftp = { version = "2" }
```

(Verify on crates.io that `russh-sftp 2.x` is compatible with `russh 0.60`. At time of writing, `russh-sftp 2.0.3` works with `russh 0.60`.)

In `crates/core/agent-ssh/Cargo.toml` `[dependencies]`:

```toml
russh-sftp = { workspace = true }
```

- [ ] **Step 2: Add sftp_put and sftp_remove to SshSession**

In `ssh_transport.rs`, add these methods on `SshSession`:

```rust
/// Upload `data` bytes to `remote_path` via an SFTP subsystem channel.
///
/// Opens a new SFTP channel on the existing session — does not open a second
/// SSH connection. The caller is responsible for ensuring `remote_path` is a
/// valid absolute path on the remote host.
pub(crate) async fn sftp_put(
    &self,
    remote_path: &str,
    data: &[u8],
) -> Result<(), SshExecError> {
    use russh_sftp::client::SftpSession;
    use tokio::io::AsyncWriteExt as _;

    let channel = self
        .handle
        .channel_open_session()
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP channel open failed: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP subsystem request failed: {e}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP session init failed: {e}")))?;

    let mut file = sftp
        .create(remote_path)
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP create '{remote_path}' failed: {e}")))?;
    file.write_all(data)
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP write to '{remote_path}' failed: {e}")))?;
    file.shutdown()
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP flush '{remote_path}' failed: {e}")))?;

    Ok(())
}

/// Delete `remote_path` via SFTP.
pub(crate) async fn sftp_remove(&self, remote_path: &str) -> Result<(), SshExecError> {
    use russh_sftp::client::SftpSession;

    let channel = self
        .handle
        .channel_open_session()
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP channel open failed: {e}")))?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP subsystem request failed: {e}")))?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP session init failed: {e}")))?;

    sftp.remove_file(remote_path)
        .await
        .map_err(|e| SshExecError::Exec(format!("SFTP remove '{remote_path}' failed: {e}")))?;

    Ok(())
}
```

> **Note:** `SshSession.handle` is `Handle<BootstrapHandler>` which is `pub(crate)` or private within the module. The SFTP methods must be
> `impl SshSession` blocks in the same file (`ssh_transport.rs`).

- [ ] **Step 3: Add sftp_put / sftp_remove delegation on SshCommandExecutor**

In `ssh_executor.rs`, on the `SshCommandExecutor` struct:

```rust
/// Upload `data` to `remote_path` via SFTP.
pub(crate) async fn sftp_put(
    &self,
    remote_path: &str,
    data: &[u8],
) -> Result<(), crate::ssh_transport::SshExecError> {
    self.session.sftp_put(remote_path, data).await
}

/// Delete `remote_path` via SFTP.
pub(crate) async fn sftp_remove(
    &self,
    remote_path: &str,
) -> Result<(), crate::ssh_transport::SshExecError> {
    self.session.sftp_remove(remote_path).await
}
```

- [ ] **Step 4: Build to verify SFTP compiles**

```bash
cargo check -p uptrakit-agent-ssh --all-features 2>&1 | head -40
```

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/core/agent-ssh/Cargo.toml crates/core/agent-ssh/src/ssh_transport.rs crates/core/agent-ssh/src/ssh_executor.rs
git commit -m "feat(agent-ssh): add SFTP sftp_put/sftp_remove to SshSession and SshCommandExecutor"
```

---

### Task 4: Define RouterOsExecutor trait + RouterOsHostRuntime in plugin-infrastructure-core

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/host_runtime.rs`
- Modify: `crates/plugins/infrastructure/core/src/lib.rs`

- [ ] **Step 1: Define the RouterOsExecutor trait**

In `host_runtime.rs`, after the `MetadataAwareHostRuntime` block, add:

```rust
// ── RouterOS runtime ─────────────────────────────────────────────────

/// Typed RouterOS CLI methods used by the RouterOS plugin.
///
/// Defined here (in `plugin-infrastructure-core`) so that the plugin crate
/// can depend on it without depending on `agent-ssh`. `RouterOsSshExecutor`
/// in `agent-ssh` implements this trait.
///
/// All methods return raw stdout from the RouterOS CLI. Parsing is the
/// caller's responsibility.
#[async_trait::async_trait]
pub trait RouterOsExecutor: Send + Sync + 'static {
    /// `/system resource print`
    async fn resource_print(&self) -> std::result::Result<String, PluginError>;

    /// `/system routerboard print`
    async fn routerboard_print(&self) -> std::result::Result<String, PluginError>;

    /// `/system license print`
    async fn license_print(&self) -> std::result::Result<String, PluginError>;

    /// `/system package update check-for-updates` — triggers an async background
    /// check on the router. RouterOS caches the result; subsequent `package_update_print`
    /// calls will show `latest-version` once the check completes. Callers must
    /// wait (poll or fixed delay) before calling `package_install`/`package_download`.
    async fn check_for_updates(&self) -> std::result::Result<(), PluginError>;

    /// `/system package update print`
    async fn package_update_print(&self) -> std::result::Result<String, PluginError>;

    /// `/system package update install` — downloads + reboots.
    async fn package_install(&self) -> std::result::Result<(), PluginError>;

    /// `/system package update download` — downloads without rebooting.
    async fn package_download(&self) -> std::result::Result<(), PluginError>;
}
```

> **Note:** `async-trait` is already a workspace dependency and `plugin-infrastructure-core` uses it (e.g. in `roles.rs`). Use
> `#[async_trait::async_trait]` here for consistency. Error type is `PluginError` (from this crate) to avoid `Result<T, String>` which violates
> `coding-standards.md#error-handling`. Implementors map their internal errors to `PluginError::PluginInternal(msg)`.

- [ ] **Step 2: Add RouterOsHostRuntime struct**

After the `RouterOsExecutor` trait, add:

```rust
/// Host runtime for MikroTik RouterOS devices.
///
/// Carries the typed RouterOS CLI executor and the `allow_reboot` flag read
/// from `routeros_host_config` by `agent-ssh` at construction time.
///
/// The RouterOS plugin downcasts `runtime.as_any()` to this type to access
/// both the executor and the flag. No DB access is needed in the plugin crate.
pub struct RouterOsHostRuntime {
    routeros_exec: Arc<dyn RouterOsExecutor>,
    capabilities: HostCapabilities,
    /// Whether the RouterOS `reboot` policy was granted at bootstrap time.
    /// Hard gate: reboot is impossible without the policy on the router group.
    pub allow_reboot: bool,
}

impl RouterOsHostRuntime {
    pub fn new(
        routeros_exec: Arc<dyn RouterOsExecutor>,
        caps: HostCapabilities,
        allow_reboot: bool,
    ) -> Self {
        Self {
            routeros_exec: routeros_exec,
            capabilities: caps,
            allow_reboot,
        }
    }

    /// The typed RouterOS CLI executor.
    pub fn routeros_executor(&self) -> Arc<dyn RouterOsExecutor> {
        Arc::clone(&self.routeros_exec)
    }
}

impl HostRuntime for RouterOsHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> Arc<dyn CommandExecutor> {
        // RouterOS does not use the generic CommandExecutor interface.
        // Return a NoopCommandExecutor so misuse is visible in traces
        // without crashing the process.
        tracing::error!(
            "RouterOsHostRuntime::executor() called — use routeros_executor() instead"
        );
        Arc::new(uptrakit_command::NoopCommandExecutor)
    }
}

/// Construct a `RouterOsHostRuntime` for a RouterOS host.
///
/// Called by `agent-ssh` instead of `construct_host_runtime` when the host
/// has a `routeros_host_config` DB row.
pub fn construct_routeros_host_runtime(
    routeros_exec: Arc<dyn RouterOsExecutor>,
    caps: HostCapabilities,
    allow_reboot: bool,
) -> Arc<dyn HostRuntime> {
    Arc::new(RouterOsHostRuntime::new(routeros_exec, caps, allow_reboot))
}
```

- [ ] **Step 3: Re-export new items from lib.rs**

In `crates/plugins/infrastructure/core/src/lib.rs`, add to the re-exports:

```rust
pub use host_runtime::{
    RouterOsExecutor,
    RouterOsHostRuntime,
    construct_routeros_host_runtime,
    // ... existing re-exports remain unchanged
};
```

- [ ] **Step 4: Write tests for RouterOsHostRuntime**

At the end of `host_runtime.rs` tests module:

```rust
#[async_trait::async_trait]
impl RouterOsExecutor for MockExec {
    async fn resource_print(&self) -> std::result::Result<String, PluginError> {
        Ok("version: 7.14\n".into())
    }
    async fn routerboard_print(&self) -> std::result::Result<String, PluginError> {
        Ok("serial-number: ABC123\n".into())
    }
    async fn license_print(&self) -> std::result::Result<String, PluginError> {
        Ok("software-id: XXXX-YYYY\n".into())
    }
    async fn check_for_updates(&self) -> std::result::Result<(), PluginError> {
        Ok(())
    }
    async fn package_update_print(&self) -> std::result::Result<String, PluginError> {
        Ok("latest-version: 7.15\n".into())
    }
    async fn package_install(&self) -> std::result::Result<(), PluginError> {
        Ok(())
    }
    async fn package_download(&self) -> std::result::Result<(), PluginError> {
        Ok(())
    }
}

#[test]
fn routeros_runtime_executor_returns_noop() {
    struct MockExec;
    #[async_trait::async_trait]
    impl RouterOsExecutor for MockExec {
        async fn resource_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn routerboard_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn license_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn check_for_updates(&self) -> std::result::Result<(), PluginError> { Ok(()) }
        async fn package_update_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn package_install(&self) -> std::result::Result<(), PluginError> { Ok(()) }
        async fn package_download(&self) -> std::result::Result<(), PluginError> { Ok(()) }
    }

    let exec: Arc<dyn RouterOsExecutor> = Arc::new(MockExec);
    let caps = HostCapabilities::new(Some("routeros"), None, None, &[]);
    let runtime = RouterOsHostRuntime::new(exec, caps, true);
    // executor() must not panic
    let _noop = runtime.executor();
    assert!(runtime.allow_reboot);
}

#[test]
fn construct_routeros_host_runtime_downcast() {
    struct MockExec;
    #[async_trait::async_trait]
    impl RouterOsExecutor for MockExec {
        async fn resource_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn routerboard_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn license_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn check_for_updates(&self) -> std::result::Result<(), PluginError> { Ok(()) }
        async fn package_update_print(&self) -> std::result::Result<String, PluginError> { Ok(String::new()) }
        async fn package_install(&self) -> std::result::Result<(), PluginError> { Ok(()) }
        async fn package_download(&self) -> std::result::Result<(), PluginError> { Ok(()) }
    }
    let runtime = construct_routeros_host_runtime(
        Arc::new(MockExec),
        HostCapabilities::new(Some("routeros"), None, None, &[]),
        false,
    );
    let downcast = runtime.as_any().downcast_ref::<RouterOsHostRuntime>();
    assert!(downcast.is_some());
    assert!(!downcast.unwrap().allow_reboot);
}
```

- [ ] **Step 5: Build and test**

```bash
cargo test -p uptrakit-plugin-infrastructure-core
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/plugins/infrastructure/core/src/
git commit -m "feat(plugin-core): add RouterOsExecutor trait and RouterOsHostRuntime"
```

---

### Task 5: Add HostRequirements::ROUTER_OS constant

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/host_requirements.rs`

- [ ] **Step 1: Add ROUTER_OS_CLI static feature array**

In the `feature_arrays` module, add:

```rust
pub(super) static ROUTER_OS_CLI: [HostFeature; 1] = [host_features::ROUTER_OS_CLI];
```

- [ ] **Step 2: Add ROUTER_OS constant**

In the `impl HostRequirements` block, after `POSIX_PRIVILEGED`, add:

```rust
/// RouterOS host with CLI access.
///
/// Assigned to `VersionDetector` and `UpdateExecutor` roles of the RouterOS
/// package-manager plugin. Fails for any POSIX host — the `OsFamily::RouterOs`
/// requirement ensures the plugin only runs on MikroTik devices.
pub const ROUTER_OS: Self = Self::new(
    &[OsFamily::RouterOs],
    &feature_arrays::ROUTER_OS_CLI,
    false,
);
```

- [ ] **Step 3: Add test**

In the `#[cfg(test)]` module of `host_requirements.rs`:

```rust
#[test]
fn router_os_compatible_with_routeros_cli_host() {
    use uptrakit_shared_types::host_features;
    let caps = HostCapabilities {
        os_family: Some(OsFamily::RouterOs),
        features: [host_features::ROUTER_OS_CLI].iter().cloned().collect(),
        ..Default::default()
    };
    assert!(HostRequirements::ROUTER_OS.is_compatible_with(&caps).is_ok());
}

#[test]
fn router_os_incompatible_with_linux() {
    let caps = HostCapabilities {
        os_family: Some(OsFamily::Linux),
        features: BTreeSet::new(),
        ..Default::default()
    };
    assert!(HostRequirements::ROUTER_OS.is_compatible_with(&caps).is_err());
}
```

- [ ] **Step 4: Test**

```bash
cargo test -p uptrakit-plugin-infrastructure-core host_requirements
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/host_requirements.rs
git commit -m "feat(plugin-core): add HostRequirements::ROUTER_OS constant"
```

---

### Task 6: Refactor agent-core version_check.rs to accept `Arc<dyn HostRuntime>`

**Files:**

- Modify: `crates/shared/agent-core/src/version_check.rs`

This is a mechanical rename. Every function that currently takes `executor: Arc<dyn CommandExecutor>` and calls
`construct_host_runtime(executor, HostCapabilities::default())` is changed to take `runtime: Arc<dyn HostRuntime>` and use it directly.

- [ ] **Step 1: Update imports in version_check.rs**

Remove `HostCapabilities`, `construct_host_runtime` from imports. Add `HostRuntime` to imports from `uptrakit_plugin_infrastructure_registry`.

Change:

```rust
use uptrakit_plugin_infrastructure_registry::{
    ExecuteUpdateResult, HostCapabilities, PluginError, UpdateLifecycleContext,
    construct_host_runtime, get_descriptor,
};
```

To:

```rust
use uptrakit_plugin_infrastructure_registry::{
    ExecuteUpdateResult, HostRuntime, PluginError, UpdateLifecycleContext,
    get_descriptor,
};
```

Also add (if not present): `use uptrakit_command::CommandExecutor;` (still needed for `refresh_package_indexes`).

- [ ] **Step 2: Change all private function signatures**

These functions change `executor: Arc<dyn CommandExecutor>` → `runtime: Arc<dyn HostRuntime>` and remove the `construct_host_runtime(executor, ...)`
call, using `runtime` directly:

| Function                                                          | Change                                                                                   |
| ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `run_detect_group(group, executor)`                               | → `run_detect_group(group, runtime: Arc<dyn HostRuntime>)`                               |
| `refresh_package_indexes(groups, executor)`                       | → `refresh_package_indexes(groups, runtime: &Arc<dyn HostRuntime>)`                      |
| `detect_installed(assignment, executor, ctx)`                     | → `detect_installed(assignment, runtime: Arc<dyn HostRuntime>, ctx)`                     |
| `fetch_latest(assignment, executor, ctx)`                         | → `fetch_latest(assignment, runtime: Arc<dyn HostRuntime>, ctx)`                         |
| `batch_check_versions_inner(assignments, executor, ctx, factory)` | → `batch_check_versions_inner(assignments, runtime: Arc<dyn HostRuntime>, ctx, factory)` |

Example: `run_detect_group` before:

```rust
async fn run_detect_group(group: BatchGroup, executor: Arc<dyn CommandExecutor>) -> Vec<(usize, DetectItemResult)> {
    // ...
    let runtime = construct_host_runtime(executor, HostCapabilities::default());
    let slot = ...;
    let detector = (slot.create)(&group.effective_config, runtime) ...
```

After:

```rust
async fn run_detect_group(group: BatchGroup, runtime: Arc<dyn HostRuntime>) -> Vec<(usize, DetectItemResult)> {
    // ...
    let slot = ...;
    let detector = (slot.create)(&group.effective_config, runtime) ...
```

Apply the same pattern to all other listed functions. Remove every `let runtime = construct_host_runtime(executor, HostCapabilities::default());`
line.

- [ ] **Step 3: Change the public batch_check_versions signature**

```rust
pub async fn batch_check_versions(
    assignments: Vec<VersionCheckAssignment>,
    runtime: Arc<dyn HostRuntime>,
    ctx: &ConnectionContext,
) -> Vec<VersionCheckResult> {
    batch_check_versions_inner(assignments, runtime, ctx, &default_fetcher_factory).await
}
```

- [ ] **Step 4: Update test helpers in version_check.rs**

The test `fn test_executor()` builds `Arc<dyn CommandExecutor>`. Change to:

```rust
fn test_runtime() -> Arc<dyn HostRuntime> {
    use uptrakit_plugin_infrastructure_core::{HostCapabilities, StandardHostRuntime};
    use uptrakit_command::NoopCommandExecutor;
    Arc::new(StandardHostRuntime::new(
        Arc::new(NoopCommandExecutor),
        HostCapabilities::default(),
    ))
}
```

Update all test call sites from `test_executor()` to `test_runtime()`.

- [ ] **Step 5: Check it compiles (expect failures in callers)**

```bash
cargo check -p uptrakit-agent-core 2>&1 | head -60
```

Expected: `version_check.rs` errors resolved, but callers (`client.rs`) will now fail — fix those in Task 8.

- [ ] **Step 6: Stage (do NOT merge until Task 8 is complete)**

> **Warning:** After this step, `uptrakit-agent-core` will not compile standalone because callers in `client.rs` and `update.rs` still pass
> `Arc<dyn CommandExecutor>`. Complete Tasks 6, 7, and 8 on the same feature branch before opening a PR. Do not merge a partially-refactored state —
> CI will fail.

```bash
git add crates/shared/agent-core/src/version_check.rs
git commit -m "refactor(agent-core): version_check accepts Arc<dyn HostRuntime> instead of Arc<dyn CommandExecutor>"
```

---

### Task 7: Refactor agent-core update.rs to accept `Arc<dyn HostRuntime>`

**Files:**

- Modify: `crates/shared/agent-core/src/update.rs`

Same pattern as Task 6 — mechanical rename throughout.

- [ ] **Step 1: Update imports in update.rs**

Remove `HostCapabilities`, `construct_host_runtime` from `uptrakit_plugin_infrastructure_registry` imports. Add `HostRuntime`.

- [ ] **Step 2: Change execute_update public signature**

```rust
pub async fn execute_update(
    payload: ExecuteUpdatePayload,
    runtime: Arc<dyn HostRuntime>,
    output_tx: mpsc::Sender<UpdateOutputMessage>,
    early_result_tx: tokio::sync::mpsc::UnboundedSender<UpdateResultPayload>,
) -> UpdateExecutionResult {
```

- [ ] **Step 3: Thread runtime through internal functions**

Every private function that currently takes `executor: Arc<dyn CommandExecutor>` and calls
`construct_host_runtime(executor, HostCapabilities::default())` changes to `runtime: Arc<dyn HostRuntime>`.

Functions to update (every function that takes `executor: Arc<dyn CommandExecutor>`):

- `execute_update_pipeline(payload, executor, output_tx, early_result_tx)`
- `run_pre_hook_plugins(plugins, executor, output_tx, ctx)` (called 2x in `execute_update_pipeline`)
- `execute_plugin_update(payload, output_tx, executor)` (line ~428)
- `run_post_hook_plugins(...)` (line ~776)
- `detect_current_version(payload, executor)` (line ~218) — calls `version_check::check_version`, pass `runtime` through
- `run_batch_pre_hook_plugins(plugins, executor, output_tx, ctx)` (line ~717)
- `run_batch_post_hook_plugins(plugins, executor, output_tx, ctx)` (line ~776)
- `execute_update_interactive(payload, executor, ...)` (line ~1164)
- `spawn_update_task(payload, executor, ...)` (line ~340 in client.rs — in the update dispatch path)

- [ ] **Step 4: Check compiles (callers in client.rs still broken — expected)**

```bash
cargo check -p uptrakit-agent-core 2>&1 | head -60
```

- [ ] **Step 5: Commit (still on the same feature branch — do not merge until Task 8 completes)**

```bash
git add crates/shared/agent-core/src/update.rs
git commit -m "refactor(agent-core): update.rs accepts Arc<dyn HostRuntime>"
```

---

### Task 8: Refactor agent-core client.rs public functions and fix all callers

**Files:**

- Modify: `crates/shared/agent-core/src/client.rs`
- Modify: `crates/core/agent-runtime/src/lib.rs`
- Modify: `crates/core/agent-ssh/src/client.rs`

- [ ] **Step 1: Update agent-core client.rs public function signatures**

```rust
pub async fn run_check_versions(
    payload: uptrakit_wire::CheckVersionsPayload,
    runtime: Arc<dyn HostRuntime>,
    ctx: &ConnectionContext,
) -> ServiceMessage {
    let results =
        crate::version_check::batch_check_versions(payload.assignments, runtime, ctx).await;
    ServiceMessage::VersionCheckResults(VersionCheckResultsPayload { results })
}

pub async fn start_update(
    payload: uptrakit_wire::ExecuteUpdatePayload,
    runtime: Arc<dyn HostRuntime>,
    conn: &mut dyn ServiceTransport,
    ctx: &ConnectionContext,
) -> InFlightUpdate { ... }

pub async fn run_discover_software(
    payload: DiscoverSoftwarePayload,
    base_runtime: Arc<dyn HostRuntime>,
    ctx: &ConnectionContext,
) -> ServiceMessage { ... }

pub async fn run_execute_batch_update(
    payload: uptrakit_wire::ExecuteBatchUpdatePayload,
    runtime: Arc<dyn HostRuntime>,
    conn: &mut dyn ServiceTransport,
    ctx: &ConnectionContext,
) -> ServiceMessage { ... }
```

Also update `spawn_update_task` to accept `runtime: Arc<dyn HostRuntime>` and pass to `execute_update`.

In `discover_software_inner`, the `base_runtime` is now a parameter (already constructed by the caller). The metadata wrapping stays:

```rust
async fn discover_software_inner(
    payload: &DiscoverSoftwarePayload,
    base_runtime: Arc<dyn HostRuntime>,
    ctx: &ConnectionContext,
) -> Vec<DiscoveryPluginResult> {
    for assignment in &payload.plugins {
        let runtime: Arc<dyn HostRuntime> = if let Some(ref provider) = ctx.metadata_provider {
            MetadataAwareHostRuntime::new(Arc::clone(&base_runtime), Arc::clone(provider))
        } else {
            Arc::clone(&base_runtime)
        };
        // ... rest unchanged
    }
}
```

- [ ] **Step 2: Update agent-runtime to wrap executor in StandardHostRuntime**

In `crates/core/agent-runtime/src/lib.rs`, add a helper:

```rust
fn make_standard_runtime(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
    use uptrakit_plugin_infrastructure_core::{HostCapabilities, StandardHostRuntime};
    Arc::new(StandardHostRuntime::new(executor, HostCapabilities::default()))
}
```

Then at every call site that passes `executor` to `run_check_versions`, `start_update`, `run_discover_software`, `run_execute_batch_update` — replace
with `make_standard_runtime(Arc::clone(&self.executor))` (or equivalent).

Example (around line 276):

```rust
// Before:
uptrakit_agent_core::run_check_versions(payload, executor, &ctx).await
// After:
uptrakit_agent_core::run_check_versions(payload, make_standard_runtime(executor), &ctx).await
```

Note: for `run_discover_software`, the `make_standard_runtime` call produces the `base_runtime`. The metadata wrapping still happens inside
`discover_software_inner` via `ctx.metadata_provider`. No change to how metadata is injected.

- [ ] **Step 3: Update agent-ssh client.rs call sites**

For every place in `crates/core/agent-ssh/src/client.rs` that currently passes `executor: Arc<dyn CommandExecutor>` to agent-core functions, replace
with:

```rust
use uptrakit_plugin_infrastructure_core::{HostCapabilities, StandardHostRuntime};

let caps = HostCapabilities::new(Some("linux"), None, None, &[]); // placeholder; RouterOS path added in Plan B
let runtime: Arc<dyn HostRuntime> = Arc::new(StandardHostRuntime::new(
    Arc::clone(&executor),
    caps,
));
uptrakit_agent_core::start_update(payload, runtime, conn, &ctx).await
```

> **Note:** Use `HostCapabilities::new(Some("linux"), ...)` as a placeholder for Plan A. Plan B will replace this with the correct runtime selection
> (POSIX vs RouterOS) per host.

- [ ] **Step 4: Full build to verify everything compiles**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -40
```

Expected: no errors.

- [ ] **Step 5: Run all tests**

```bash
cargo test --all-features 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/shared/agent-core/src/client.rs crates/core/agent-runtime/src/lib.rs crates/core/agent-ssh/src/client.rs
git commit -m "refactor(agent-core): public functions accept Arc<dyn HostRuntime> instead of Arc<dyn CommandExecutor>"
```

---

## Self-Review

**Spec coverage check:**

| Spec requirement                                                      | Covered     |
| --------------------------------------------------------------------- | ----------- |
| Remove "groundwork" stubs from OsFamily::RouterOs + ROUTER_OS_CLI     | Task 1 ✓    |
| SSH executor split: base SshCommandExecutor + PosixSshCommandExecutor | Task 2 ✓    |
| SFTP sftp_put / sftp_remove on SshCommandExecutor                     | Task 3 ✓    |
| RouterOsExecutor trait in plugin-infrastructure-core                  | Task 4 ✓    |
| RouterOsHostRuntime + construct_routeros_host_runtime                 | Task 4 ✓    |
| HostRequirements::ROUTER_OS constant                                  | Task 5 ✓    |
| agent-core accepts `Arc<dyn HostRuntime>`                             | Tasks 6–8 ✓ |

**No placeholders:** All steps show exact file paths and code.

**Type consistency:** `RouterOsExecutor` trait defined in Task 4 used in Tasks 4–8; `SshCommandExecutor` base defined in Task 2 used in Tasks 3 and
extended in Plan B Task 1.
