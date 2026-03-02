# Code Review: uptrakit-shared-types

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-shared-types` (~2,892 LoC across 15 source files) defines the foundational domain types
used throughout the workspace: `PluginType`, `HookShell`, `SecretString`, `ServiceStatus`,
`DeviceAuthStatus`, `MqttTransport`, and more. Each type follows a consistent pattern: typed
`FromStr` with a dedicated error type, `Display` via `as_str()`, and optional SeaORM/OpenAPI
integration behind feature gates. The crate is high quality with no outstanding issues.

## Architecture

### Strengths

- `Cargo.toml:9-13` -- `sea-orm` and `openapi` (utoipa) integrations are behind optional Cargo
  features. Crates that need only the base types incur zero ORM or schema-generation cost.
- `src/lib.rs:1-32` -- Clean module structure with selective re-exports. Each domain type lives
  in its own module with a dedicated error type.
- `src/secret_string.rs` -- `SecretString` defined centrally and re-exported widely. Used
  throughout `uptrakit-web-api-types` and `uptrakit-internal-wire` for all credential fields.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/secret_string.rs` -- `SecretString` wraps a plain `String` with `Debug` producing
  `"***"` and `Display` producing `"***"`. `Zeroizing` on drop via `zeroize` dependency
  ensures secret material is scrubbed from memory.
- Zero `unsafe` in the entire crate.

### Issues

**[LOW]** `src/masked_email.rs:26` -- `ZeroizeOnDrop` added to `MaskedEmail` is a
category mismatch. `MaskedEmail` stores a plain email address (e.g.
`user@example.com`) that is routinely serialised to JSON, stored in the database in
cleartext, and displayed (masked) to operators in logs and API responses. It is not a
secret: it is shared with users and persists on disk. Adding `ZeroizeOnDrop` alongside
`SecretString` (which stores passwords and bearer tokens) may mislead future readers
into thinking email addresses have the same secrecy requirements as credentials. The
`ZeroizeOnDrop` derive also has no measurable security effect here: any clones produced
before the value is dropped (e.g. by `Clone`, iterator adaptors, or JSON serialisation
intermediates) are not covered by the zeroization. Consider reverting and documenting
why `MaskedEmail` differs from `SecretString`.

## Code Quality

### Strengths

- `src/plugin_types.rs` -- 11 tests covering serialization round-trips for all `PluginType`
  variants (including `Other(String)`), `Display`, `FromStr` (valid and invalid), `as_str` /
  `Display` consistency, and optional field omission for `ReleaseAsset`/`ReleaseInfo`.
- Every `FromStr` implementation pairs with a dedicated typed error (e.g.,
  `ParsePluginTypeError`, `ParseHookShellError`). No `FromStr` returns `String` as its error
  type.
- `src/masked_email.rs` -- `MaskedEmail` type with comprehensive validation tests covering
  Unicode, edge cases, and format preservation.

### Issues

**[LOW]** `src/masked_email.rs:26` -- No test verifies the zeroization behaviour introduced
in commit `5da34db`. `SecretString` has a comparable test gap, but there at least the type's
secrecy justification is unambiguous. If `ZeroizeOnDrop` is retained on `MaskedEmail`, a
test using a raw pointer read after drop (in a `#[test]` with `unsafe`) or a canary-value
check would document the intent and catch future regressions (e.g. if the inner `String` is
moved out before drop).

## High Availability

### Strengths

N/A -- Pure type definitions with no I/O, no async, no state.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- `Cargo.toml:25-26` -- Uses workspace lints. Zero `#[allow(clippy::...)]` suppressions.
- Consistent `as_str()` / `FromStr` / `Display` pattern across all domain enums. SeaORM
  derives gated behind `sea-orm` feature flag.
- `src/plugin_types.rs` -- `PluginType` correctly includes `#[non_exhaustive]` and
  `Other(String)` for forward compatibility.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `src/plugin_types.rs` -- `PluginType::Other(String)` with `#[non_exhaustive]` matches the
  wire protocol's forward-compatibility pattern. Unknown plugin types are preserved through
  round-trips.
- `src/hook_shell.rs` -- `HookShell` has `#[non_exhaustive]` with typed variants.

### Issues

**[MEDIUM]** `src/batch_status.rs:8-29` -- `BatchStatus` has `#[non_exhaustive]` but no
`Other(String)` catch-all variant. New status values added by a newer controller will fail
client deserialization. Unlike `PluginType` which correctly includes `Other(String)` for
forward compatibility, `BatchStatus` silently breaks older clients when extended.

**[MEDIUM]** `src/update_category.rs:13-35` -- `UpdateCategory` lacks an `Other` variant.
Same concern as `BatchStatus`: new categories added server-side will cause deserialization
failures on clients running an older version of this crate.

## Tests

### Strengths

- `src/plugin_types.rs` -- 11 tests cover all `PluginType` variants for `as_str()`,
  `Display`, `FromStr` (valid and invalid), the `Other(String)` passthrough, `ReleaseAsset`
  and `ReleaseInfo` optional-field omission on serialisation.
- `src/masked_email.rs` -- 16+ tests cover `MaskedEmail` validation: valid addresses, Unicode
  local-part, empty local-part rejection, missing-at rejection, empty domain rejection,
  domain-only forms, and round-trip `Display` / `Debug` formatting.
- `src/hook_shell.rs` -- 10 tests cover `HookShell` `FromStr` (all shell variants, invalid),
  `as_str`/`Display` consistency, and `#[serde(rename_all)]` alignment.
- `src/service_status.rs` -- Four tests cover `ServiceStatus` `as_str`, `Display`, `FromStr`
  valid and invalid paths.
- `src/session_token_type.rs` -- Five tests cover `SessionTokenType` round-trips.
- `src/discovered_software.rs` -- Two tests for `DiscoveredSoftware` field validation.
- `src/batch_status.rs:73` -- 4 tests covering serde round-trip, display, and `FromStr` for
  all `BatchStatus` variants.
- `src/update_category.rs:70` -- 4 tests covering all `UpdateCategory` variants.
- All tests use synchronous `#[test]` (correct -- no async I/O anywhere in this crate).

### Issues

**[LOW]** `src/secret_string.rs` -- `SecretString` has no inline tests for `Debug` redaction
(`"***"`), `Display` redaction, or `Zeroize` on drop. Although `SecretString` is a thin
wrapper over a well-known pattern, a one-line `assert_eq!(format!("{:?}", s), "***")` test
would prevent inadvertent removal of the redaction during future refactors.

**[LOW]** `src/mqtt_transport.rs`, `src/mqtt_connection_status.rs`, `src/output_stream_type.rs`,
`src/device_auth_status.rs`, `src/plugin_role.rs`, `src/plugin_capability.rs`, and
`src/software_discovery_state.rs` have no tests despite following the same `as_str`/`FromStr`
pattern as the tested types. A regression in any `FromStr` implementation would not be
detected until runtime.
