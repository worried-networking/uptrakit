# ControllerRuntime Executor Access — Design

## Problem

`CargoPlugin::new()` calls `require_posix_executor()` unconditionally. When the controller
constructs a `ReleaseFetcher` for a cargo-managed package, it passes `ControllerRuntime` —
which cannot be downcast to `PosixHostRuntime`, causing a construction failure:

```text
plugin construction failed: configuration error: this plugin requires a POSIX host runtime
```

More broadly, the current design conflates "getting an executor" with "verifying POSIX
compatibility" via `downcast_ref::<PosixHostRuntime>()`. Every host runtime has a command
executor — POSIX, Windows (future), RouterOS (future). Plugins like Cargo that work across
multiple OS families shouldn't need to downcast to a specific runtime type.

## Approach

Add `fn executor(&self) -> Arc<dyn CommandExecutor>` to the `HostRuntime` trait. Every
runtime implements it — the executor is a fundamental capability of any host runtime.

`PosixHostRuntime` is renamed to `StandardHostRuntime` — it is platform-neutral
(wraps any `CommandExecutor` + `HostCapabilities`). `ControllerRuntime` embeds one
internally with `LocalCommandExecutor` and delegates `executor()` and `capabilities()`
to it. The `new(config: CatalogConfig)` signature is unchanged.

`require_posix_executor()` is removed. Plugins use `runtime.executor()` directly.
Host compatibility is enforced at dispatch time by `HostRequirements` (via `os_families`
and `is_compatible_with()`), not at construction time via downcast. Incompatible commands
(e.g., `apt` on Windows) fail naturally at runtime with clear errors.

## Architecture

### `host_runtime.rs`

Add `executor()` to the `HostRuntime` trait:

```rust
pub trait HostRuntime: Send + Sync + 'static {
    fn capabilities(&self) -> &HostCapabilities;
    fn as_any(&self) -> &dyn std::any::Any;
    fn executor(&self) -> Arc<dyn CommandExecutor>;
}
```

Rename `PosixHostRuntime` to `StandardHostRuntime`. It implements `executor()` by
cloning its inner `Arc<dyn CommandExecutor>`.

The existing inherent `PosixHostRuntime::executor(&self) -> &Arc<dyn CommandExecutor>`
method (returns a reference) is removed. All callers use the trait method instead, which
returns `Arc<dyn CommandExecutor>` (owned/cloned). This is a minor API change — callers
that did `posix.executor().clone()` now just call `runtime.executor()`.

Remove `require_posix_executor()` and its re-export from `lib.rs`.

Update `construct_host_runtime()` doc comment to reference `StandardHostRuntime`
(the function body is unchanged — it already returns the right type, just renamed).

### `descriptor.rs`

`ControllerRuntime` (gated on `#[cfg(feature = "catalog")]`) gains a `local_runtime` field:

```rust
pub struct ControllerRuntime {
    local_runtime: Arc<dyn HostRuntime>,
    config: CatalogConfig,
}
```

`new(config)` constructs a `StandardHostRuntime` with `LocalCommandExecutor` and
`HostCapabilities::default()`. Both are cross-platform — no `#[cfg]` needed.

`executor()` and `capabilities()` delegate to `self.local_runtime`. `as_any()` returns
`self` (preserving `ControllerRuntime` identity — no need to delegate since `executor()`
is on the trait and no downcast to the inner runtime is required):

```rust
impl HostRuntime for ControllerRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        self.local_runtime.capabilities()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn executor(&self) -> Arc<dyn CommandExecutor> {
        self.local_runtime.executor()
    }
}
```

Existing inherent methods — `catalog_config()`, `http_client()`,
`cancellation_token()` — are unchanged and remain available via
`as_any().downcast_ref::<ControllerRuntime>()`.

The static `OnceLock<HostCapabilities>` in the current `capabilities()` impl is
removed — delegation to `self.local_runtime.capabilities()` replaces it.

A `#[cfg(test)]` constructor allows injecting a custom executor:

```rust
#[cfg(test)]
pub fn new_for_test(
    config: CatalogConfig,
    executor: Arc<dyn CommandExecutor>,
) -> Self
```

### `cargo/src/plugin.rs`

`CargoPlugin::new()` switches from `require_posix_executor(runtime.as_ref())?` to
`runtime.executor()`:

```rust
let executor = runtime.executor();
```

The `executor` field type stays `Arc<dyn CommandExecutor>` (no longer `Option`, no
`require_executor()` helper needed).

Note: Cargo's `HostRequirements::POSIX` in `declare_plugin!` stays unchanged for now.
While Cargo is cross-platform in capability (works on POSIX + Windows), there is no
Windows `OsFamily` variant yet. The `HostRequirements` will be broadened when Windows
support is added.

### `controller_fetch.rs`

No changes. `ControllerRuntime::new(CatalogConfig::default())` already constructs a
runtime with a real executor.

## File Map

| File | Change |
| --- | --- |
| `crates/plugins/infrastructure/core/src/host_runtime.rs` | Rename `PosixHostRuntime` to `StandardHostRuntime`; add `executor()` to trait; remove inherent `executor()` method; remove `require_posix_executor()` |
| `crates/plugins/infrastructure/core/src/lib.rs` | Rename `PosixHostRuntime` re-export to `StandardHostRuntime`; remove `require_posix_executor` re-export |
| `crates/plugins/infrastructure/core/src/descriptor.rs` | `ControllerRuntime` gains `local_runtime` field; delegates `executor()` and `capabilities()`; `as_any()` returns `self`; `#[cfg(test)]` `new_for_test` |
| `crates/plugins/infrastructure/core/src/testing.rs` | Rename `PosixHostRuntime` to `StandardHostRuntime` in `test_runtime()` and `test_runtime_with_executor()` |
| All plugin `plugin.rs` files | `runtime.executor()` instead of `require_posix_executor()`; update `PosixHostRuntime` imports to `StandardHostRuntime` in test code (see full list below) |

## Rename: `PosixHostRuntime` to `StandardHostRuntime`

The struct is platform-neutral — it wraps any `CommandExecutor` + `HostCapabilities`.
The "Posix" name is misleading since the same struct will be used for Windows hosts
and potentially RouterOS in the future. All references across the codebase update to
`StandardHostRuntime`, including:

- Production code imports
- Test helpers (`testing.rs`: `test_runtime()`, `test_runtime_with_executor()`)
- Plugin test modules that construct runtimes directly (apt, pacman, snap, homebrew,
  mas, pkg, proxmox, forgejo, gitlab, and others)
- Doc comments on `construct_host_runtime()` and the struct itself

## What Does NOT Change

- `controller_fetch.rs` — no changes
- `StandardHostRuntime` struct fields and constructor — unchanged (renamed only)
- `HostRequirements` system — unchanged (still the primary dispatch-time gate)
- `construct_host_runtime()` body — unchanged (returns renamed type, doc updated)
- `ControllerRuntime` inherent methods (`catalog_config()`, `http_client()`,
  `cancellation_token()`) — unchanged
- `ControllerRuntime` feature gate (`#[cfg(feature = "catalog")]`) — unchanged

## Callers of `require_posix_executor` to Migrate

All callers switch to `runtime.executor()`. This is a mechanical change — every call site
replaces `require_posix_executor(runtime.as_ref()).map_err(|e| format!("{e}"))?` with
`runtime.executor()`. No behavioral change; dispatch-time `HostRequirements` already
protects each plugin.

**Cross-platform plugins** (work on POSIX + Windows, `HostRequirements` to be broadened
when Windows `OsFamily` is added):

- `cargo/src/plugin.rs`
- `generic/shell/src/plugin.rs`
- `hooks/shell/src/plugin.rs`
- `package-managers/npm/src/plugin.rs`

**POSIX-only plugins** (protected by `HostRequirements::POSIX` at dispatch):

- `package-managers/apt/src/plugin.rs`
- `package-managers/dnf/src/plugin.rs`
- `package-managers/pacman/src/plugin.rs`
- `package-managers/apk/src/plugin.rs`
- `package-managers/pkg/src/plugin.rs`
- `package-managers/snap/src/plugin.rs`
- `package-managers/homebrew/src/plugin.rs`
- `package-managers/mas/src/plugin.rs`
- `hooks/systemd/src/plugin.rs`
- `discovery/proxmox-helper-scripts/src/plugin.rs`

**Optional executor plugins** (already use `.ok()`):

- `releases/github/src/plugin.rs` — simplifies to `Some(runtime.executor())`
- `releases/docker/src/plugin.rs` — simplifies to `Some(runtime.executor())`

## Testing

- `host_runtime.rs`: rename `posix_runtime_downcast` and `require_posix_executor_succeeds`
  tests to reflect new naming; test `executor()` via the trait method
- `descriptor.rs`: add test that `ControllerRuntime::new_for_test(...)` returns a working
  executor (verify `executor()` returns the injected mock); add test that production
  `new()` provides a real executor
- `testing.rs`: update `test_runtime()` and `test_runtime_with_executor()` to use
  `StandardHostRuntime`
- Plugin test modules: update `PosixHostRuntime` imports and constructions to
  `StandardHostRuntime` (mechanical rename)
- Existing plugin tests continue to pass (executor is always available)

## Non-Goals

- No `new_with_executor` public constructor on `ControllerRuntime` (YAGNI)
- No changes to `HostRequirements` constants (`POSIX`, `POSIX_PRIVILEGED` stay as-is)
- No Windows or RouterOS runtime implementation (future work)
- No broadening of Cargo/Shell/npm `HostRequirements` yet (deferred until Windows
  `OsFamily` is added)
