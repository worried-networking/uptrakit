# Code Review: uptrakit-web-api-types

## Summary

HTTP request/response type definitions crate (~2500+ lines across 29 source files) providing shared DTOs for the REST API. Covers authentication, agents, hosts, software items, MQTT, OIDC, permissions, settings, update hooks, and more. Feature-gated OpenAPI schema generation via `utoipa`. Extensive test coverage (150+ tests).

## Architecture

- **Module structure**: `lib.rs` exposes 28 public modules. Each module corresponds to an API domain (auth, agents, hosts, etc.).
- **Public API surface**: Request/response structs, enums with `FromStr`/`Display`, error types for parsing, pagination helpers, and validation functions.
- **Dependency choices**: `serde`/`serde_json` (serialization), `thiserror` (parse errors), `utoipa` (optional OpenAPI), `uptrakit-internal-wire` (wire protocol types).
- **Layering**: Shared between the web-api crate (server) and potentially CLI/frontend clients. Correctly positioned as a shared boundary type crate.

## Security & Safety

- **Update hooks validation**: `SystemdServiceHook::validate()` and `DockerComposeHook::validate()` reject shell metacharacters (`; | & $ \` etc.) and path traversal (`..`). Comprehensive test coverage for injection attempts.
- **Auth DTOs use plain `String` for passwords**: `RegisterRequest.password` (`src/auth.rs:20`), `LoginRequest.password` (`src/auth.rs:35`). These are HTTP DTO types where plaintext passwords are expected in the request body, but using `SecretString` would prevent accidental logging.
- **No `Debug` derive on sensitive types**: Auth request types lack `Debug`, which is actually protective -- adding `Debug` later without using `SecretString` for passwords would expose secrets.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: Parse error types (`ParseAgentStatusError`, `ParseUpdateStatusError`, etc.) use `thiserror` with clear messages. `MqttUrlError` has 7 variants covering all URL parsing failure modes.
- **Serialization**: Consistent use of `#[serde(skip_serializing_if = "Option::is_none")]` for optional fields, `#[serde(default = "...")]` for defaults, `#[serde(rename_all = "snake_case")]` for enums.
- **Pagination**: `PaginationParams` clamps values to `[1, MAX_PAGE_SIZE]` range, preventing abuse.
- **Test coverage**: 150+ tests across `lib.rs` (serde roundtrips, defaults, skip_serializing_if, pagination), `mqtt_transport.rs` (14 tests), `mqtt_url.rs` (23 tests), and `update_hooks.rs` (50+ tests including injection prevention).
- **Missing `Debug` derives**: Several request/response types lack `Debug`. This is partially protective (prevents password logging) but inconsistent across the crate.

## Coding Standards Compliance

- Parse error types use `thiserror` -- compliant.
- `rootcause` is not used here (leaf DTO types, no complex error chains) -- acceptable.
- No `#[allow()]` directives.
- Enum types consistently implement `FromStr`, `Display`/`as_str`, and serde traits.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| WAT-01 | Low | Code Quality | `CreateProviderConfigRequest`, `UpdateProviderConfigRequest`, and `ProviderConfigResponse` lack `Debug` derive. These types do not contain secrets, so `Debug` should be safe to add for logging and error reporting. | `src/provider_configs.rs:7`, `src/provider_configs.rs:20`, `src/provider_configs.rs:28` |
| WAT-02 | Low | Code Quality | `DeviceAuthPollResponse.status` is `String`. Should be a typed enum (e.g., `AuthorizationPending`, `Complete`, `SlowDown`, `Expired`) for type safety. Tests show values `"authorization_pending"` and `"complete"`. | `src/device_auth.rs:28` |
| WAT-03 | Low | Code Quality | `SystemAlert.severity` is `String`. Could be a typed enum (e.g., `Critical`, `Warning`, `Info`) for consistent severity handling across the system. | `src/system_alerts.rs:7` |
| WAT-04 | Info | Security | Auth request DTOs (`RegisterRequest`, `LoginRequest`) use plain `String` for passwords. Typical for HTTP DTOs, but using `SecretString` from `uptrakit-shared-types` would add accidental-logging protection. The absence of `Debug` on these types partially mitigates this. | `src/auth.rs:20`, `src/auth.rs:35` |
| WAT-05 | Info | Code Quality | Inconsistent `Debug` derive across the crate. Some types have it (`SystemAlert`, `ErrorResponse`, `Permission`, enums), others don't (most request/response DTOs). A consistent policy would improve debuggability. | Multiple files |

## Verdict

**Pass.** Well-structured DTO crate with strong validation (especially update hooks), consistent serde patterns, and comprehensive test coverage. The `String`-typed status/severity fields (WAT-02, WAT-03) are the most actionable findings for improving type safety. Auth password handling (WAT-04) is acceptable for HTTP DTOs but could benefit from `SecretString`.
