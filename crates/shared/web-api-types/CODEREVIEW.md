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

## Extensibility Assessment

This crate is the primary interface for external API client developers. Several issues impact usability:

1. **No prelude or root-level re-exports**: The `lib.rs` exposes 28 `pub mod` declarations with zero
   re-exports. An external developer building an API client must know which module contains each type and
   write verbose imports like `use uptrakit_web_api_types::agents::AgentResponse`. A `pub mod prelude` or
   flat re-exports of the most commonly used types would dramatically improve ergonomics.

2. **Duplicate types with wire crate**: `ServiceType` and `MqttTransport` exist in both this crate and the
   wire crate. The `update_hooks` module correctly re-exports `HookShell` from wire, proving the pattern
   works. The other duplicates should follow suit.

3. **`CreateProviderConfigRequest.provider_type` is `String`**: This loses type safety. Using `ProviderType`
   from `shared-types` would catch errors at compile time and deserialization time.

4. **`ListAgentsQuery.status` is `Option<String>`**: Should be `Option<AgentStatus>` for type safety.

5. **Dependency on `uptrakit-internal-wire`**: An HTTP-only API client that never uses WebSocket still pulls
   in the wire protocol as a transitive dependency. The dependency exists to re-export `HookShell` and
   `MqttTransport`. If these types moved to `shared-types`, the wire dependency could be removed.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| WAT-01 | Low | Code Quality | `CreateProviderConfigRequest`, `UpdateProviderConfigRequest`, and `ProviderConfigResponse` lack `Debug` derive. These types do not contain secrets, so `Debug` should be safe to add for logging and error reporting. | `src/provider_configs.rs:7`, `src/provider_configs.rs:20`, `src/provider_configs.rs:28` |
| ~~WAT-02~~ | ~~Low~~ | ~~Code Quality~~ | ~~`DeviceAuthPollResponse.status` is `String`.~~ **FIXED.** `DeviceAuthStatus` enum (Pending, Authorized, Expired) defined in `shared-types` with feature-gated `DeriveActiveEnum` and `ToSchema`. Used across DB entity, device flow logic, and API response. | `src/device_auth.rs` |
| ~~WAT-03~~ | ~~Low~~ | ~~Code Quality~~ | ~~`SystemAlert.severity` is `String`.~~ **FIXED.** `AlertSeverity` enum (Info, Warning, Error, Critical) added with `FromStr`/`Display`/`as_str()`, `#[serde(rename_all = "snake_case")]`, and `ToSchema`. `SystemAlert.severity` changed from `String` to `AlertSeverity`. Tests added (serde roundtrip, display, fromstr, as_str). Exported via prelude. | `src/system_alerts.rs` |
| WAT-04 | Info | Security | Auth request DTOs (`RegisterRequest`, `LoginRequest`) use plain `String` for passwords. Typical for HTTP DTOs, but using `SecretString` from `uptrakit-shared-types` would add accidental-logging protection. The absence of `Debug` on these types partially mitigates this. | `src/auth.rs:20`, `src/auth.rs:35` |
| WAT-05 | Info | Code Quality | Inconsistent `Debug` derive across the crate. Some types have it (`SystemAlert`, `ErrorResponse`, `Permission`, enums), others don't (most request/response DTOs). A consistent policy would improve debuggability. | Multiple files |
| ~~WAT-06~~ | ~~Major~~ | ~~Extensibility~~ | ~~No prelude or root-level re-exports.~~ **FIXED.** `pub mod prelude` added with ~35 commonly used type re-exports grouped by domain (auth, agents, hosts, services, software items, provider configs, update history, API tokens, OIDC, MQTT, settings, common). | `src/prelude.rs` |
| ~~WAT-07~~ | ~~Major~~ | ~~Extensibility~~ | ~~`ServiceType` and `MqttTransport` duplicated.~~ **FIXED.** Both types now imported from `shared-types` (with `openapi` feature). Local definitions removed. | `src/services.rs`, `src/mqtt_transport.rs` |
| ~~WAT-08~~ | ~~Minor~~ | ~~Extensibility~~ | ~~`CreateProviderConfigRequest.provider_type` is `String`.~~ **FIXED.** Changed to typed `ProviderType` from `shared-types`. Serde validates on deserialization. | `src/provider_configs.rs` |
| ~~WAT-09~~ | ~~Minor~~ | ~~Extensibility~~ | ~~`ListAgentsQuery.status` is `Option<String>`.~~ **FIXED.** Changed to `Option<AgentStatus>`. | `src/agents.rs` |
| WAT-10 | Minor | Extensibility | ~~`Permission` enum uses `parse()` returning `Option` instead of implementing `FromStr`.~~ **RESOLVED** -- `Permission` now implements `FromStr` with typed `ParsePermissionError`. | `src/permissions.rs` |

## Verdict

**Pass.** Well-structured DTO crate with strong validation (especially update hooks), consistent serde patterns, and comprehensive test coverage. The missing prelude (WAT-06) and type duplication (WAT-07) are the most impactful extensibility findings. The `String`-typed fields (WAT-02, WAT-03, WAT-08, WAT-09) are the most actionable for type safety improvement.
