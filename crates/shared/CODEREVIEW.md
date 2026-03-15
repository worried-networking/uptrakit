# Code Review: Shared Libraries (Umbrella)

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture | security | quality | HA | standards |
  extensibility | tests | consistency | maintainability | database | crate-structure)
- **Branch**: docs/codereview-backend

**Crates covered:** `uptrakit-backoff`, `uptrakit-build-info`, `uptrakit-shared-macros`,
`uptrakit-directories`, `uptrakit-update-hooks`, `uptrakit-command`

These five crates are small utility crates (105-768 lines each) and are covered in this umbrella
review. Non-trivial shared crates have individual `CODEREVIEW.md` files in their respective
directories.

## Summary

These utility crates form the lowest layer of the workspace dependency graph. The overall
quality is high: `uptrakit-build-info` provides deterministic compile-time/runtime separation,
`uptrakit-shared-macros` exports a single well-documented macro, `uptrakit-directories` provides
async-first directory management with platform permission hardening, `uptrakit-update-hooks`
defines the hook execution model with both legacy and structured formats, `uptrakit-backoff`
provides a clean synchronous exponential backoff with jitter, and `uptrakit-command` provides
shell-escaped command execution with fail-early settings and resource limits. No Critical or
High issues apply.

The `uptrakit-update-hooks` crate has grown to 768 lines with comprehensive test coverage for
predefined hook resolution (systemd, docker-compose) and legacy merge behavior. The
`uptrakit-backoff` crate is a well-tested 105-line single-file crate that is a candidate for
merging into `service-sdk` or `agent-core` if the crate count becomes a maintenance concern.

**Note on sibling shared crates (2026-03-06 parallel review):** `uptrakit-shared-types` is a
grab-bag containing many unrelated types (`PluginType`, `ServiceStatus`, `MqttTransport`,
`SecretString`, `ReleaseInfo`, `DiscoveryTarget`, etc.). Almost every crate in the workspace
depends on it, meaning any change triggers widespread recompilation. Types used only in the
plugin subsystem (e.g., `DiscoveryTarget`, `DiscoveredSoftware`) could live in
`plugin-infrastructure-core`. Similarly, `uptrakit-shared-db` contains 55 entity modules --
the largest compilation unit in terms of generated code (SeaORM entities produce substantial
derive macro output). Splitting is non-trivial due to cross-entity foreign key relationships.

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
- `crates/shared/command/src/command.rs` -- `shell_escape()` wraps values in single quotes with
  embedded single-quote escaping (`'\''`). Fail-early shell settings (`set -euo pipefail` for
  bash, `set -eu` for sh, `$ErrorActionPreference = 'Stop'` for PowerShell) prevent partial
  execution. `kill_on_drop(true)` prevents orphaned processes. `stdin(Stdio::null())` prevents
  interactive prompts. 10 MB output limit prevents OOM from runaway commands.

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
- `crates/shared/command/src/command.rs:334` -- Injection prevention test verifies
  `"2.0.0'; echo 'MARKER"` passes through safely via `shell_escape()`.
- `crates/shared/command/src/sudo.rs` -- `SudoContext` determines sudo usage based on
  runtime state (root status, sudo availability, policy). Shell-mode commands are explicitly
  excluded from sudo transformation with a warning. Environment variables forwarded as inline
  `NAME=VALUE` assignments (not via `/usr/bin/env`).
- Zero `unsafe` in production code across all covered crates.

### Issues

**[LOW]** `crates/shared/directories/src/lib.rs:829,837` -- `unsafe` env var mutation in tests
(`std::env::remove_var("HOME")` / `std::env::set_var("HOME", val)`) creates data-race risk.
Rust's test harness runs tests on multiple threads. Mutating environment variables without
synchronization is undefined behavior. Fix: use `#[serial_test::serial]` or acquire a shared
mutex before calling `remove_var`/`set_var`. *Note (2026-03-06 parallel review -- security and
coding standards):* `set_var`/`remove_var` are `unsafe` in Rust 2024 edition due to thread-safety
concerns. The `unsafe` usage is acceptable in test context but reinforces the need for
serialization.

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

---

*Parallel review notes (2026-03-06): Findings from architecture, security, code quality,
coding standards, extensibility, consistency, maintainability, HA, tests, and database reviews
have been merged into this file where specific to umbrella crates. The `command` crate coverage
was added (shell escape, fail-early settings, kill_on_drop, output limit, sudo context). The
`shared-types` recompilation concern and `shared-db` entity module count are noted in the
Summary section.*

---

## Review — 2026-03-10

### Summary

This review adds findings for `uptrakit-command` and `uptrakit-extension-framework` on
2026-03-10. The crates covered here also include `uptrakit-backoff`, `uptrakit-build-info`,
`uptrakit-shared-macros`, `uptrakit-directories`, and `uptrakit-update-hooks` (no new findings
for those five). Prior open issues are confirmed where still unresolved.

### `uptrakit-command` — Idiomatic Rust

**[MEDIUM]** `src/executor.rs` — `run_command_exec_impl` takes `&[(String, String)]` for
environment variables, while the `CommandSpec` builder method `with_env` accepts `impl Into<String>`
for individual key and value parameters. The API surface is inconsistent: callers building a
`CommandSpec` use ergonomic generic bounds, but the underlying executor function takes a
concrete slice of owned `String` pairs, forcing callers who have `&str` key-value pairs to
allocate. Recommendation: align the internal slice type with the builder API or document the
reason for the divergence.

### `uptrakit-extension-framework` — Extensibility

**[INFO]** All public framework types (`ExtensionManifest`, `ActionDef`, `FieldDef`, `FormDef`,
`WizardStep`, etc.) carry `#[non_exhaustive]` with `new()` constructors and `with_*()` builder
methods. This is the correct pattern for an evolving schema where external crates must not
depend on struct literal initialization. Confirmed correct.

### `uptrakit-scheduler-engine` — Maintainability (cross-reference)

**[MEDIUM]** `uptrakit-scheduler-engine/Cargo.toml:12` — `oidc = []` empty feature declaration
with no documentation. Full finding recorded in `crates/shared/scheduler-engine/CODEREVIEW.md`.
Noted here to flag that the same documentation gap may apply to other small feature flags in
umbrella crates.

### Strengths (2026-03-10)

- `uptrakit-command`: `CommandSpec` builder pattern with `#[must_use]` on all builder methods.
  Confirmed correct.
- `uptrakit-command`: `NoopCommandExecutor` in `crates/shared/command/src/executor.rs` is the
  canonical no-op implementation for the controller side. Do not duplicate inline. Confirmed.
- `uptrakit-extension-framework`: `#[non_exhaustive]` with `new()` constructors applied
  consistently to all public framework types. Confirmed correct.

---

## 2026-03-10 Review Update

Comprehensive 12-dimension review covering architecture, security, code quality, tests, HA,
database, coding standards, extensibility, consistency, idiomatic Rust, references & heap,
and maintainability.

### Dimension: Architecture (D1)

#### Strengths

- `crates/shared/backoff/src/lib.rs` -- Synchronous backoff crate with no async dependency is the
  correct design choice. Consumers in async contexts wrap the delay in `tokio::time::sleep`; consumers
  in synchronous contexts (e.g., blocking retry loops) use it directly without pulling in a runtime.

### Dimension: Coding Standards (D7)

#### Issues

**[LOW]** `crates/shared/directories/src/lib.rs` -- `cfg(not(target_os))` guards for platform-specific
code are acceptable for platform-divergent filesystem operations. No action required; noted for
completeness.

### Dimension: Extensibility (D8)

#### Strengths

- `crates/shared/extension-framework/src/lib.rs` -- Comprehensive `#[non_exhaustive]` coverage on all
  public framework types (`ExtensionManifest`, `ActionDef`, `FieldDef`, `FormDef`, `WizardStep`, etc.)
  with `new()` constructors and `with_*()` builder methods. External crates cannot construct these types
  via struct literals, ensuring additive evolution without breaking downstream code.

### Dimension: Idiomatic Rust (D10)

#### Strengths

- `crates/shared/backoff/src/lib.rs` -- Simple, no-dependency design with a minimal `next_delay` /
  `reset` API. The `rand` crate is the sole dependency, used only for jitter. No async runtime, no
  trait objects, no generics beyond what is necessary.

### Dimension: Maintainability (D12)

#### Issues

**[LOW]** `crates/shared/extension-framework/src/lib.rs` -- Single file at 1,562 lines containing all
framework types, builders, validation, and tests. While the file is well-organized with clear section
headers, splitting into `types.rs`, `builders.rs`, `validation.rs`, and `tests.rs` would improve
navigability as the framework grows.

---

## 2026-03-15 Review Update

### Coding Standards

**[LOW]** `service-sdk/src/lifecycle.rs:270` — `#[allow(unused_variables)]` suppresses a warning on
the `resolve_connection` parameter without any comment naming the feature that makes it conditionally
unused. Per the project policy, every `#[allow()]` on a feature-gated item requires an inline comment
of the form `// used by feature "X"`. This is a violation. The `#[allow(unreachable_code)]` at line
346 (with a comment explaining the `zeroconf` feature) is the approved pattern.

**[APPROVED]** `service-sdk/src/lifecycle.rs:346` — `#[allow(unreachable_code)]` with an inline
comment referencing the `zeroconf` feature. Correctly follows the mandatory-comment policy.

**[APPROVED]** `plugins/infrastructure/registry/src/lib.rs:178,191` — `#[allow(unused_mut)]` on
mutation inside feature-gated blocks. The mutation is conditionally used depending on which
notification-channel features are enabled; the allow annotation is acceptable.

**[APPROVED]** `plugins/infrastructure/registry/src/registry.rs:650,652` — `#[allow(unused_variables)]`
and `#[allow(unused_mut)]` with explicit comment `'Used conditionally by feature-gated notification
channel blocks'`. Correctly follows the mandatory-comment policy.

### Crate Boundaries

**[MEDIUM]** `uptrakit-agent-core` imports `plugin-infrastructure-registry`. This pulls the full
compiled plugin tree (all release and package-manager plugins) into both agent binaries. `agent-core`
needs only the plugin trait and dispatch interfaces, which live in `plugin-infrastructure-core`.
Moving the registry dependency to the binary-level `Cargo.toml` of `uptrakit-agent` and
`uptrakit-agent-ssh` would reduce `agent-core`'s compilation footprint and improve layering.

**[MEDIUM]** `web-api-types` imports `uptrakit-internal-wire`. External-facing API types are
entangled with the internal WebSocket protocol crate. Types shared between the HTTP API and the wire
protocol should be extracted to a neutral shared location (e.g., `uptrakit-shared-types`) rather than
`web-api-types` depending on the wire crate. This coupling means any change to the internal protocol
triggers recompilation of every consumer of `web-api-types`.
