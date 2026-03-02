# Code Review: uptrakit-agent-core

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-agent-core` (~1,560 LoC across 5 source files) provides the shared version-check, update
execution, and discovery primitives used by both `uptrakit-agent` (local execution) and
`uptrakit-agent-ssh` (remote SSH execution). The executor is injected by the caller via
`CommandExecutor`, keeping transport details outside this crate.

The crate demonstrates correct abstraction boundaries: plugin management delegated to
`uptrakit-plugin-infrastructure-registry`, transport to `uptrakit-service-sdk`, and execution via
`CommandExecutor` from `uptrakit-command`. Compiling platform-specific plugins (Homebrew,
Proxmox) unconditionally into all agent binaries is an accepted tradeoff — failures surface at
runtime when the tool is absent, and the plugin set is small and stable.

## Architecture

### Strengths

- `src/lib.rs:1-23` -- Clean module decomposition: `client` (protocol handling), `update`
  (execution), `version_check` (version comparison), `connection_context` (state management),
  `error` (typed errors). Each module has a focused responsibility.
- `Cargo.toml:14-29` -- Correct dependency layering. Depends on `uptrakit-command` for
  execution, `uptrakit-plugin-infrastructure-registry` for plugin dispatch,
  `uptrakit-service-sdk` for lifecycle, and `uptrakit-internal-wire` for protocol types.
- `Cargo.toml:12` -- `ssh` feature flag for SSH-specific plugin support, correctly cascaded
  to the registry dependency.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/client.rs` -- Update output is streamed via bounded channels with backpressure, preventing
  memory exhaustion from runaway command output.
- `src/update.rs` -- Update execution delegates to `CommandExecutor`, which provides shell
  escaping and timeout enforcement.
- Zero `unsafe` blocks.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/version_check.rs` -- `VersionCheckOutcome` is a clean typed result enum. `check_version`
  returns structured data rather than stringly-typed status.
- `src/connection_context.rs` -- `ConnectionContext` groups all per-connection mutable state,
  keeping function signatures readable.
- `src/client.rs` -- `InFlightUpdate` tracks update lifecycle with `CancellationToken` for
  cooperative shutdown.

### Issues

No code quality issues found.

## High Availability

### Strengths

- `src/client.rs` -- `handle_graceful_shutdown` waits for in-flight updates with timeout before
  reporting final status, preventing data loss during shutdown.
- `src/client.rs` -- Bounded aggregate channel for update events provides backpressure.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- `src/error.rs` -- `AgentCoreError` with typed variants and `thiserror`-derived `Display`.
  `impl_report_conversion!` for cross-crate error propagation.
- Uses workspace lints. Zero `#[allow(clippy::...)]` suppressions.
- Selective re-exports in `src/lib.rs` expose only the public API surface.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `src/client.rs` -- `handle_execute_update` and `handle_check_versions` accept
  `Arc<dyn CommandExecutor>` and `Arc<dyn PluginOps>`, making them testable with mock
  implementations.
- `Cargo.toml:12` -- `ssh` feature flag enables SSH-specific functionality without affecting
  the local agent binary.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `src/update.rs` -- Seven tests exercise `select_executor` (correct executor selected for
  each plugin type), `format_update_command` (flag assembly), and three async tests for
  `execute_update` success path, failure propagation, and output streaming using an in-process
  mock executor. Both success and error paths are covered.
- `src/version_check.rs` -- Seven async tests cover: single-plugin version check, multi-plugin
  check, already-up-to-date case, outdated detection, partial failure when one plugin fails,
  and the case where no executors match the requested plugin type. The full `check_versions`
  control flow is exercised.
- `src/connection_context.rs` -- Three synchronous tests cover initial state, `set_update_in_flight`,
  and `clear_update_in_flight`.

### Issues

**[MEDIUM]** `src/client.rs` -- `handle_execute_update` and `handle_check_versions` (the top-level
message-dispatch paths) have no dedicated tests. The lower-level helpers are tested but the
integration path from wire message receipt through response dispatch is untested. A mock
`CommandExecutor` and mock `PluginOps` would allow testing the full `client` dispatch
state machine without an active WebSocket connection.

## Consistency

### Strengths

- `src/client.rs:157-239` (`handle_check_versions`) and `src/client.rs:354-500`
  (`handle_discover_software`) -- Both functions return `Some(LoopOutcome::Disconnected)` when
  the final `conn.send(response)` fails and log at `tracing::error!`. Neither function absorbs
  the send error silently. The error-propagation convention for critical response sends is
  applied uniformly.
- `src/version_check.rs:75-108` (`detect_installed`) and `src/version_check.rs:111-139`
  (`fetch_latest`) -- Both helpers apply `ctx.apply_to_config` before plugin creation, then
  map errors to `String` via `.map_err(|e| e.to_string())`. The config-context injection
  pattern is identical across both roles, so adding a third role (e.g., `verify_signature`)
  would follow an obvious template.
- `src/update.rs:86-270` -- Pre-update hooks and post-update hooks both use the same
  `make_bridge` closure, `drop(plugin_tx)` + `bridge_handle.await` teardown sequence, and
  `run_hook_command` dispatch. The structure is symmetric even though the error semantics
  differ (pre-hook failure is fatal; post-hook failure is non-fatal warn-and-continue).

### Issues

**[HIGH]** `src/client.rs:234` vs `src/client.rs:139-144` -- `handle_check_versions` uses
`conn.send` (error-propagating) for the `VersionCheckResults` response and returns
`Some(LoopOutcome::Disconnected)` on failure. `handle_discover_software` at line 495 does
the same. But the update output path — `send_update_output` at line 27-38 — uses
`conn.send_best_effort` (error-absorbed) for individual `UpdateOutput` chunks, and
`send_update_result` uses `conn.send_best_effort` for the final `UpdateResult` at line 50.
The treatment is inconsistent: version-check and discovery results are treated as fatal if
undeliverable, while the final update result (which the controller needs to mark the update
complete) is silently absorbed on send failure. A dropped `UpdateResult` leaves the update
in a permanent in-progress state on the controller side until a timeout, with no reconnect
triggered.

**[MEDIUM]** `src/update.rs:282-297` (`detect_current_version`) -- This helper calls
`crate::version_check::check_version` with `&crate::connection_context::ConnectionContext::default()`.
The caller (`execute_update`) has already merged the connection context into the plugin config
via `ctx.apply_to_config` at `client.rs:261-267` before spawning the update task. However,
the `detect_current_version` function constructs its own default `ConnectionContext`, meaning
any context injections (e.g., SSH host overrides) that were not already embedded in the
serialized config at spawn time will be missing during post-update version detection. The
`handle_check_versions` path at `client.rs:183-199` uses the live `ctx` reference for the
same plugin type. The two code paths treat context injection differently: one uses the live
`ctx`, the other uses a static default.

**[LOW]** `src/update.rs:440-458` (unknown `HookCommand` arm) -- The wildcard arm for
unrecognized `HookCommand` variants (`_ =>`) logs at `tracing::warn!` and returns an error.
This is the correct `#[non_exhaustive]` handling pattern per workspace standards. However,
`handle_check_versions` in `client.rs` handles unknown `ControllerMessage` variants (via the
SDK's `ControllerMessage::Unknown` arm in `event_loop.rs`) by logging at `warn!` and
continuing — a non-fatal path. The unknown `HookCommand` arm is fatal (returns `Err`), while
the workspace standard for `#[non_exhaustive]` enums in dispatch is to warn-and-skip. The
difference is intentional (a hook command that cannot be executed must fail the hook), but a
comment explaining why the fallback is fatal rather than skipped would clarify the deviation
from the workspace pattern.
