# Code Review: uptrakit-web-api-types

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-web-api-types` (~6,323 LoC) defines all HTTP API request/response types shared between
`uptrakit-web-api` (the server) and `uptrakit-openapi-client` (the generated client). It provides a
comprehensive set of typed pagination types, domain models, and a well-structured `Validate` trait
for input validation. The crate is a strong shared vocabulary layer. The primary concerns are
missing `Validate` implementations on several request types and public enums lacking
`#[non_exhaustive]`.

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

**[MEDIUM]** `src/api_tokens.rs:8` -- `CreateApiTokenRequest`, `CreateMqttClientRequest`,
`UpdateMqttClientRequest`, `CreateAutodiscoveryIgnoreRequest`, `UpdateOidcProviderRequest`, and
`TriggerUpdateRequest` all accept user-controlled input but have no `Validate` implementation.
Fields like token names and MQTT client identifiers can be submitted with arbitrary lengths or
characters. Add `Validate` implementations.

**[LOW]** `ServiceStatus` mapping from wire protocol status codes is duplicated in at least
three locations across `mqtt`, `agent`, and `web-api`. Centralize the conversion as a
`From<WireStatus> for ServiceStatus` implementation in this crate.

## Extensibility

### Strengths

- `openapi` feature gate ensures utoipa annotations compiled only when needed.
- Type-safe pagination abstraction is reusable across all endpoints.

### Issues

**[MEDIUM]** `src/permissions.rs:9` -- `Permission` enum has 9 variants and will grow as new
features are added, but lacks `#[non_exhaustive]`. Adding this attribute now is non-breaking;
deferring forces all downstream exhaustive matches to be updated simultaneously. The same
applies to `AlertSeverity`, `TriggerUpdateStatus`, `UpdateStatus`, `RegistrationMode`,
`SystemdAction`, `DockerComposeAction`, and `PredefinedHook`.

**[LOW]** `src/software_discovery_state.rs:23` and related files -- `SoftwareDiscoveryState`,
`DeviceAuthStatus`, `ServiceStatus`, `OutputStreamType`, and `MqttClientConnectionStatus`
could plausibly gain new variants in future releases. Mark all five with `#[non_exhaustive]`.

**[LOW]** API version is embedded in route paths (`/api/v1/`) but not in response types or
envelope headers. Future breaking changes to response shapes will require either a new route
version or a migration period. Consider adding an `X-API-Version` header response field.
