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

### ~~HIGH: No field validation on request types~~ RESOLVED

**Resolution:** Added a `Validate` trait (in `validation.rs`) with `ValidationError { field, message }`. Implemented
`Validate` for 7 request types: `RegisterRequest` (email format, first_name non-empty, password 8-1024 chars),
`LoginRequest` (email format, password non-empty), `CreateOidcProviderRequest` (name non-empty, slug format,
issuer_url scheme, client_id non-empty), `UpdateScheduledTaskRequest` (cron non-empty, 5 fields),
`UpdateNetworkSettingsRequest` (trusted_proxies items non-empty, real_ip_header non-empty, pki_addr URL format),
`CreateSoftwareItemRequest` (name non-empty, exactly one of provider_config_id/provider_config),
`CreateProviderConfigRequest` (name non-empty). All 7 route handlers wire `req.validate()` at entry, returning 400
on failure. 18 tests added.

### ~~HIGH: Secrets in plain `String` fields~~ (FIXED)

**Resolution:** All secret fields now use `SecretString` from `uptrakit-shared-types`. This includes passwords,
tokens, client secrets, access tokens, and refresh tokens across request and response types. `SecretString`
provides transparent serde, redacted `Debug`/`Display`, and `ZeroizeOnDrop`.

### ~~MEDIUM: Inconsistent `skip_serializing_if` for `Option` fields~~ (FIXED)

**Resolution:** Removed `#[serde(skip_serializing_if = "Option::is_none")]` from all response type `Option` fields
(`DeviceAuthPollResponse`, `ErrorResponse`, `SystemAlert`, `NetworkSettingsResponse`, `ReleaseAssetInfoRequest`,
`TriggerUpdateRequest`). All `Option` fields now consistently serialize as `null` when `None`. Kept
`skip_serializing_if` on request types with PATCH semantics (`CreateMqttClientRequest`, `UpdateMqttClientRequest`)
and configuration types (`UpdateHookConfig`, `HooksConfig`, `DockerComposeHook`) where absent-vs-null distinction
is meaningful.

### ~~MEDIUM: `UpdateMqttClientRequest` type mismatch between `username` and `password`~~ (FIXED)

**Resolution:** `password` now uses `Option<serde_json::Value>` (matching `username`) with three-state
semantics: omit = keep, null = clear, string = set. The route handler parses both fields identically.

### ~~MEDIUM: `ListAgentsQuery` uses `ToSchema` instead of `IntoParams`~~ (NOT APPLICABLE)

**Resolution:** `ListAgentsQuery` does not exist. Agents use `ListServicesQuery` which correctly uses
`IntoParams`. This finding was a false positive.

### ~~MEDIUM: `uses_remaining` allows negative values~~ (FIXED)

**Resolution:** Changed `uses_remaining` from `Option<i32>` to `Option<u32>` in `MqttEnrollmentTokenResponse`,
`MqttEnrollmentTokenListResponse`, and `CreateMqttEnrollmentTokenRequest`.

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

### ~~LOW: Query filter parameters use raw `String` instead of typed enums~~ RESOLVED

**Resolution:** `ListServicesQuery.r#type` is now `Option<ServiceType>`, `ListServicesQuery.status` is now
`Option<ServiceStatus>`, and `UpdateHistoryQuery.status` is now `Option<UpdateStatus>`. Invalid filter values
are rejected at deserialization time by axum/serde with a 400 error. Route handlers, CLI, and openapi-client
tests updated accordingly. (`ListMqttServicesQuery.status` was a false positive -- it does not exist.)

### ~~LOW: `ProviderConfigResponse.provider_type` is `String` not `ProviderType`~~ RESOLVED

**Resolution:** Changed `provider_type` field from `String` to `ProviderType`. The response builder
(`provider_config_response_from`) now returns `Option<ProviderConfigResponse>`, logging and skipping rows with
invalid `provider_type` values. Callers use `filter_map` (list) or `match` (get/create/update).

### LOW: Missing `Display` implementations for some enums

`AgentStatus`, `RegistrationMode`, `UpdateStatus`, and `MqttServiceStatus` have `as_str()` but no `Display`. In
contrast, `Permission` and `AlertSeverity` do implement `Display`.

### ~~LOW: Missing OpenAPI schema derives on update hooks types~~ RESOLVED

**Resolution:** Added `#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]` to all 7 public types in
`update_hooks.rs`: `SystemdAction`, `DockerComposeAction`, `SystemdServiceHook`, `DockerComposeHook`,
`PredefinedHook`, `UpdateHookConfig`, `HooksConfig`. Also added the derive to `HookShell` in `uptrakit-shared-types`.

### ~~LOW: `HookValidationError` does not use `thiserror`~~ RESOLVED

**Resolution:** Converted from manual `impl Display` + `impl Error` to `#[derive(thiserror::Error)]` with
`#[error("{field}: {message}")]`.

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
| Input validation | ~~**HIGH**~~ FIXED | `Validate` trait + impls for 7 request types, wired into route handlers |
| Secret handling | ~~**HIGH**~~ FIXED | All secret fields now use `SecretString` |
| Wire dependency | PASS | `HookShell` now imported from `uptrakit-shared-types` directly |
| OpenAPI correctness | PASS | All hooks types now have OpenAPI schema derives |
| Serialization | GOOD | Correct attributes; minor consistency issues |
| Pagination | PASS | Well-implemented and well-tested |
| Permission model | FAIR | Very coarse (5 variants); many resources not covered |
| Test coverage | FAIR | 114 tests, but many modules have zero coverage |
| `unwrap`/`panic` | PASS | Zero in production code |
| Type safety | GOOD | Typed enums for query filters; `ProviderConfigResponse.provider_type` typed; `uses_remaining` now `u32` |
