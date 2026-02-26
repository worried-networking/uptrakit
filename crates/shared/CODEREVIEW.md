# CODEREVIEW — Shared Libraries

**Crates covered:** `uptrakit-crypto`, `uptrakit-shared-types`, `uptrakit-shared-macros`, `uptrakit-command`, `uptrakit-agent-core`, `uptrakit-directories`, `uptrakit-build-info`, `uptrakit-openapi-client`, `uptrakit-web-api-types`

---

## Summary

These nine crates form the foundational layer that every other crate in the workspace builds upon. The overall quality is high: cryptographic primitives are correctly implemented, type safety is enforced throughout, and the abstractions are clean and well-documented. The most serious systemic issue is an architectural layering violation in `uptrakit-crypto` where an unconditional `sea-orm` dependency couples a foundational primitive to the ORM.

---

## Architecture

### Strengths

- **Clean dependency layering (with one exception).** `uptrakit-shared-types` correctly gates its `sea-orm` and `openapi` (utoipa) integrations behind optional Cargo features (`sea-orm` and `openapi`). `uptrakit-web-api-types` mirrors this pattern with its own `openapi` feature gate. Any crate that does not need ORM or OpenAPI annotation machinery incurs zero cost and no transitive dependency.

- **Correct abstraction boundaries for `uptrakit-agent-core`.** The crate delegates all plugin management to `uptrakit-plugin-registry`, keeps transport concerns in `uptrakit-service-sdk`, and injects execution via the `CommandExecutor` trait from `uptrakit-command`. The three concerns — plugin logic, transport, and execution — never bleed across boundaries at this layer.

- **`uptrakit-openapi-client` re-export strategy.** The client re-exports `uptrakit-web-api-types` as `types`, `reqwest::StatusCode`, `reqwest::Error`, and `uuid::Uuid` under a single facade. Downstream crates (the CLI in particular) depend on `uptrakit-openapi-client` alone and do not accumulate a web of transitive explicit dependencies. This is a deliberate and sound API surface decision.

- **`uptrakit-build-info` compile-time / runtime separation.** `emit_enabled_features_env()` runs in a `build.rs` context, and `BuildInfo::current()` runs at runtime. The two phases are cleanly separated: the build script emits a `UPTRAKIT_BUILD_ENABLED_FEATURES` env var via `cargo:rustc-env=`, and the runtime code reads it via `env!()`. No mutable global state, no `OnceLock`, no runtime I/O.

- **`uptrakit-directories` async-first design.** All public I/O functions are `async` (backed by `tokio::fs`), which the module documentation correctly identifies as eliminating TOCTOU risks. Platform permission hardening (0700 dirs, 0600 files) is appropriately conditioned on `#[cfg(unix)]` with documentation calling out that non-Unix paths are not security-hardened.

---

## Security & Safety

### Strengths

- **AES-256-GCM with FIPS-validated primitives.** `uptrakit-crypto` uses `aws-lc-rs`, which wraps AWS-LC (a fork of BoringSSL with FIPS 140-3 validation). The nonce is a randomly generated 96-bit value drawn from `rand::rng()`. The `ENC:v1:` version prefix enables future cipher migration without a full re-encryption pass.

- **`Zeroizing<[u8; 32]>` for the master key.** The `MASTER_KEY` static is typed as `OnceLock<Zeroizing<[u8; 32]>>`. If the value were ever dropped (not normally expected for a `'static`), the key material would be zeroed. `init_master_key()` accepts a `Zeroizing<[u8; 32]>` parameter, ensuring the caller cannot accidentally hand over a plain array.

- **`EncryptedString` `Debug` and `Display` redaction.** `fmt::Debug` writes `"EncryptedString(***)"` and `fmt::Display` writes `"***REDACTED***"`. Both implementations are covered by an automated test (`test_debug_display_redact`). This prevents accidental exposure of plaintext secrets in log output, panic messages, or error chains.

- **`SecretString` defined centrally and re-exported widely.** `uptrakit-shared-types` defines `SecretString` and it is re-exported from `uptrakit-openapi-client` and used throughout `uptrakit-web-api-types` for all credential fields. All password, token, and OIDC secret fields carry this type at API boundaries.

- **Key verification sentinel.** `create_key_verification_token()` and `verify_key_verification_token()` allow HA deployments to verify that all instances share the same master key before accepting traffic. The mechanism is correctly implemented: decryption failure maps to `MasterKeyMismatch`, preventing oracle attacks on the sentinel.

- **Zero `unsafe` in production code.** The entire set of shared crates contains no `unsafe` blocks outside of test code. The two `unsafe` blocks that do exist are in test helpers (see Issues).

### Issues

**[SEVERITY: Medium]** `crates/shared/crypto/src/lib.rs:244-251` — `EncryptedString::new` silently falls back to plaintext when no master key is configured

When `master_key_available()` returns `false`, `EncryptedString::new` logs a single `tracing::warn!` and stores the plaintext value as the `db_value`. There is no startup guard, no environment check, and no way for a caller to distinguish a successfully encrypted value from a silently unencrypted one at the call site. In a production deployment where the master key environment variable is accidentally absent (misconfigured container, missing secret mount), OIDC client secrets, provider credentials, and similar sensitive values would be stored in the database in plaintext. The only observable signal is a single `WARN`-level log line that may be missed in high-volume startup output. The doc-comment on `EncryptedString` describes this as "development mode", but there is no code-level enforcement preventing this path in production builds. Fix: add a startup assertion (e.g., checked at `AppState` build time) that `master_key_available()` returns `true` unless an explicit development-mode flag is set. Alternatively, expose a `EncryptedString::new_require_key` variant that returns `Err` when no key is present and deprecate the silent fallback path.

**[SEVERITY: Low]** `crates/shared/directories/src/lib.rs:829,837` — `unsafe` env var mutation in tests creates data-race risk

```rust
unsafe { std::env::remove_var("HOME") };
// ... test logic ...
unsafe { std::env::set_var("HOME", val) };
```

These two calls appear in a `#[test]` function (`expand_tilde_works_without_home_env`). Rust's test harness runs tests in the same process on multiple threads by default. Mutating environment variables without synchronization is undefined behavior when any other thread simultaneously reads them (e.g., via `std::env::var` in another test). Fix: gate this test with `#[tokio::test]` (or plain `#[test]`) and acquire a `Mutex` lock shared across all env-mutating tests in the module before calling `remove_var`/`set_var`. Alternatively, annotate with `#[serial_test::serial]` if the crate adds `serial_test` as a dev-dependency.

---

## Code Quality

### Strengths

- **`uptrakit-shared-macros` single responsibility.** The crate exports exactly one macro, `impl_report_conversion!`. The three arms (single variant-mapping, single closure-based, multi-variant) are clearly documented with expansion examples and the one known constraint (target identifier must be a single segment, not a path) is explicitly documented with a rationale. Zero dead code, zero suppressed lints.

- **`CommandSpec` builder pattern.** `CommandSpec` uses a clean `with_working_dir` / `with_timeout` builder API annotated with `#[must_use]`. The `resolve()` method consolidates all execution-mode specifics (shell wrapping, argument construction) in one place, keeping `LocalCommandExecutor::execute` free of branching. The `Exec` and `Shell` modes are well-separated with `set -euo pipefail` injection handled in `wrap_command_for_shell` rather than inline.

- **`uptrakit-build-info` deterministic output.** `normalize_feature_list` sorts and deduplicates the parsed feature list before storage. This guarantees that `render_human()` and `Serialize` output are stable regardless of the order in which Cargo emits `CARGO_CFG_FEATURE`. The `render_human_uses_stable_keys_and_order` test validates the exact string output, catching any future column reorder.

- **`uptrakit-openapi-client` paginated fetch.** `fetch_all_pages` is a clean generic implementation that serializes the base query to JSON, overrides only the `page` and `per_page` keys on each iteration, and stops when `page >= total_pages`. The logic correctly handles the zero-pages edge case (`total_pages == 0`) without panicking.

- **`RetryConfig` documentation.** The retry logic in `uptrakit-openapi-client` is clearly documented: which status codes trigger retry, how exponential backoff is computed (doubling, capped at `max_delay`), and which codes are never retried (4xx, network errors, auth failures). The `Retry-After` header parsing path is documented as numeric-seconds-only. This level of documentation on a retry implementation is uncommon and highly valuable.

- **Typed errors throughout `uptrakit-shared-types`.** Every domain type with a `FromStr` implementation pairs it with a dedicated error type (e.g., `ParsePluginTypeError`, `ParseHookShellError`, `ParseServiceTypeError`). No `FromStr` implementation returns `String` as its error type. This is correct Rust idiom and enables callers to match on specific failure cases.

#### 2026-02-24 Review

#### Strengths

- **Typed `FromStr` error types for all domain enums.** Every `FromStr` implementation pairs with a dedicated error type. No `FromStr` returns `String` as error.

#### Issues

**[SEVERITY: Low]** `crates/shared/command/src/executor.rs:126` — Unnecessary `clone()` in `CommandSpec::resolve()` for Exec mode

Could take ownership or return references.

**[SEVERITY: Low]** `crates/shared/openapi-client/src/mock.rs:232,259` — `unwrap()` on `serde_json::to_string` in mock helper methods outside test code

Should use `expect()` with infallibility reason.

### Issues

**[SEVERITY: Medium]** `crates/shared/command/src/executor.rs` — `execute` and `execute_quiet` share identical timeout logic across two methods

Both `LocalCommandExecutor::execute` (lines 176-183) and `LocalCommandExecutor::execute_quiet` (lines 192-199) contain the exact same `tokio::time::timeout` / `report!(CommandError::TimedOut)` block. The only difference between the two methods is whether `output_tx` is passed to `run_command_exec_impl`. This duplication means any future change to timeout behavior (e.g., exposing a `cancelled` token, killing the child process) must be applied in two places. Fix: extract a private `run_with_optional_timeout(fut, timeout) -> crate::Result<(String, i32)>` helper.

---

## Tests

### Strengths

- **`uptrakit-crypto` test suite is comprehensive.** Fourteen tests cover: round-trip correctness, nonce uniqueness, prefix detection, `Debug`/`Display` redaction, ciphertext tampering, SeaORM `Value` round-trip, `ValueType` error cases, `Nullable` contract, clone/equality semantics, key verification token creation and tampering, and all error variant paths (`AlreadyInitialized`, `CiphertextTooShort`, `Decryption`, `HexDecode`). The `TEST_LOCK: Mutex<()>` pattern correctly serializes all tests that touch the global `MASTER_KEY` within the same test binary.

- **`uptrakit-command` timeout tests use `start_paused = true`.** Both `execute_quiet_timeout_fires` and `execute_timeout_fires` are annotated with `#[tokio::test(start_paused = true)]` and advance Tokio's mock clock by 10 seconds, triggering a 5-second timeout without burning wall-clock time. This is the correct pattern per AGENTS.md and ensures CI stability.

- **`uptrakit-openapi-client` retry tests verify call counts.** The retry-exhaustion tests (`retry_exhausted_on_repeated_503`, `retry_exhausted_on_repeated_429`) use `mock.assert_calls(3)` to verify the exact number of HTTP requests made, confirming that the retry loop iterates the expected number of times. The no-retry tests (`no_retry_on_400`, `no_retry_on_401`) confirm that `mock.assert_calls(1)` holds for non-retriable status codes.

- **`uptrakit-shared-types` serialization tests.** `plugin_types.rs` carries 11 tests covering serialization round-trips for all `PluginType` variants (including `Other(String)`), `Display`, `FromStr` (valid and invalid), `as_str` / `Display` consistency, and optional field omission for `ReleaseAsset`/`ReleaseInfo`. All are deterministic and have no external dependencies.

- **`uptrakit-build-info` output stability test.** `render_human_uses_stable_keys_and_order` constructs a `BuildInfo` with fixed values and asserts the exact multi-line string output. This is a contract test that will catch any future field reordering or format change.

#### 2026-02-24 Review

#### Issues

**[SEVERITY: Medium]** `crates/shared/service-sdk/src/ca.rs` and `identity.rs` — 21 service-sdk tests use bare `#[tokio::test]`

6 in `ca.rs`, 15 in `identity.rs`. Per `testing.md`, the annotation is required regardless of time-dependence.

**[SEVERITY: Medium]** `crates/shared/agent-core/src/version_check.rs` and `update.rs` — Eight `agent-core` tests use bare `#[tokio::test]`

The `update.rs` module uses `tokio::time::timeout` in production code, creating a maintenance hazard.

**[SEVERITY: Medium]** `crates/shared/command/src/command.rs` and `executor.rs` — 21 `uptrakit-command` tests use bare `#[tokio::test]`

Only 2 of 5 executor tests use `start_paused`. The module uses `tokio::time::timeout` in production code.

### Issues

**[SEVERITY: Medium]** `crates/shared/openapi-client/src/lib.rs:687-885` — Retry-backoff tests do not verify actual delay durations

The retry tests run with `start_paused = false` (the default). They confirm the correct number of HTTP calls and that error values are propagated, but they never assert the inter-retry delay values. For example, `retry_exhausted_on_repeated_429` sets `Retry-After: 1` and uses `retrying_client` which has `initial_delay: 1ms` and `max_delay: 10ms`. The test passes regardless of whether the sleep duration was 1 millisecond or 1 full second — the delay is simply not measured. A regression that changed `Duration::from_secs` to `Duration::from_millis` in the `Retry-After` parsing path, or inverted the `min(max_delay)` cap, would pass all existing tests. Fix: switch to `#[tokio::test(start_paused = true)]` and measure elapsed virtual time between the first and last mock invocation using `tokio::time::Instant`.

**[SEVERITY: Low]** `crates/shared/directories/src/lib.rs` — Several `#[tokio::test]` tests perform only synchronous filesystem operations

Tests such as `expand_tilde_user_syntax_unchanged` and `validate_path_name_*` contain no `await` points and do not require an async runtime. Using `#[tokio::test]` for these allocates a runtime that is never used. This is a minor noise issue but could mislead readers into thinking the functions under test are async. Fix: convert to `#[test]` where no `await` is present.

---

## High Availability

### Strengths

- **`uptrakit-command` timeout is non-blocking.** The timeout is implemented via `tokio::time::timeout`, which yields cooperative control back to the runtime when the deadline fires. This means a timed-out command does not block the agent's event loop or prevent other concurrent operations from proceeding.

- **`uptrakit-openapi-client` exponential backoff is correctly bounded.** The backoff factor computation uses `1u32.checked_shl(attempt as u32).unwrap_or(u32::MAX)` to guard against shift overflow, followed by `saturating_mul` and `.min(max_delay)`. Delay is guaranteed to stay within `max_delay` regardless of retry count.

- **`uptrakit-openapi-client` `Retry-After` header is respected.** The 429 path parses the `Retry-After` header as numeric seconds and uses it as the delay, falling back to `initial_delay` only when the header is absent or unparseable. The value is additionally clamped to `max_delay`, preventing a server from forcing an arbitrarily long client pause.

### Issues

**[SEVERITY: Low]** `crates/shared/openapi-client/src/lib.rs` — `Retry-After` parsing only handles numeric seconds, not HTTP-date format

`parse_retry_after` (called at line 234) parses the header value as a `u64`. RFC 9110 §10.2.4 permits `Retry-After` to carry either a delay-seconds integer or an HTTP-date string. A server responding with `Retry-After: Wed, 21 Oct 2025 07:28:00 GMT` would cause `parse_retry_after` to return `None`, silently falling back to `initial_delay`. The fallback is safe, but the limitation is undocumented at the call site and could surprise operators inspecting retry behavior. Fix: add a doc-comment noting the numeric-only constraint, or implement HTTP-date parsing.

---

## Database

### Strengths

- **`EncryptedString` is transparent to ORM callers.** The `ValueType`, `TryGetable`, `Nullable`, and `From<EncryptedString> for Value` implementations make the type a drop-in replacement for `String` in SeaORM entity definitions. Callers do not handle encryption/decryption explicitly. The `from_db` constructor correctly preserves the original DB representation, so a re-read does not re-encrypt, avoiding nonce consumption on read paths.

- **Legacy plaintext migration path.** The `try_from` (line 305-307) and `try_get_by` (line 363-365) implementations both accept values that do not start with `ENC:v1:` and treat them as legacy plaintext. This allows a rolling migration: deploy the new code, then re-encrypt at rest with a background job using `is_db_value_encrypted()`, without requiring a migration step before the deploy.

### Issues

No database-specific issues are unique to these shared crates beyond the `sea-orm` unconditional dependency already noted under Architecture. The DB-level issues identified in Phase 1 (N+1 queries, full table scans, missing indexes) reside in higher-level query modules, not in the shared crates layer.

---

## Coding Standards

### Strengths

- **`uptrakit-web-api-types` `SecretString` coverage.** All credential-bearing response fields (OIDC tokens, device-flow tokens, API key values) use `SecretString`. The `openapi` feature gate ensures utoipa annotations are only compiled when needed, and downstream non-annotated consumers (the CLI) pay no binary size cost for the schema generation machinery.

- **`impl_report_conversion!` macro enforces the `rootcause` error convention.** By generating `ReportConversion` implementations, the macro eliminates the `Report::new()` anti-pattern. All cross-crate error propagation uses `.context_to()` and the macro ensures the conversion is always via `context_transform`, not by constructing a fresh report that loses the original error chain. The macro's own documentation explicitly notes the crate-rename constraint.

- **`CommandSpec` uses `#[must_use]` on builder methods.** Both `with_working_dir` and `with_timeout` carry `#[must_use]`, making it a compile-time warning to call them without using the returned `CommandSpec`. This prevents the common mistake of calling `.with_timeout(...)` on a spec and then continuing to use the original unmodified spec.

- **No `publish = false` inconsistency in `uptrakit-web-api-types`.** The crate omits `publish = false`, which is appropriate given its `description` field suggests it is intended to be consumable externally. All other internal crates (`uptrakit-crypto`, `uptrakit-agent-core`, etc.) correctly carry `publish = false`.

#### 2026-02-24 Review

#### Strengths

- **`Validate` trait is well-designed with consistent implementation pattern.** Structured field-level error reporting via `ValidationError { field, message }`. Seven implementations follow the same pattern.
- **All `FromStr` implementations follow the standard pattern with typed error types.** Every domain type pairs `FromStr` with a dedicated `Parse{TypeName}Error`.

#### Issues

**[SEVERITY: Medium]** `crates/shared/web-api-types/src/api_tokens.rs:8` — `CreateApiTokenRequest` and 5 other request types accept user input without `Validate` implementation

Missing validation: `CreateApiTokenRequest`, `CreateMqttClientRequest`, `UpdateMqttClientRequest`, `CreateAutodiscoveryIgnoreRequest`, `UpdateOidcProviderRequest`, `TriggerUpdateRequest`.

**[SEVERITY: Low]** `crates/shared/types/src/software_discovery_state.rs:23` and 4 others — Five public domain enums lack `#[non_exhaustive]`

`SoftwareDiscoveryState`, `DeviceAuthStatus`, `ServiceStatus`, `OutputStreamType`, `MqttClientConnectionStatus` could plausibly gain new variants.

### Issues

**[SEVERITY: Low]** `crates/shared/openapi-client/src/mock.rs:221,276` — `StatusCode::NO_CONTENT.as_u16()` and `StatusCode::TOO_MANY_REQUESTS.as_u16()` used outside approved serialization sites

AGENTS.md restricts `as_u16()` to documented serialization sites only. Both calls here are inside the `mock` feature (test-only) and pass the numeric status code to `httpmock`'s `then.status(u16)` API, which requires a `u16` rather than a `StatusCode`. The usage is therefore justified by an API constraint rather than preference, but neither call carries an inline comment to that effect. Any reviewer auditing `as_u16()` usage across the codebase will flag these without context. Fix: add a brief inline comment at each site: `// httpmock requires u16 — StatusCode::as_u16() approved here`.

---

## Extensibility

### Strengths

- **`uptrakit-command` `CommandExecutor` is injected via `Arc<dyn CommandExecutor>`.** Plugins receive an executor through dependency injection rather than constructing a `LocalCommandExecutor` directly. This makes plugins independently testable without spawning subprocesses: test code can substitute a `TrackingExecutor` or a `MockExecutor` that records calls and returns canned outputs. The trait is `Send + Sync`, making it safe to store in `Arc`.

- **`uptrakit-openapi-client` `mock` feature is correctly test-only.** `mock = ["dep:httpmock"]` is declared in `[features]`, and `httpmock` appears as both `optional = true` in `[dependencies]` (for the feature activation) and in `[dev-dependencies]` (for tests within the crate itself). Downstream crates activate the feature in their `[dev-dependencies]` section. Production builds never compile `httpmock` or the `mock` module.

- **`uptrakit-build-info` is trivially extensible.** Adding a new field to `BuildInfo` is a single-struct change; `render_human` output format is tested with an exact-match string test, which will fail immediately if a new field is added without updating the renderer. The test acts as a change-detection mechanism, not just a correctness check.

- **`uptrakit-shared-macros` closure arm enables non-trivial error wrapping.** The second macro arm (`$source:ty => $target:ident, $closure:expr`) allows callers to express complex conversions like `|e| ControllerError::WebSocket(Box::new(e))` without writing a full `impl ReportConversion` block. This covers cases where the source error must be heap-allocated (e.g., because the target variant holds a `Box<dyn Error>`), keeping boilerplate to a minimum.

#### 2026-02-24 Review

#### Issues

**[SEVERITY: Medium]** `crates/shared/types/src/service_type.rs:11,61-66` — `ServiceType` has `#[non_exhaustive]` but no `Other(String)` forward-compatibility variant

Unlike `Capability` which uses `Other(String)`, `ServiceType` fails deserialization on unknown strings. Adding `Other(String)` would make the pattern consistent.

**[SEVERITY: Low]** `crates/shared/web-api-types/src/permissions.rs:9` and others — Several public API-facing enums lack `#[non_exhaustive]` unlike wire-protocol enums

`Permission`, `UpdateStatus`, `AlertSeverity`, `TriggerUpdateStatus`, `RegistrationMode` should follow the same convention as wire types.

**[SEVERITY: Low]** `crates/shared/types/src/` — Six shared-types domain enums lack `#[non_exhaustive]` despite being cross-crate types

Inconsistent: `PluginType`, `ServiceType`, `HookShell` have it, but `SoftwareDiscoveryState`, `MqttTransport`, `MqttClientConnectionStatus`, `DeviceAuthStatus`, `OutputStreamType`, `ServiceStatus` do not.

### Issues

**[SEVERITY: Medium]** `crates/shared/agent-core/Cargo.toml:22` — `uptrakit-agent-core` links all plugins unconditionally

`uptrakit-plugin-registry` is an unconditional dependency of `uptrakit-agent-core`. The registry in turn compiles all plugin crates (GitHub, Docker Registry, Homebrew, Proxmox Helper Scripts) into every binary that links `uptrakit-agent-core`. A Linux agent binary currently includes `HomebrewPlugin` even though `brew` will never be present on a Linux host. The plugin's `validate()` call will succeed (it does not check for `brew` at validation time), and the failure only surfaces at runtime when an update assignment for a Homebrew software item arrives. At current scale this is acceptable, but as the plugin set grows the binary size impact will increase and the runtime-failure-only discovery will become harder to diagnose. Fix: introduce `#[cfg(target_os = "macos")]` guards in the registry or plugin-specific Cargo features.
