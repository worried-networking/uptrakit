# Code Review: uptrakit-agent-core

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-agent-core` (~1,560 LoC across 5 source files) provides the shared version-check, update
execution, and discovery primitives used by both `uptrakit-agent` (local execution) and
`uptrakit-agent-ssh` (remote SSH execution). The executor is injected by the caller via
`CommandExecutor`, keeping transport details outside this crate.

The crate demonstrates correct abstraction boundaries: plugin management delegated to
`uptrakit-plugin-infrastructure-registry`, transport to `uptrakit-service-sdk`, and execution via
`CommandExecutor` from `uptrakit-command`. The main concern is that all plugins are linked
unconditionally, including platform-specific ones.

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

**[MEDIUM]** `Cargo.toml:27` -- `uptrakit-plugin-infrastructure-registry` is an unconditional
dependency. The registry compiles all plugin crates (GitHub, Docker Registry, Homebrew, Proxmox
Helper Scripts) into every binary that links `uptrakit-agent-core`. A Linux agent binary
includes `HomebrewPlugin` even though `brew` will never be present. The plugin's `validate()`
does not check for `brew` at validation time, so the failure only surfaces at runtime. As the
plugin set grows, introduce `#[cfg(target_os = "macos")]` guards or plugin-specific Cargo
features.

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

**[MEDIUM]** `src/version_check.rs` and `src/update.rs` -- Eight async tests use bare
`#[tokio::test]`. The `update.rs` module uses `tokio::time::timeout` in production code,
creating a maintenance hazard if tests are ever time-sensitive. Per `testing.md`,
`start_paused = true` is required for all async tests.

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
