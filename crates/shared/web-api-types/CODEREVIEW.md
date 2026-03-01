# Code Review: uptrakit-web-api-types

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
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
- All tests use synchronous `#[test]` (correct -- no async I/O in this crate).

### Issues

**[MEDIUM]** `src/validation.rs` -- The `Validate` trait and its implementations for
`RegisterRequest`, `LoginRequest`, `UpdateScheduledTaskRequest`, `UpdateNetworkSettingsRequest`,
`CreateSoftwareItemRequest`, `CreatePluginConfigRequest`, `CreateApiTokenRequest`,
`CreateEnrollmentTokenRequest`, `UpdateServiceRequest`, and `CreateAutodiscoveryIgnoreRequest`
have no tests. Validation logic is the primary security boundary for all HTTP endpoints; a
regression in any `validate()` implementation (e.g., accepting an empty field that should be
rejected) would not be caught before deployment.

**[LOW]** `src/masked_url.rs` -- `MaskedUrl` has no tests for its `Debug`/`Display`
redaction behaviour. A regression that accidentally exposes embedded credentials in a URL
(e.g., `mqtt://user:password@host`) would go undetected.
