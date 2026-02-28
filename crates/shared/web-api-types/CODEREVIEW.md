# Code Review — `crates/shared/web-api-types`

> Review date: 2026-02-28 | Reviewer: AI multi-agent review (6 specialist dimensions)
> Dimensions covered: Architecture · Security & Safety · Code Quality ·
> High Availability · Coding Standards · Extensibility

## Summary

`uptrakit-web-api-types` (~6,323 LOC) defines all HTTP API request/response types shared
between `uptrakit-web-api` (the server) and `uptrakit-openapi-client` (the generated client).
It provides a comprehensive set of typed pagination types, domain models, and a well-structured
`Validate` trait for input validation. The crate is a strong shared vocabulary layer. The
primary concerns are missing `Validate` implementations on several request types that accept
user input, and a cluster of public enums that could gain variants in future releases but
lack `#[non_exhaustive]` guards.

---

## What's Well-Implemented

- **[Security & Safety]** All credential-bearing response fields (OIDC tokens, device-flow
  codes, API token values) use `secrecy::SecretString`. The no-secrets-in-logs invariant is
  enforced at the type level; no credential can reach a log line through `Debug` formatting.

- **[Code Quality]** `Validate` trait is well-designed: structured `ValidationError { field,
  message }` allows callers to map errors to HTTP 422 responses with field-level context.
  Seven types implement it correctly, following a consistent pattern.

- **[Code Quality]** `PaginationParams`, `ResolvedPagination`, and `PaginatedResponse<T>` form
  a complete, reusable pagination abstraction. `ResolvedPagination::resolve()` clamps page and
  per-page to configured bounds, preventing unbounded queries from user input.

- **[Coding Standards]** All `FromStr` implementations pair with dedicated typed error types
  (`ParsePluginTypeError`, `ParseHookShellError`, etc.). No `FromStr` returns `String` as its
  error type, which is the correct Rust idiom and enables callers to match on specific failure
  cases.

- **[Extensibility]** The `openapi` feature gate means `utoipa` annotation machinery is
  compiled only when needed. Downstream crates that do not generate OpenAPI schemas (the CLI,
  test harnesses) pay no binary size cost.

---

## What Requires Attention

### Major

- **[Coding Standards]** `src/api_tokens.rs:8` — `CreateApiTokenRequest`, as well as
  `CreateMqttClientRequest`, `UpdateMqttClientRequest`, `CreateAutodiscoveryIgnoreRequest`,
  `UpdateOidcProviderRequest`, and `TriggerUpdateRequest` all accept user-controlled input but
  have no `Validate` implementation. Fields like token names and MQTT client identifiers can
  be submitted with arbitrary lengths or characters. Add `Validate` implementations capping
  lengths, requiring non-empty strings, or enforcing character sets where appropriate.

- **[Extensibility]** `src/permissions.rs:9` — `Permission` enum has 9 variants and will grow
  as new features are added, but lacks `#[non_exhaustive]`. Adding this attribute now is a
  non-breaking change; deferring forces all downstream exhaustive matches to be updated
  simultaneously when a new variant is added. The same applies to `AlertSeverity`,
  `TriggerUpdateStatus`, `UpdateStatus`, `RegistrationMode`, `SystemdAction`,
  `DockerComposeAction`, and `PredefinedHook`.

### Minor

- **[Code Quality]** `src/software_discovery_state.rs:23` and related files — `SoftwareDiscoveryState`,
  `DeviceAuthStatus`, `ServiceStatus`, `OutputStreamType`, and `MqttClientConnectionStatus`
  are shared across the wire protocol boundary and could plausibly gain new variants in future
  releases. Mark all five with `#[non_exhaustive]` to allow additive extension without
  breaking downstream exhaustive matches.

- **[Coding Standards]** `ServiceStatus` mapping from wire protocol status codes to this
  crate's `ServiceStatus` enum is duplicated in at least three locations across `mqtt`,
  `agent`, and `web-api`. Centralise the conversion as a `From<WireStatus> for ServiceStatus`
  implementation in this crate to eliminate the duplication.

### Observations

- **[Architecture]** `crates/shared/web-api-types` omits `publish = false`. All other internal
  crates mark themselves as non-publishable; this one does not. If this is intentional (it may
  be intended for external consumption as a client type library), document the rationale in
  `Cargo.toml` with a comment. If it is an oversight, add `publish = false`.

- **[Extensibility]** API version is embedded in route paths (`/api/v1/`) but not in response
  types or envelope headers. Future breaking changes to response shapes will require either a
  new route version or a migration period. Consider adding an `X-API-Version` header response
  field or a version negotiation mechanism before v2 is needed.
