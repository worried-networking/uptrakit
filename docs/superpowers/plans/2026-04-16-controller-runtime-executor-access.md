# ControllerRuntime Executor Access — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `executor()` to `HostRuntime` so every runtime provides a command executor, rename `PosixHostRuntime`
to `StandardHostRuntime`, and make `ControllerRuntime` embed a real executor — fixing controller-side plugin
construction failures.

**Architecture:** `HostRuntime` gains `executor()` returning `Arc<dyn CommandExecutor>`. `PosixHostRuntime` is
renamed to `StandardHostRuntime` (platform-neutral). `ControllerRuntime` wraps a `StandardHostRuntime` with
`LocalCommandExecutor` internally. `require_posix_executor()` is removed — all 16 plugin call sites switch to
`runtime.executor()`.

**Tech Stack:** Rust, `uptrakit_plugin_infrastructure_core`, `uptrakit_command`, `async_trait`

---

## File Map

| File | Change |
| --- | --- |
| `crates/plugins/infrastructure/core/src/host_runtime.rs` | Rename struct, add trait method, remove `require_posix_executor()` |
| `crates/plugins/infrastructure/core/src/lib.rs` | Update re-exports |
| `crates/plugins/infrastructure/core/src/descriptor.rs` | `ControllerRuntime` gains `local_runtime`, delegates, `new_for_test` |
| `crates/plugins/infrastructure/core/src/testing.rs` | Rename `PosixHostRuntime` to `StandardHostRuntime` |
| 16 plugin `plugin.rs` files + 24 plugin test files | `runtime.executor()` + rename imports |

---

## Task 1: Rename `PosixHostRuntime` to `StandardHostRuntime` and add `executor()` to the trait

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/host_runtime.rs`

- [ ] **Step 1: Write a failing test for the new `executor()` trait method**

In `crates/plugins/infrastructure/core/src/host_runtime.rs`, replace the entire `#[cfg(test)] mod tests` block (lines 104–152) with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_shared_types::{OsFamily, host_features};

    #[test]
    fn standard_runtime_executor_returns_executor() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
        let runtime: Arc<dyn HostRuntime> =
            Arc::new(StandardHostRuntime::new(executor, caps));
        // Should return an executor via the trait method
        let _exec = runtime.executor();
    }

    #[test]
    fn standard_runtime_downcast() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(Some("linux"), None, None, &[]);
        let runtime: Arc<dyn HostRuntime> =
            Arc::new(StandardHostRuntime::new(executor, caps));

        let std_rt = runtime.as_any().downcast_ref::<StandardHostRuntime>();
        assert!(std_rt.is_some());
        assert_eq!(
            std_rt.unwrap().capabilities().os_family,
            Some(OsFamily::Linux)
        );
    }

    #[test]
    fn executor_via_construct_host_runtime() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(
            Some("linux"),
            None,
            None,
            &["posix_shell".to_string()],
        );
        let runtime = construct_host_runtime(executor, caps);
        let _exec = runtime.executor();
    }

    #[test]
    fn construct_host_runtime_preserves_capabilities() {
        let executor = Arc::new(uptrakit_command::NoopCommandExecutor);
        let caps = HostCapabilities::new(
            Some("linux"),
            Some("Ubuntu 24.04"),
            Some("x86_64"),
            &["posix_shell".to_string(), "systemd".to_string()],
        );
        let runtime = construct_host_runtime(executor, caps);

        assert_eq!(runtime.capabilities().os_family, Some(OsFamily::Linux));
        assert!(
            runtime
                .capabilities()
                .has_feature(host_features::POSIX_SHELL)
        );
        assert!(runtime.capabilities().has_feature(host_features::SYSTEMD));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-core standard_runtime 2>&1 | tail -10
```

Expected: `FAILED` — `StandardHostRuntime` is not defined.

- [ ] **Step 3: Apply the rename and add `executor()` to the trait**

Replace the entire content of `crates/plugins/infrastructure/core/src/host_runtime.rs` above the `#[cfg(test)]` block (lines 1–102) with:

```rust
//! Host runtime abstraction.
//!
//! [`HostRuntime`] makes the execution environment a first-class concept.
//! Plugins receive `Arc<dyn HostRuntime>` at creation time and call
//! `runtime.executor()` to obtain the command executor.
//!
//! [`StandardHostRuntime`] is the standard implementation wrapping a
//! `CommandExecutor` and `HostCapabilities`. It is platform-neutral — the
//! same struct is used for POSIX, Windows, and other host types.
//!
//! [`construct_host_runtime`] is the single dispatch point where runtime
//! type selection happens on the agent side.

use std::sync::Arc;

use uptrakit_command::CommandExecutor;
use uptrakit_shared_types::HostCapabilities;

/// Execution environment provided to plugins at creation time.
///
/// Plugins obtain the command executor via [`executor()`](HostRuntime::executor).
/// Host compatibility is enforced at dispatch time by `HostRequirements`, not
/// at construction time — plugins do not need to downcast to a specific runtime.
pub trait HostRuntime: Send + Sync + 'static {
    /// Host capabilities for runtime compatibility checks.
    fn capabilities(&self) -> &HostCapabilities;

    /// Downcast to the concrete runtime type.
    fn as_any(&self) -> &dyn std::any::Any;

    /// The command executor for this runtime.
    fn executor(&self) -> Arc<dyn CommandExecutor>;
}

/// Standard host runtime wrapping a command executor and capabilities.
///
/// Platform-neutral — used for POSIX, Windows (future), and other host types.
/// The executor implementation determines the actual command execution strategy
/// (local shell, SSH, device API, etc.).
pub struct StandardHostRuntime {
    executor: Arc<dyn CommandExecutor>,
    capabilities: HostCapabilities,
}

impl StandardHostRuntime {
    /// Create a new standard host runtime.
    pub fn new(executor: Arc<dyn CommandExecutor>, capabilities: HostCapabilities) -> Self {
        Self {
            executor,
            capabilities,
        }
    }
}

impl HostRuntime for StandardHostRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        &self.capabilities
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> Arc<dyn CommandExecutor> {
        Arc::clone(&self.executor)
    }
}

/// Construct the appropriate `HostRuntime` for a host based on its capabilities.
///
/// Currently always returns [`StandardHostRuntime`]. When non-standard host
/// types are added (e.g., RouterOS), this function dispatches based on
/// `caps.os_family`. This is the SINGLE point where runtime type selection
/// happens on the agent side.
pub fn construct_host_runtime(
    executor: Arc<dyn CommandExecutor>,
    caps: HostCapabilities,
) -> Arc<dyn HostRuntime> {
    // Future: match on caps.os_family to select the right runtime
    // e.g., Some(OsFamily::RouterOs) => Arc::new(RouterOsHostRuntime::new(session, caps))
    Arc::new(StandardHostRuntime::new(executor, caps))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-core standard_runtime 2>&1 | tail -10
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/host_runtime.rs
git commit -m "refactor(plugin-core): rename PosixHostRuntime to StandardHostRuntime, add executor() to HostRuntime trait"
```

---

## Task 2: Update `lib.rs` re-exports and `descriptor.rs` doc comment

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/lib.rs`
- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`

- [ ] **Step 1: Update `lib.rs` re-exports**

In `crates/plugins/infrastructure/core/src/lib.rs`, replace line 102–104:

```rust
pub use host_runtime::{
    HostRuntime, PosixHostRuntime, construct_host_runtime, require_posix_executor,
};
```

with:

```rust
pub use host_runtime::{HostRuntime, StandardHostRuntime, construct_host_runtime};
```

- [ ] **Step 2: Update `CreateRoleFn` doc comment in `descriptor.rs`**

In `crates/plugins/infrastructure/core/src/descriptor.rs`, replace the doc comment on lines 118–121:

```rust
/// Sync creation for a software/hook role.
///
/// Receives `Arc<dyn HostRuntime>` — NOT `Arc<dyn CommandExecutor>`.
/// POSIX plugins extract the executor via `require_posix_executor()`.
```

with:

```rust
/// Sync creation for a software/hook role.
///
/// Receives `Arc<dyn HostRuntime>` — NOT `Arc<dyn CommandExecutor>`.
/// Plugins extract the executor via `runtime.executor()`.
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check -p uptrakit-plugin-infrastructure-core --features catalog,testing 2>&1 | grep "^error" | head -20
```

Expected: no errors from this crate (downstream crates will fail until migrated).

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/infrastructure/core/src/lib.rs crates/plugins/infrastructure/core/src/descriptor.rs
git commit -m "refactor(plugin-core): update re-exports and doc comments for StandardHostRuntime"
```

---

## Task 3: Update `testing.rs`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/testing.rs`

- [ ] **Step 1: Update imports and type references**

In `crates/plugins/infrastructure/core/src/testing.rs`, replace line 20:

```rust
use crate::host_runtime::{HostRuntime, PosixHostRuntime};
```

with:

```rust
use crate::host_runtime::{HostRuntime, StandardHostRuntime};
```

- [ ] **Step 2: Update `test_runtime()` function body**

Replace lines 179–184:

```rust
pub fn test_runtime() -> Arc<dyn HostRuntime> {
    Arc::new(PosixHostRuntime::new(
        Arc::new(crate::LocalCommandExecutor),
        HostCapabilities::default(),
    ))
}
```

with:

```rust
pub fn test_runtime() -> Arc<dyn HostRuntime> {
    Arc::new(StandardHostRuntime::new(
        Arc::new(crate::LocalCommandExecutor),
        HostCapabilities::default(),
    ))
}
```

- [ ] **Step 3: Update `test_runtime_with_executor()` function body**

Replace lines 190–192:

```rust
pub fn test_runtime_with_executor(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
    Arc::new(PosixHostRuntime::new(executor, HostCapabilities::default()))
}
```

with:

```rust
pub fn test_runtime_with_executor(executor: Arc<dyn CommandExecutor>) -> Arc<dyn HostRuntime> {
    Arc::new(StandardHostRuntime::new(executor, HostCapabilities::default()))
}
```

- [ ] **Step 4: Verify compilation**

```bash
cargo check -p uptrakit-plugin-infrastructure-core --features testing 2>&1 | grep "^error" | head -5
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/testing.rs
git commit -m "refactor(plugin-core): update testing.rs to use StandardHostRuntime"
```

---

## Task 4: Update `ControllerRuntime` to embed `StandardHostRuntime`

**Files:**

- Modify: `crates/plugins/infrastructure/core/src/descriptor.rs`

- [ ] **Step 1: Write the failing test**

At the end of `crates/plugins/infrastructure/core/src/descriptor.rs`, add:

```rust
#[cfg(test)]
mod controller_runtime_tests {
    use super::*;
    use crate::host_runtime::HostRuntime;
    use uptrakit_command::NoopCommandExecutor;

    #[test]
    fn controller_runtime_provides_executor() {
        let rt = ControllerRuntime::new_for_test(
            CatalogConfig::default(),
            std::sync::Arc::new(NoopCommandExecutor),
        );
        let _exec = rt.executor();
    }

    #[test]
    fn controller_runtime_preserves_identity() {
        let rt = ControllerRuntime::new_for_test(
            CatalogConfig::default(),
            std::sync::Arc::new(NoopCommandExecutor),
        );
        let any = rt.as_any();
        assert!(
            any.downcast_ref::<ControllerRuntime>().is_some(),
            "as_any() should return ControllerRuntime, not the inner runtime"
        );
    }

    #[test]
    fn controller_runtime_catalog_config_accessible() {
        let rt = ControllerRuntime::new_for_test(
            CatalogConfig::default(),
            std::sync::Arc::new(NoopCommandExecutor),
        );
        let _config = rt.catalog_config();
    }

    #[test]
    fn production_new_provides_executor() {
        let rt = ControllerRuntime::new(CatalogConfig::default());
        let _exec = rt.executor();
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --features catalog controller_runtime 2>&1 | tail -10
```

Expected: `FAILED` — `new_for_test` and `executor()` do not exist on `ControllerRuntime`.

- [ ] **Step 3: Update `ControllerRuntime` struct and impl**

In `crates/plugins/infrastructure/core/src/descriptor.rs`, replace the `ControllerRuntime` section (lines 313–355) with:

```rust
// ── ControllerRuntime ───────────────────────────────────────────────────────

/// Runtime for controller-side plugins.
///
/// Wraps a [`StandardHostRuntime`] with [`LocalCommandExecutor`] for local
/// command execution. Carries shared resources from [`CatalogConfig`].
/// Controller-side per-instance roles (e.g., GitHub `ReleaseFetcher`) access
/// the executor via the [`HostRuntime::executor()`] trait method.
#[cfg(feature = "catalog")]
pub struct ControllerRuntime {
    local_runtime: std::sync::Arc<dyn crate::host_runtime::HostRuntime>,
    config: CatalogConfig,
}

#[cfg(feature = "catalog")]
impl ControllerRuntime {
    pub fn new(config: CatalogConfig) -> Self {
        let local_runtime: std::sync::Arc<dyn crate::host_runtime::HostRuntime> =
            std::sync::Arc::new(crate::host_runtime::StandardHostRuntime::new(
                std::sync::Arc::new(uptrakit_command::LocalCommandExecutor),
                uptrakit_shared_types::HostCapabilities::default(),
            ));
        Self {
            local_runtime,
            config,
        }
    }

    #[cfg(test)]
    pub fn new_for_test(
        config: CatalogConfig,
        executor: std::sync::Arc<dyn uptrakit_command::CommandExecutor>,
    ) -> Self {
        let local_runtime: std::sync::Arc<dyn crate::host_runtime::HostRuntime> =
            std::sync::Arc::new(crate::host_runtime::StandardHostRuntime::new(
                executor,
                uptrakit_shared_types::HostCapabilities::default(),
            ));
        Self {
            local_runtime,
            config,
        }
    }

    pub fn catalog_config(&self) -> &CatalogConfig {
        &self.config
    }

    pub fn http_client(&self) -> Option<&reqwest::Client> {
        self.config.http_client.as_ref()
    }

    pub fn cancellation_token(&self) -> Option<&tokio_util::sync::CancellationToken> {
        self.config.cancellation_token.as_ref()
    }
}

#[cfg(feature = "catalog")]
impl crate::host_runtime::HostRuntime for ControllerRuntime {
    fn capabilities(&self) -> &uptrakit_shared_types::HostCapabilities {
        self.local_runtime.capabilities()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn executor(&self) -> std::sync::Arc<dyn uptrakit_command::CommandExecutor> {
        self.local_runtime.executor()
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p uptrakit-plugin-infrastructure-core --features catalog controller_runtime 2>&1 | tail -10
```

Expected: all 4 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/plugins/infrastructure/core/src/descriptor.rs
git commit -m "fix(plugin-core): ControllerRuntime embeds StandardHostRuntime with LocalCommandExecutor"
```

---

## Task 5: Migrate all plugins from `require_posix_executor` to `runtime.executor()`

This is a mechanical change across 16 plugin files. Each plugin's `new()` replaces
`require_posix_executor(runtime.as_ref())...` with `runtime.executor()`.

**Files:**

- Modify: all 16 plugin files listed below

- [ ] **Step 1: Migrate the cargo plugin (the original bug)**

In `crates/plugins/package-managers/cargo/src/plugin.rs`, replace line 10 import:

```rust
    PluginRole, Result, declare_plugin, plugin_ids, require_posix_executor,
```

with:

```rust
    PluginRole, Result, declare_plugin, plugin_ids,
```

And replace line 116:

```rust
        let executor = require_posix_executor(runtime.as_ref()).map_err(|e| format!("{e}"))?;
```

with:

```rust
        let executor = runtime.executor();
```

- [ ] **Step 2: Migrate all other plugins**

Apply the same pattern to every remaining plugin. In each file:

1. Remove `require_posix_executor` from the import statement
2. Replace `require_posix_executor(runtime.as_ref()).map_err(|e| format!("{e}"))?` with `runtime.executor()`

The full list of files and their import lines:

**`crates/plugins/generic/shell/src/plugin.rs`** — remove `require_posix_executor` from line 11 import; replace line 38.

**`crates/plugins/hooks/shell/src/plugin.rs`** — remove `require_posix_executor` from line 8 import; replace line 30.

**`crates/plugins/hooks/systemd/src/plugin.rs`** — remove `require_posix_executor` from line 9 import; replace line 30.

**`crates/plugins/package-managers/npm/src/plugin.rs`** — remove `require_posix_executor` from line 11 import; replace line 182.

**`crates/plugins/package-managers/apt/src/plugin.rs`** — remove `require_posix_executor` from line 7 import; replace line 88.

**`crates/plugins/package-managers/dnf/src/plugin.rs`** — remove `require_posix_executor` from line 13 import; replace line 91.

**`crates/plugins/package-managers/pacman/src/plugin.rs`** — remove `require_posix_executor` from line 7 import; replace line 77.

**`crates/plugins/package-managers/apk/src/plugin.rs`** — remove `require_posix_executor` from line 13 import; replace line 232.

**`crates/plugins/package-managers/pkg/src/plugin.rs`** — remove `require_posix_executor` from line 13 import; replace line 69.

**`crates/plugins/package-managers/snap/src/plugin.rs`** — remove `require_posix_executor` from line 8 import; replace line 189.

**`crates/plugins/package-managers/homebrew/src/plugin.rs`** — remove `require_posix_executor` from line 6 import; replace line 80.

**`crates/plugins/package-managers/mas/src/plugin.rs`** — remove `require_posix_executor` from line 13 import; replace line 154.

**`crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`** — remove `require_posix_executor` from line 8 import; replace line 112.

**`crates/plugins/releases/github/src/plugin.rs`** — replace line 103:

```rust
            uptrakit_plugin_infrastructure_core::require_posix_executor(runtime.as_ref()).ok();
```

with:

```rust
            Some(runtime.executor());
```

**`crates/plugins/releases/docker/src/plugin.rs`** — replace line 66:

```rust
            uptrakit_plugin_infrastructure_core::require_posix_executor(runtime.as_ref()).ok();
```

with:

```rust
            Some(runtime.executor());
```

- [ ] **Step 3: Verify compilation**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/
git commit -m "refactor(plugins): migrate all plugins from require_posix_executor to runtime.executor()"
```

---

## Task 6: Rename `PosixHostRuntime` to `StandardHostRuntime` in all plugin test code

The rename affects test modules that construct `PosixHostRuntime::new(...)` directly.

**Files:**

- Modify: all files listed below

- [ ] **Step 1: Rename in all test files**

In every file below, replace `PosixHostRuntime` with `StandardHostRuntime` in both imports and constructor
calls. Mechanical find-and-replace within each file.

**Files with `PosixHostRuntime` in test code:**

- `crates/plugins/package-managers/cargo/src/plugin.rs`
- `crates/plugins/package-managers/cargo/src/detection.rs`
- `crates/plugins/package-managers/apt/src/plugin.rs`
- `crates/plugins/package-managers/apt/src/discovery.rs`
- `crates/plugins/package-managers/apt/src/releases.rs`
- `crates/plugins/package-managers/apt/src/detection.rs`
- `crates/plugins/package-managers/pacman/src/plugin.rs`
- `crates/plugins/package-managers/pacman/src/discovery.rs`
- `crates/plugins/package-managers/pacman/src/releases.rs`
- `crates/plugins/package-managers/pacman/src/detection.rs`
- `crates/plugins/package-managers/snap/src/discovery.rs`
- `crates/plugins/package-managers/snap/src/releases.rs`
- `crates/plugins/package-managers/snap/src/detection.rs`
- `crates/plugins/package-managers/pkg/src/plugin.rs`
- `crates/plugins/package-managers/homebrew/src/plugin.rs`
- `crates/plugins/package-managers/mas/src/plugin.rs`
- `crates/plugins/releases/github/src/plugin.rs`
- `crates/plugins/releases/forgejo/src/plugin.rs`
- `crates/plugins/releases/gitlab/src/plugin.rs`
- `crates/plugins/infrastructure/proxmox/src/plugin.rs`
- `crates/plugins/hooks/shell/src/plugin.rs`
- `crates/plugins/hooks/systemd/src/plugin.rs`
- `crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`
- `crates/plugins/generic/shell/src/plugin.rs`

In each file, the pattern is the same:

1. In imports, replace `PosixHostRuntime` with `StandardHostRuntime`
2. In function bodies, replace `PosixHostRuntime::new(` with `StandardHostRuntime::new(`

- [ ] **Step 2: Verify compilation**

```bash
cargo check --all-features 2>&1 | grep "^error" | head -20
```

Expected: no errors.

- [ ] **Step 3: Run all plugin tests**

```bash
cargo test --all-features 2>&1 | tail -30
```

Expected: all tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/plugins/
git commit -m "refactor(plugins): rename PosixHostRuntime to StandardHostRuntime in test code"
```

---

## Task 7: Full quality gate

- [ ] **Step 1: Run the full quality gate**

```bash
cargo fmt --all
cargo check --no-default-features --features db-sqlite 2>&1 | grep "^error"
cargo check --all-features 2>&1 | grep "^error"
cargo clippy --all-targets --no-default-features --features db-sqlite 2>&1 | grep "^error"
cargo clippy --all-targets --all-features 2>&1 | grep "^error"
cargo test -p uptrakit-plugin-infrastructure-core --all-features 2>&1 | tail -20
```

Expected: no errors, all tests pass.

- [ ] **Step 2: Run all plugin tests**

```bash
cargo test --all-features -p uptrakit-plugin-package-manager-cargo -p uptrakit-plugin-package-manager-apt -p uptrakit-plugin-package-manager-homebrew -p uptrakit-plugin-package-manager-npm -p uptrakit-plugin-releases-github -p uptrakit-plugin-releases-docker 2>&1 | tail -20
```

Expected: all tests pass.

- [ ] **Step 3: Verify the original regression is fixed**

The original error was:

```text
plugin construction failed: configuration error: this plugin requires a POSIX host runtime
```

This came from `controller_fetch.rs` calling
`(slot.create)(&job.merged_config, controller_runtime.clone())`
for a cargo `ReleaseFetcher` slot. After the fix,
`ControllerRuntime` provides a real executor via
`runtime.executor()`, so `CargoPlugin::new()` succeeds.

There is no isolated integration test for this path. Manual
verification via the running application is the final check:
trigger a version check for a cargo-managed package and confirm
it completes without the error.

- [ ] **Step 4: Verify no stale references remain**

```bash
# Should return zero matches (only in docs/specs/plans and git history)
grep -r "require_posix_executor\|PosixHostRuntime" crates/ --include="*.rs" | grep -v "target/" | head -20
```

Expected: no matches in any `.rs` file under `crates/`.
