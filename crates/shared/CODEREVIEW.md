# Code Review: `crates/shared` Umbrella

- Review date: 2026-03-17
- Scope: current-state review for shared crates without their own dedicated `CODEREVIEW.md`

## Covered Crates

- `uptrakit-audit-log`
- `uptrakit-backoff`
- `uptrakit-build-info`
- `uptrakit-directories`
- `uptrakit-extension-framework`
- `uptrakit-shared-macros`
- `uptrakit-config-merge` (update-hooks)

## Summary

The shared utility layer remains strong overall. Most of these crates are small, stable, and easy
to reason about. The existing maintainability concern in `extension-framework` (1970-line single
file) is confirmed. All other crates are clean across coding standards, security, and correctness
dimensions.

## Strengths

- `backoff` is small, focused, and correctly implements exponential doubling with jitter capped
  at 25% of the delay. Zero base duration does not panic. Tests verify doubling, max cap, reset,
  and zero-base edge cases.
- `build-info` and `config-merge` stay small and focused.
- `directories` provides good platform-aware permission handling (0o700 dirs, 0o600 files) with
  atomic write-to-temp-then-rename for file creation. Path traversal validation (`validate_path_name`)
  rejects `..`, `.`, path separators, absolute paths, and empty names. Tilde expansion is
  component-based (no lossy string conversion). Intermediate directory permissions are hardened
  after recursive creation.
- `audit-log` uses an unbounded channel to guarantee no audit entries are dropped due to
  backpressure, with clear documentation of the memory trade-off. `MultiplexBackend` fans out
  concurrently and isolates backend failures. `AuditFilter` supports per-tenant override of the
  global filter mode.
- `shared-macros` provides `impl_report_conversion!` and `wire_safe_enum!` with correct macro
  hygiene (fully qualified paths for `rootcause`, `serde`, `thiserror`, `tracing`).
  `wire_safe_enum!` auto-appends `#[non_exhaustive]` and `Other(String)`, generates a strict
  `FromStr` alongside infallible serde, and produces a named error type for parse failures.
- `config-merge` implements shallow three-layer merge correctly with non-object layers silently
  ignored.
- All required `#[non_exhaustive]`, `Other(String)` catch-all, and `parking_lot` patterns are
  present and correct across shared types, wire, and web-api-types.

## Active Findings

### [MEDIUM] `uptrakit-extension-framework` is a monolithic single-file schema crate

- **Dimension**: maintainability, crate structure
- **Scope**: `crates/shared/extension-framework/src/lib.rs` (1970 lines)
- **Description**: Two distinct domains live in one file: UI definitions (manifests, forms,
  fields, actions, placements) and wire payloads (register/request/response messages). Changes
  to either domain require reasoning about the full 1970-line file.
- **Why it matters**: adding a new extension-form feature risks unintentional serialization
  regressions in the wire domain. Code navigation is slower than necessary.
- **Failure scenario**: a developer modifying the `ExtensionManifest` struct accidentally
  changes a serde attribute on `ExtensionRequestPayload` 1500 lines away in the same file.

### [LOW] `PluginType::From<PluginType> for String` reimplements the `as_str()` match table

- **Dimension**: idiomatic Rust, maintainability
- **Scope**: `crates/shared/types/src/plugin_types.rs:230-258`
- **Description**: The `From<PluginType> for String` match arm duplicates the string values
  already present in `as_str()` (lines 63-87), creating two sources of truth for the same
  mapping. A future rename of a plugin type string requires updating both locations.
- **Why it matters**: with 20+ variants, divergence between `as_str()` and
  `From<PluginType> for String` is easy to introduce and hard to detect without explicit tests
  for every variant.
- **Failure scenario**: a new variant is added to `as_str()` but the corresponding
  `From<PluginType> for String` arm is forgotten, causing DB writes to use the wrong string.

## Removed Findings

- **[MEDIUM] `agent-core` clones large update payloads unnecessarily in the dispatch hot path**:
  moved to the dedicated `crates/shared/agent-core/CODEREVIEW.md`.
- **[MEDIUM] `HashSet` is cloned in full before the early-emptiness check in WS event handlers**:
  this finding applies to `crates/ui/web-api/`, not to `crates/shared/`. It should be tracked
  in the web-api code review, not in the shared umbrella.
- **[LOW] `uptrakit-directories` is drifting toward the same monolithic shape**: removed. The
  crate is 984 lines (490 of which are tests) and is well-organized with clear functional
  separation. It does not currently exhibit monolithic symptoms.
