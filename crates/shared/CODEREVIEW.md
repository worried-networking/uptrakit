# Code Review: Shared Libraries (Umbrella)

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

**Crates covered:** `uptrakit-build-info`, `uptrakit-shared-macros`, `uptrakit-directories`,
`uptrakit-update-hooks`

These four crates are trivial (1-2 source files each) and are covered in this umbrella review.
Non-trivial shared crates have individual `CODEREVIEW.md` files in their respective directories.

## Summary

These four utility crates form the lowest layer of the workspace dependency graph. The overall
quality is high: `uptrakit-build-info` provides deterministic compile-time/runtime separation,
`uptrakit-shared-macros` exports a single well-documented macro, `uptrakit-directories` provides
async-first directory management with platform permission hardening, and `uptrakit-update-hooks`
defines the hook execution model. No Critical or High issues apply.

## Architecture

### Strengths

- `crates/shared/build-info/src/lib.rs` -- `emit_enabled_features_env()` runs in `build.rs`,
  `BuildInfo::current()` runs at runtime. The two phases are cleanly separated via
  `cargo:rustc-env=UPTRAKIT_BUILD_ENABLED_FEATURES`. No mutable global state.
- `crates/shared/macros/src/lib.rs` -- Exports exactly one macro (`impl_report_conversion!`).
  Three arms (single variant-mapping, single closure-based, multi-variant) are documented with
  expansion examples. Zero dead code.
- `crates/shared/directories/src/lib.rs` -- All public I/O functions are `async` (backed by
  `tokio::fs`). Platform permission hardening (0700 dirs, 0600 files) is conditioned on
  `#[cfg(unix)]` with documentation noting non-Unix paths are not security-hardened.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `crates/shared/directories/src/lib.rs` -- `write_secure_file_str` enforces 0600 permissions on
  Unix. Async I/O eliminates TOCTOU risks between file creation and permission setting.
- `crates/shared/macros/src/lib.rs` -- The `impl_report_conversion!` closure arm enables secure
  error wrapping (e.g., `|e| ControllerError::WebSocket(Box::new(e))`) without losing error
  chain context.
- Zero `unsafe` in production code across all four crates.

### Issues

**[LOW]** `crates/shared/directories/src/lib.rs:829,837` -- `unsafe` env var mutation in tests
(`std::env::remove_var("HOME")` / `std::env::set_var("HOME", val)`) creates data-race risk.
Rust's test harness runs tests on multiple threads. Mutating environment variables without
synchronization is undefined behavior. Fix: use `#[serial_test::serial]` or acquire a shared
mutex before calling `remove_var`/`set_var`.

## Code Quality

### Strengths

- `crates/shared/build-info/src/lib.rs` -- `normalize_feature_list` sorts and deduplicates the
  parsed feature list, guaranteeing stable `render_human()` and `Serialize` output.
  `render_human_uses_stable_keys_and_order` test validates the exact string output.
- `crates/shared/macros/src/lib.rs` -- Single responsibility. The one known constraint (target
  identifier must be a single segment, not a path) is explicitly documented with a rationale.
- `crates/shared/directories/src/lib.rs` -- Comprehensive test suite covering `expand_tilde`,
  path validation, and directory creation scenarios.

### Issues

**[LOW]** `crates/shared/directories/src/lib.rs` -- Several `#[tokio::test]` tests perform only
synchronous filesystem operations (e.g., `expand_tilde_user_syntax_unchanged`,
`validate_path_name_*`). Contains no `await` points and does not require an async runtime.
Convert to `#[test]` where no `await` is present.

## High Availability

### Strengths

- `crates/shared/directories/src/lib.rs` -- Async-first design means directory operations do not
  block the runtime. `tokio::fs` operations yield cooperative control.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- All four crates use `edition = "2024"` and workspace-pinned dependencies.
- `crates/shared/macros/src/lib.rs` -- `impl_report_conversion!` enforces the `rootcause` error
  convention, generating `ReportConversion` implementations that use `context_transform` rather
  than constructing fresh reports.
- Zero `#[allow(clippy::...)]` suppressions across all four crates.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `crates/shared/build-info/src/lib.rs` -- Adding a new field to `BuildInfo` is a single-struct
  change. The `render_human` exact-match string test acts as a change-detection mechanism.
- `crates/shared/macros/src/lib.rs` -- The closure arm enables non-trivial error wrapping
  without writing a full `impl ReportConversion` block, covering cases where the source error
  must be heap-allocated.

### Issues

No extensibility issues found.
