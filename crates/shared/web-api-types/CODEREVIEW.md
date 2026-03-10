# Code Review: uptrakit-web-api-types

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-web-api-types` (~6,323 LoC) defines all HTTP API request/response types shared between
`uptrakit-web-api` (the server) and `uptrakit-openapi-client` (the generated client). It provides a
comprehensive set of typed pagination types, domain models, and a well-structured `Validate` trait
for input validation. The crate is a strong shared vocabulary layer. `Validate` implementations now cover all HTTP
request types including the MQTT client request types.

## Architecture

### Strengths

- `src/` -- Clean separation of request/response types from server-side handler logic. This crate
  contains only types and validation -- no HTTP framework dependency in the default feature set.
- `Cargo.toml` -- `openapi` feature gate means `utoipa` annotation machinery is compiled only
  when needed. Downstream crates that do not generate OpenAPI schemas pay no binary size cost.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- All credential-bearing response fields (OIDC tokens, device-flow codes, API token values) use
  `secrecy::SecretString`. The no-secrets-in-logs invariant is enforced at the type level.
- `Validate` trait provides structured `ValidationError { field, message }` for input validation,
  enabling HTTP 422 responses with field-level context.

### Issues

No security issues found.

## Code Quality

### Strengths

- `Validate` trait is well-designed with consistent implementation pattern. Structured field-level
  error reporting via `ValidationError { field, message }`. Seven implementations follow the
  same pattern.
- `PaginationParams`, `ResolvedPagination`, and `PaginatedResponse<T>` form a complete, reusable
  pagination abstraction. `ResolvedPagination::resolve()` clamps page and per-page to configured
  bounds, preventing unbounded queries.
- All `FromStr` implementations pair with dedicated typed error types (`ParsePluginTypeError`,
  `ParseHookShellError`, etc.). No `FromStr` returns `String` as error type.

### Issues

No code quality issues found.

## High Availability

### Strengths

N/A -- Pure type definitions with no I/O, no async, no state.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- All `FromStr` implementations correctly pair with typed errors.
- `SecretString` used consistently for all credential fields.

### Issues

**[LOW]** `ServiceStatus` mapping from wire protocol status codes is duplicated in at least
three locations across `mqtt`, `agent`, and `web-api`. Centralize the conversion as a
`From<WireStatus> for ServiceStatus` implementation in this crate.

## Extensibility

### Strengths

- `openapi` feature gate ensures utoipa annotations compiled only when needed.
- Type-safe pagination abstraction is reusable across all endpoints.

### Issues

**[LOW]** API version is embedded in route paths (`/api/v1/`) but not in response types or
envelope headers. Future breaking changes to response shapes will require either a new route
version or a migration period. Consider adding an `X-API-Version` header response field.

## Tests

### Strengths

- `src/hosts.rs` -- Nine tests cover `HostSortField` `FromStr` (valid, invalid, case-sensitive),
  `SortOrder` `FromStr` (valid, invalid), and `ListHostsParams` serialisation edge cases.
- `src/settings_mqtt.rs` -- 13 tests cover `MqttUrlValidation` (scheme validation, host
  validation, port bounds, full valid URL), `UpdateMqttSettingsRequest` nullable-field
  semantics (all combinations of set/clear/omit for username, password, CA), and MQTT URL
  parsing against valid and invalid URLs.
- `src/settings_nats.rs` -- Nine tests cover `NatsSettings` serialisation, validation (valid
  URL, HTTP rejected, private IP rejected, missing host, not-a-URL), and nullable-field
  semantics for the NATS URL.
- `src/settings_combined.rs` -- One test verifies `CombinedSettingsResponse` round-trips.
- `src/oidc_providers.rs` -- Eight tests cover `CreateOidcProviderRequest::validate`
  (empty name, empty client ID, empty client secret, empty discovery URL, invalid URL,
  valid request, URL format enforcement).
- `src/update_batches.rs:205` -- 7 tests for batch request validation and response
  serialization.
- `src/auth.rs` -- 10 tests cover `RegisterRequest::validate` (valid input, email too long,
  no `@` sign, empty first name, password too short, password too long) and
  `LoginRequest::validate` (valid input, email too long, no `@` sign, empty password).
- All tests use synchronous `#[test]` (correct -- no async I/O in this crate).

### Issues

**[LOW]** `src/masked_url.rs` -- `MaskedUrl` has no tests for its `Debug`/`Display`
redaction behaviour. A regression that accidentally exposes embedded credentials in a URL
(e.g., `mqtt://user:password@host`) would go undetected.

## Consistency

### Strengths

N/A

### Issues

**[MEDIUM]** `src/update_batches.rs:83,121,148` -- Mixed integer types across batch-related
structs: `total_created: usize` in `BatchUpdateResponse`, `total_count: i32` in the DB
entity, and `completed_count: i64` in `BatchProgressEvent`. These represent the same
conceptual quantity (number of updates) but use three different integer types. Conversions
between them require explicit casts that can silently truncate on platforms where `usize`
differs from `i32`. Standardise on a single integer type (e.g., `i64` to match the DB
aggregate return type) across all batch-related structs.

---

## Review — 2026-03-10

### Summary

This review adds new findings across coding standards, maintainability, and code consistency on
2026-03-10. The `Validate` coverage gap for `UpdateHostRequest` and `InvokeExtensionActionRequest`
is the highest-severity new finding.

### Coding Standards

**[MEDIUM]** `src/hosts.rs:54` — `UpdateHostRequest` has no `Validate` implementation. The
route handler writes to the database without validating `friendly_name` length. An attacker
could supply an arbitrarily long string, causing a DB write rejection at the ORM boundary
(unhandled error) or silently truncating the value. Recommendation: implement `Validate` for
`UpdateHostRequest` enforcing a maximum byte length on `friendly_name` consistent with the DB
column definition.

**[MEDIUM]** `src/extensions.rs:33` — `InvokeExtensionActionRequest` has no `Validate`
implementation. The `params: serde_json::Value` field is unbounded; `sensitive_params:
Option<SecretString>` is also unbounded. A malicious caller could send a multi-megabyte JSON
payload that passes deserialization but exhausts memory when the extension handler processes
it. Recommendation: add `Validate` checking the maximum serialized byte length of `params`
(align with `MAX_EXTENSION_PARAMS_LEN` from `uptrakit-internal-wire`'s `limits.rs`).

**[LOW]** `src/agents.rs` — `MergeAgentRequest` has no `Validate` implementation. The request
contains only a strongly-typed `Uuid` field so there is no injection risk, but the absence
breaks the workspace-wide pattern that all HTTP request types implement `Validate`. Add a
trivial `impl Validate` returning `Ok(())` for consistency.

### Maintainability

**[MEDIUM]** `src/settings_ca.rs` — This file is a 1-line stub: `pub use super::agents::MessageResponse as RotateCaResponse;`. The misleading module name implies CA-settings request/response types but the file contains only a type alias re-export. Recommendation: remove the file and move the re-export to `src/lib.rs` directly.

**[MEDIUM]** The ten `settings_*.rs` files have inconsistent granularity: `src/settings_auth.rs`
is ~40 LOC with 3 structs, `src/settings_ca.rs` is 1 LOC, while `src/settings_mqtt.rs` is
~506 LOC. The smallest settings modules create file-navigation noise without providing
modularity benefit. Recommendation: group the smallest modules (`settings_ca.rs`,
`settings_auth.rs`, `settings_zeroconf.rs` if present) under a `settings/` sub-directory or
consolidate into a single `settings_misc.rs`.

### Code and Logic Consistency

**[LOW]** `src/admin_events.rs` (or equivalent SSE event types) — `AdminEvent::ServiceStatusChanged
{ status: String }`, `AdminEvent::UpdateCompleted { status: String }`, and
`AdminEvent::SystemServiceStatusChanged { status: String }` use plain `String` for typed status
values rather than the appropriate typed enums (`ServiceStatus`, `UpdateStatus`). This prevents
the compiler from catching invalid status strings at the serialization boundary and requires
callers to perform their own string-to-enum conversion. Recommendation: replace `String` fields
with the appropriate typed enums.

### Strengths (2026-03-10)

- `SecretString` used consistently for all sensitive fields in HTTP API request/response types.
  Confirmed correct — no credential field uses a bare `String`.
- `Validate` trait implementations confirmed for all previously identified request types:
  `RegisterRequest`, `LoginRequest`, `CreateOidcProviderRequest`, `UpdateScheduledTaskRequest`,
  `UpdateNetworkSettingsRequest`, `CreateSoftwareItemRequest`, `CreatePluginConfigRequest`,
  `CreateApiTokenRequest`, `CreateEnrollmentTokenRequest`, `UpdateServiceRequest`,
  `CreateAutodiscoveryIgnoreRequest`, and all MQTT client request types.
