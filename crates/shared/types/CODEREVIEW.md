# Code Review: uptrakit-shared-types

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-shared-types` (~2,892 LoC across 15 source files) defines the foundational domain types
used throughout the workspace: `PluginType`, `HookShell`, `SecretString`, `ServiceStatus`,
`DeviceAuthStatus`, `MqttTransport`, and more. Each type follows a consistent pattern: typed
`FromStr` with a dedicated error type, `Display` via `as_str()`, and optional SeaORM/OpenAPI
integration behind feature gates. The crate is high quality with no Critical or High issues.

The main concern is that several public enums that could plausibly gain new variants in future
releases lack `#[non_exhaustive]`, unlike the wire-protocol enums in `uptrakit-internal-wire`
which correctly apply it.

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

No security issues found.

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

No code quality issues found.

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

**[MEDIUM]** `src/software_discovery_state.rs:23`, `src/service_status.rs`,
`src/output_stream_type.rs`, `src/mqtt_connection_status.rs`, `src/device_auth_status.rs` --
Five public domain enums lack `#[non_exhaustive]` despite being cross-crate types.
`SoftwareDiscoveryState`, `ServiceStatus`, `OutputStreamType`, `MqttClientConnectionStatus`,
and `DeviceAuthStatus` could plausibly gain new variants. This is inconsistent with
`PluginType` and `HookShell` which correctly have the attribute. Similarly, `MqttTransport`
in `src/mqtt_transport.rs` lacks it.
