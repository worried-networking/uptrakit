# ControllerRuntime POSIX Extension — Design

## Problem

`CargoPlugin::new()` calls `require_posix_executor()` unconditionally. When the controller
constructs a `ReleaseFetcher` for a cargo-managed package, it passes `ControllerRuntime` —
which cannot be downcast to `PosixHostRuntime`, causing a construction failure:

```text
plugin construction failed: configuration error: this plugin requires a POSIX host runtime
```

The controller always runs on a POSIX OS. There is no goal to support non-POSIX controllers.
`ControllerRuntime` should therefore be able to satisfy POSIX executor requirements.

## Approach

Add `fn as_posix(&self) -> Option<&PosixHostRuntime>` to the `HostRuntime` trait (default:
`None`). Both `PosixHostRuntime` and `ControllerRuntime` opt in by returning `Some`. Update
`require_posix_executor` to use this method instead of `downcast_ref::<PosixHostRuntime>()`.

`ControllerRuntime` embeds a `PosixHostRuntime` constructed with `LocalCommandExecutor`
internally in `new()` — no change to the public `new(config: CatalogConfig)` signature.
A `#[cfg(test)]`-only constructor `new_for_test(config, executor)` allows test code to
inject a `NoopCommandExecutor`.

## Architecture

### `host_runtime.rs`

Add `as_posix()` to the `HostRuntime` trait with a default `None` impl:

```rust
fn as_posix(&self) -> Option<&PosixHostRuntime> { None }
```

`PosixHostRuntime` implements it returning `Some(self)`.

Update `require_posix_executor` to call `runtime.as_posix()`:

```rust
pub fn require_posix_executor(runtime: &dyn HostRuntime) -> Result<Arc<dyn CommandExecutor>> {
    runtime
        .as_posix()
        .map(|r| Arc::clone(r.executor()))
        .ok_or_else(|| report!(PluginError::Configuration(
            "this plugin requires a POSIX host runtime".to_string()
        )))
}
```

### `descriptor.rs`

`ControllerRuntime` gains a `posix: PosixHostRuntime` field:

```rust
pub struct ControllerRuntime {
    posix: PosixHostRuntime,
    config: CatalogConfig,
}
```

`new(config)` constructs `posix` with `Arc::new(LocalCommandExecutor)` and
`HostCapabilities::default()` internally. Signature unchanged.

`as_posix()` returns `Some(&self.posix)`. `as_any()` still returns `self` (as `ControllerRuntime`).

A `#[cfg(test)]` constructor allows injecting a custom executor in tests:

```rust
#[cfg(test)]
pub fn new_for_test(config: CatalogConfig, executor: Arc<dyn CommandExecutor>) -> Self
```

### `controller_fetch.rs`

No changes. `ControllerRuntime::new(CatalogConfig::default())` already constructs a POSIX-capable runtime.

## File Map

| File | Change |
| --- | --- |
| `crates/plugins/infrastructure/core/src/host_runtime.rs` | Add `as_posix()` to trait + `PosixHostRuntime` impl; update `require_posix_executor` |
| `crates/plugins/infrastructure/core/src/descriptor.rs` | Embed `PosixHostRuntime` in `ControllerRuntime`; update `new()`; add `as_posix()` impl; add `#[cfg(test)]` `new_for_test` |

## What Does NOT Change

- `controller_fetch.rs` — no changes
- All plugin files — no changes (zero cargo/github/docker/etc. plugin modifications)
- `require_posix_executor` error message — same string, different internal path
- `PosixHostRuntime` public API — unchanged

## Testing

- `host_runtime.rs`: add tests for `as_posix()` on `PosixHostRuntime` and a non-POSIX stub
- `descriptor.rs`: add test that `ControllerRuntime::new_for_test(...)` satisfies `require_posix_executor`;
  add test that `as_posix()` returns `Some` on production `new()`
- Existing `require_posix_executor_succeeds` test continues to pass

## Non-Goals

- No `new_with_executor` public constructor (YAGNI — no current use case beyond tests)
- No change to `ControllerRuntime` capabilities (`HostCapabilities::default()` remains)
- No support for non-POSIX controller runtimes
