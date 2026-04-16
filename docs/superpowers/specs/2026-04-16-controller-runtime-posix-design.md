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
internally with `LocalCommandExecutor` and delegates `executor()`, `as_any()`,
and `capabilities()` to it. The `new(config: CatalogConfig)` signature is unchanged.

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

`StandardHostRuntime` implements it by cloning its inner `Arc<dyn CommandExecutor>`.

Remove `require_posix_executor()`. All call sites switch to `runtime.executor()`.

Update `construct_host_runtime` — no changes needed (already returns `StandardHostRuntime`
which will implement `executor()`).

### `descriptor.rs`

`ControllerRuntime` wraps an inner `Arc<dyn HostRuntime>`:

```rust
pub struct ControllerRuntime {
    local_runtime: Arc<dyn HostRuntime>,
    config: CatalogConfig,
}
```

`new(config)` constructs a `StandardHostRuntime` with `LocalCommandExecutor` and
`HostCapabilities::default()`. Both are cross-platform — no `#[cfg]` needed.

All `HostRuntime` methods delegate to `self.local_runtime`:

```rust
impl HostRuntime for ControllerRuntime {
    fn capabilities(&self) -> &HostCapabilities {
        self.local_runtime.capabilities()
    }
    fn as_any(&self) -> &dyn Any {
        self.local_runtime.as_any()
    }
    fn executor(&self) -> Arc<dyn CommandExecutor> {
        self.local_runtime.executor()
    }
}
```

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

### `controller_fetch.rs`

No changes. `ControllerRuntime::new(CatalogConfig::default())` already constructs a
runtime with a real executor.

## File Map

| File | Change |
| --- | --- |
| `crates/plugins/infrastructure/core/src/host_runtime.rs` | Rename `PosixHostRuntime` → `StandardHostRuntime`; add `executor()` to trait; remove `require_posix_executor()` |
| `crates/plugins/infrastructure/core/src/lib.rs` | Update re-exports |
| `crates/plugins/infrastructure/core/src/descriptor.rs` | `ControllerRuntime` wraps `Arc<dyn HostRuntime>`; delegates all methods; `#[cfg(test)]` `new_for_test` |
| All plugin `plugin.rs` files | `runtime.executor()` instead of `require_posix_executor()` (see full list below) |

## Rename: `PosixHostRuntime` → `StandardHostRuntime`

The struct is platform-neutral — it wraps any `CommandExecutor` + `HostCapabilities`.
The "Posix" name is misleading since the same struct will be used on Windows and
potentially RouterOS. All references across the codebase update to `StandardHostRuntime`.

## What Does NOT Change

- `controller_fetch.rs` — no changes
- `StandardHostRuntime` struct and public API — unchanged (renamed + gains `executor()` impl)
- `HostRequirements` system — unchanged (still the primary dispatch-time gate)
- `construct_host_runtime()` — unchanged (returns renamed type)

## Callers of `require_posix_executor` to Migrate

All callers switch to `runtime.executor()`. This is a mechanical change — every call site
replaces `require_posix_executor(runtime.as_ref()).map_err(|e| format!("{e}"))?` with
`runtime.executor()`. No behavioral change; dispatch-time `HostRequirements` already
protects each plugin.

**Cross-platform plugins** (work on POSIX + Windows):

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

- `host_runtime.rs`: update existing `require_posix_executor_succeeds` test to use
  `executor()` directly; add test that `executor()` returns a valid executor
- `descriptor.rs`: add test that `ControllerRuntime::new_for_test(...)` returns a
  working executor; add test that production `new()` provides an executor
- Existing plugin tests continue to pass (executor is always available)

## Non-Goals

- No `new_with_executor` public constructor on `ControllerRuntime` (YAGNI)
- No changes to `HostRequirements` constants (`POSIX`, `POSIX_PRIVILEGED` stay as-is)
- No Windows or RouterOS runtime implementation (future work)
