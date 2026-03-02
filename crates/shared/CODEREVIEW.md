# Code Review: Shared Libraries (Umbrella)

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture | security | quality | HA | standards |
  extensibility | tests | consistency | maintainability | database | crate-structure)
- **Branch**: docs/codereview-backend

**Crates covered:** `uptrakit-backoff`, `uptrakit-build-info`, `uptrakit-shared-macros`,
`uptrakit-directories`, `uptrakit-update-hooks`

These five crates are small utility crates (105-768 lines each) and are covered in this umbrella
review. Non-trivial shared crates have individual `CODEREVIEW.md` files in their respective
directories.

## Summary

These five utility crates form the lowest layer of the workspace dependency graph. The overall
quality is high: `uptrakit-build-info` provides deterministic compile-time/runtime separation,
`uptrakit-shared-macros` exports a single well-documented macro, `uptrakit-directories` provides
async-first directory management with platform permission hardening, `uptrakit-update-hooks`
defines the hook execution model with both legacy and structured formats, and `uptrakit-backoff`
provides a clean synchronous exponential backoff with jitter. No Critical or High issues apply.

The `uptrakit-update-hooks` crate has grown to 768 lines with comprehensive test coverage for
predefined hook resolution (systemd, docker-compose) and legacy merge behavior. The
`uptrakit-backoff` crate is a well-tested 105-line single-file crate that is a candidate for
merging into `service-sdk` or `agent-core` if the crate count becomes a maintenance concern.

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
- `crates/shared/backoff/src/lib.rs` -- Pure synchronous implementation with no async
  dependency. Jitter calculation uses `rand::rng().random_range(0..=quarter_ms)` to prevent
  thundering herd. The `next_delay` / `reset` API is minimal and correct.
- `crates/shared/update-hooks/src/lib.rs` -- Clean two-phase resolution: structured hooks
  (`HooksConfig` with predefined templates) take precedence, falling back to legacy
  `pre_update_commands` / `post_update_commands` arrays. Override completely replaces base
  when present, avoiding merge ambiguity.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `crates/shared/directories/src/lib.rs` -- `write_secure_file_str` enforces 0600 permissions on
  Unix. Async I/O eliminates TOCTOU risks between file creation and permission setting.
- `crates/shared/macros/src/lib.rs` -- The `impl_report_conversion!` closure arm enables secure
  error wrapping (e.g., `|e| ControllerError::WebSocket(Box::new(e))`) without losing error
  chain context.
- `crates/shared/update-hooks/src/lib.rs:143-156` -- Unknown `PredefinedHook` variants are
  handled with `tracing::warn!` and a no-op `HookCommand::Exec { program: "true", .. }`,
  following the `#[non_exhaustive]` wildcard convention.
- Zero `unsafe` in production code across all five crates.

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
- `crates/shared/backoff/src/lib.rs` -- Four tests cover doubling behavior, max cap, reset,
  and zero-base edge case. Jitter bounds are asserted with range checks rather than exact
  equality, correctly accounting for randomness.
- `crates/shared/update-hooks/src/lib.rs` -- 23 tests covering predefined hook resolution
  (systemd stop/start/restart/reload, docker-compose down/up/pull/restart with and without
  compose file and project dir), structured hooks with custom commands, override-replaces-base,
  legacy merge, partial override, clear-with-empty-array, and config merge.

### Issues

**[LOW]** `crates/shared/directories/src/lib.rs` -- Several `#[tokio::test]` tests perform only
synchronous filesystem operations (e.g., `expand_tilde_user_syntax_unchanged`,
`validate_path_name_*`). Contains no `await` points and does not require an async runtime.
Convert to `#[test]` where no `await` is present.

## High Availability

### Strengths

- `crates/shared/directories/src/lib.rs` -- Async-first design means directory operations do not
  block the runtime. `tokio::fs` operations yield cooperative control.
- `crates/shared/backoff/src/lib.rs` -- Jitter prevents thundering herd on reconnection,
  supporting HA reconnect patterns in consumers like `agent-core` and `service-sdk`.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- All five crates use `edition = "2024"` and workspace-pinned dependencies.
- `crates/shared/macros/src/lib.rs` -- `impl_report_conversion!` enforces the `rootcause` error
  convention, generating `ReportConversion` implementations that use `context_transform` rather
  than constructing fresh reports.
- Zero `#[allow(clippy::...)]` suppressions across all five crates.

### Issues

**[LOW]** `crates/shared/backoff/Cargo.toml` -- No `publish = false` declaration. The crate is
an internal utility and should not be accidentally published to crates.io.

## Extensibility

### Strengths

- `crates/shared/build-info/src/lib.rs` -- Adding a new field to `BuildInfo` is a single-struct
  change. The `render_human` exact-match string test acts as a change-detection mechanism.
- `crates/shared/macros/src/lib.rs` -- The closure arm enables non-trivial error wrapping
  without writing a full `impl ReportConversion` block, covering cases where the source error
  must be heap-allocated.
- `crates/shared/update-hooks/src/lib.rs:143-156` -- The `PredefinedHook` match has a wildcard
  arm with `tracing::warn!` and no-op fallback. Adding a new `PredefinedHook` variant to
  `web-api-types` does not require a simultaneous update to `update-hooks`; the new variant
  will be logged and gracefully skipped until explicit support is added.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `crates/shared/macros/tests/report_conversion.rs` -- Integration test (separate binary)
  exercises all three arms of `impl_report_conversion!`: single variant-mapping, single
  closure-based, and multi-variant. Verifies that the generated `ReportConversion` impls
  produce the expected error types via `context_transform`.
- `crates/shared/build-info/src/lib.rs` -- `render_human_uses_stable_keys_and_order` is a
  snapshot test asserting the exact human-readable string for `BuildInfo::current()`.
  Acts as a change-detection guard for the output format.
- `crates/shared/directories/src/lib.rs` -- Test suite covers `expand_tilde` (user syntax
  unchanged, home dir expansion), `validate_path_name` (valid, invalid characters, empty
  string), and directory creation.
- `crates/shared/backoff/src/lib.rs:51-105` -- Four tests: `doubling_behaviour` verifies
  exponential progression with jitter bounds, `max_cap` confirms the delay does not exceed
  the configured maximum, `reset_returns_to_base` validates state reset after successful
  connection, `zero_base_does_not_panic` covers the edge case of `Duration::ZERO` as base.
- `crates/shared/update-hooks/src/lib.rs:266-768` -- 23 tests covering all hook resolution
  paths. Predefined hooks are tested for all four systemd actions and four docker-compose
  actions. The structured-vs-legacy fallback path is tested. Override semantics (full replace,
  partial override, clear-with-empty-array) are tested for both legacy and structured formats.
  `merge_config` is tested for basic merge, no-override, and empty-override cases.

### Issues

**[LOW]** `crates/shared/directories/src/lib.rs:829,837` -- Tests that call
`std::env::remove_var("HOME")` / `std::env::set_var("HOME", val)` mutate the process
environment without synchronization. The default Rust test harness runs tests across multiple
threads, making concurrent `env` mutation a data race. Fix: use `#[serial_test::serial]` or
acquire a shared mutex before calling `remove_var`/`set_var`.

**[LOW]** `crates/shared/directories/src/lib.rs` -- Several `#[tokio::test]` tests perform
only synchronous filesystem or string operations with no `await` points (e.g.,
`expand_tilde_user_syntax_unchanged`, `validate_path_name_*`). These should use `#[test]`
instead of `#[tokio::test]` -- spinning up a Tokio runtime for synchronous tests adds
overhead and misleads readers.

## Consistency

### Strengths

- All five crates follow the same `Cargo.toml` field inheritance pattern (workspace edition,
  license, authors, repository, version).
- Error handling is consistent: `rootcause` / `report!` / `bail!` used in `update-hooks` and
  `directories`, while `backoff` and `build-info` are infallible by design.
- `update-hooks` follows the established `#[non_exhaustive]` wildcard convention with
  `tracing::warn!` on unknown variants, matching the pattern documented in `coding-standards.md`.

### Issues

No consistency issues found.

## Maintainability

### Strengths

- Each crate has a focused responsibility and minimal public API surface.
- `crates/shared/update-hooks/src/lib.rs` -- Despite growing to 768 lines, the file is
  well-organized with clear section headers: predefined hook resolution, structured hooks
  resolution, legacy merge, config merge, and a comprehensive test section.

### Issues

**[INFO]** `crates/shared/backoff/` -- At 105 lines with a single `rand` dependency, this
crate is a candidate for merging into `service-sdk` or `agent-core` if reducing the workspace
crate count becomes a priority. However, keeping it separate is reasonable if other crates
need backoff without pulling in the larger SDK.

**[INFO]** `crates/shared/macros/` -- At 142 lines, this crate must remain separate due to
Rust's proc-macro compilation model. This is not a refactoring opportunity.

**[INFO]** `crates/shared/build-info/` -- At 216 lines with a distinct `build.rs`
compilation concern, this crate is reasonable to keep separate. The compile-time/runtime split
requires an independent compilation unit.
