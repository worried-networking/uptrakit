# Code Review: `uptrakit-web-api-types`

**Date:** 2026-02-17
**Reviewer:** Claude Opus 4.6 (automated)
**Scope:** Architecture, security, code quality, coding standards
**Overall quality: GOOD (82/100)**

All 114 tests pass. 1 doc-test passes.

---

## Architecture

The crate defines HTTP request/response DTOs for the Uptrakit web API. It contains 30 domain modules covering auth,
agents, hosts, software items, services, MQTT, OIDC, settings, scheduler, permissions, pagination, and more. Feature-
gated for `openapi` (utoipa schema generation).

---

## Code Quality Findings

### HIGH: No field validation on request types

None of the request types perform input validation. This is the most significant finding.

**Critical examples:**

| Type | Field | Issue |
| --- | --- | --- |
| `RegisterRequest` | `email` | No format validation |
| `RegisterRequest` | `password` | No minimum length (OpenAPI says min 8) |
| `LoginRequest` | `email`, `password` | No validation |
| `CreateOidcProviderRequest` | `issuer_url` | No URL format validation |
| `CreateOidcProviderRequest` | `slug` | No slug format validation |
| `UpdateScheduledTaskRequest` | `cron_expression` | No format validation |
| `UpdateNetworkSettingsRequest` | `trusted_proxies` | No IP/CIDR format validation |
| `CreateSoftwareItemRequest` | mutual exclusivity | `provider_config_id` / `provider_config` not enforced |

**Exception:** `update_hooks.rs` provides proper `validate()` methods -- this is the gold standard the rest should
follow.

**Recommendation:** Add `validate()` methods to all request types, starting with security-relevant inputs (email, URLs,
passwords, CIDR ranges). Follow the pattern in `update_hooks.rs`.

### HIGH: Secrets in plain `String` fields

Several types contain secrets as plain `String`, meaning they can appear in Debug output, log messages, and error traces:

- `RegisterRequest.password`, `LoginRequest.password`
- `CreateOidcProviderRequest.client_secret`, `UpdateOidcProviderRequest.client_secret`
- `CreateMqttClientRequest.password`, `UpdateMqttClientRequest.password`
- `AuthResponse.access_token`, `AuthResponse.refresh_token`
- `DeviceAuthPollResponse.token`, `EnrollmentTokenResponse.token`
- `CreateApiTokenResponse.token`
- `UpdateRegistrationSettingsRequest.token`

The wire crate already uses `SecretString` from `secrecy`. Response types like `OidcProviderResponse` and
`MqttClientResponse` correctly mask secrets with `has_*` boolean fields.

**Recommendation:** Use `SecretString` for password/secret fields in request types, or at minimum ensure custom `Debug`
implementations that redact secrets.

### MEDIUM: Inconsistent `skip_serializing_if` for `Option` fields

Most response structs serialize `null` for `None` values. `DeviceAuthPollResponse` and `ErrorResponse` skip `None`
entirely. Both approaches are valid, but inconsistency within the same API confuses consumers.

### MEDIUM: `UpdateMqttClientRequest` type mismatch between `username` and `password`

**File:** `src/settings_mqtt.rs`, lines 80-82

`username` uses `Option<serde_json::Value>` (to distinguish "set to null to clear" vs "omit to keep"), while `password`
uses `Option<String>` (cannot distinguish). These should use the same pattern.

### MEDIUM: `ListAgentsQuery` uses `ToSchema` instead of `IntoParams`

**File:** `src/agents.rs`, line 57

Query parameter structs should use `utoipa::IntoParams`, not `ToSchema`. Compare with `ListServicesQuery` and
`ListMqttServicesQuery` which correctly use `IntoParams`.

### MEDIUM: `uses_remaining` allows negative values

**File:** `src/mqtt_services.rs`, line 99

`Option<i32>` allows negative values. Should be `Option<u32>` since a negative remaining-uses count is semantically
meaningless. Same issue in `MqttEnrollmentTokenResponse` and `MqttEnrollmentTokenListResponse`.

### MEDIUM: Missing test coverage for many modules

The following have **zero tests**: `api_tokens`, `hosts`, `oidc_auth`, `provider_configs`, `scheduler`, `server_cert`,
`services`, `settings`, `settings_agent_certs`, `settings_auth`, `settings_ca`, `settings_combined`, `settings_network`,
`software_items`, `registration`.

Well-tested modules (good examples): `pagination`, `mqtt_url`, `update_hooks`, `system_alerts`, `settings_mqtt`.

### MEDIUM: Coarse permission model

Only 5 permission variants (`ViewSettings`, `ManageSettings`, `ViewAgents`, `ManageAgents`, `ManageGlobalSettings`).
Notable gaps: no `ViewHosts`/`ManageHosts`, no `ManageSoftwareItems`, no `ManageOidcProviders`, no `ManageApiTokens`,
no `ManageScheduledTasks`, no `ViewUpdateHistory`.

### LOW: Duplicate `default_enabled()` functions

`provider_configs.rs` and `software_items.rs` both define identical `pub fn default_enabled() -> bool { true }`.
Should be consolidated.

### LOW: Duplicate message response types

`MessageResponse` in `agents.rs`, `HostMessageResponse` in `hosts.rs`, and similar single-`message` structs in
`server_cert.rs` and `settings_ca.rs`. Consider a single generic `MessageResponse`.

### LOW: Timestamps as `String` instead of typed datetime

Throughout the crate, timestamps use `String`. Using `time::OffsetDateTime` or similar would provide compile-time format
guarantees.

### LOW: Query filter parameters use raw `String` instead of typed enums

`ListMqttServicesQuery.status` is `Option<String>` rather than `Option<MqttServiceStatus>`. Similarly for
`ListServicesQuery.status`, `ListServicesQuery.r#type`, and `UpdateHistoryQuery.status`. Compare with
`ListAgentsQuery.status` which correctly uses `Option<AgentStatus>`.

### LOW: `ProviderConfigResponse.provider_type` is `String` not `ProviderType`

Loses type safety on responses. The request type correctly uses the typed enum.

### LOW: Missing `Display` implementations for some enums

`AgentStatus`, `RegistrationMode`, `UpdateStatus`, and `MqttServiceStatus` have `as_str()` but no `Display`. In
contrast, `Permission` and `AlertSeverity` do implement `Display`.

### LOW: Missing OpenAPI schema derives on update hooks types

None of the types in `update_hooks.rs` have `#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]`. If exposed
via API responses, they need schemas.

### LOW: `HookValidationError` does not use `thiserror`

Uses manual `impl std::error::Error` and `impl Display`. Inconsistent with the rest of the crate.

### PASS: No production `unwrap`/`panic`

All uses confined to `#[cfg(test)]` modules.

### PASS: Pagination implementation

Solid: `u64` throughout, clamps page to min 1, clamps per_page to [1, 1000], `div_ceil` for total pages, correct
1-indexed offset calculation.

### PASS: Secret masking in response types

`OidcProviderResponse`, `MqttClientResponse`, `ProviderConfigResponse` all correctly mask secrets with `has_*` booleans
or "secrets masked" documentation.

### PASS: Hook validation

`update_hooks.rs` provides proper `validate()` methods with clear error messages. This is the best-designed module in
the crate and the pattern others should follow.

### PASS: `MqttUrl` parsing

Robust parser handling IPv6 brackets, default ports, path rejection, trailing slash tolerance, and round-trip
serialization. 16 thorough tests.

---

## Extensibility Positives

- **Clean module organization** -- 30+ modules organized by domain (auth, services, hosts,
  settings, etc.).
- **Comprehensive prelude** re-exporting the most commonly used types.
- **Feature-gated OpenAPI derives** (`#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]`)
  keep the base crate lightweight for non-OpenAPI consumers.
- **No server-side logic** -- pure DTOs with serde derives, exactly as intended.
- **Pagination types** (`PaginationParams`, `PaginatedResponse`) are well-designed and reusable.

---

## Summary

| Category | Status | Notes |
| --- | --- | --- |
| Input validation | **HIGH** | No validation on request types (except update hooks) |
| Secret handling | **HIGH** | Plain `String` for passwords/tokens in request types |
| Wire dependency | PASS | `HookShell` now imported from `uptrakit-shared-types` directly |
| OpenAPI correctness | FAIR | One wrong derive; missing derives on hooks types |
| Serialization | GOOD | Correct attributes; minor consistency issues |
| Pagination | PASS | Well-implemented and well-tested |
| Permission model | FAIR | Very coarse (5 variants); many resources not covered |
| Test coverage | FAIR | 114 tests, but many modules have zero coverage |
| `unwrap`/`panic` | PASS | Zero in production code |
| Type safety | FAIR | Several `String` fields where typed enums would be better |
