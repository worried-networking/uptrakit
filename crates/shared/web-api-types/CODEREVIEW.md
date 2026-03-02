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

**[LOW]** `Cargo.toml` -- `publish = false` is omitted, unlike all other internal crates which
mark themselves as non-publishable. If intentional (intended for external consumption as a
client type library), document the rationale. If an oversight, add `publish = false`.

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

**[HIGH]** `src/update_batches.rs:30-43` -- `HostBatchUpdateRequest::validate()` hardcodes the
category list `["security", "bugfix", "feature", "unknown"]`. This duplicates the variant set
from the `UpdateCategory` enum in `uptrakit-shared-types`. When a new category is added to the
enum, the validation will reject it until the hardcoded list is manually updated. Use
`UpdateCategory::iter()` (via `strum::IntoEnumIterator`) or validate by parsing through
`UpdateCategory::from_str()` instead.

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
- All tests use synchronous `#[test]` (correct -- no async I/O in this crate).

### Issues

**[MEDIUM]** `src/validation.rs` -- The `Validate` trait and its implementations for
`RegisterRequest`, `LoginRequest`, `UpdateScheduledTaskRequest`, `UpdateNetworkSettingsRequest`,
`CreateSoftwareItemRequest`, `CreatePluginConfigRequest`, `CreateApiTokenRequest`,
`CreateEnrollmentTokenRequest`, `UpdateServiceRequest`, and `CreateAutodiscoveryIgnoreRequest`
have no tests. Validation logic is the primary security boundary for all HTTP endpoints; a
regression in any `validate()` implementation (e.g., accepting an empty field that should be
rejected) would not be caught before deployment.

**[HIGH]** `src/notifications.rs:336` -- `ALL_EVENT_TYPES` lists 6 of 8 variants, missing
`BatchUpdateCompleted` and `BatchUpdatePartiallyCompleted`. Three tests iterate this array
to verify exhaustive coverage of notification event types. Those tests now silently pass
while leaving two event types untested. Any bug in the handling of the missing variants
will not be caught by the test suite.

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
