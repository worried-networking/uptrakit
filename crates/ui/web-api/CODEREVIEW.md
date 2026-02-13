# Code Review: Web-API Crate

**Crate:** `crates/ui/web-api/`
**Date:** 2026-02-13
**Scope:** 64 source files, ~24,600 lines

---

## Architecture

**Rating: Excellent**

- Clean layered architecture: middleware stack -> route handlers -> service
  layer -> DB
- `AppState` with 22 fields is large but well-documented; each field serves a
  distinct purpose with an inline doc comment
- Proper middleware ordering: `request_log` -> `resolve_ip` -> `rate_limit` ->
  `resolve_proxy_headers` -> (auth on protected routes)
- Separate PKI router (`build_pki_router`) without proxy header resolution
  (correct for plain HTTP OCSP)
- OpenAPI spec generation via `utoipa` with proper security scheme
- Dual router design: authenticated routes vs public routes with clear boundary
  (`lib.rs:430-515` authenticated, `lib.rs:516-570` public)

## Security and Safety

**Rating: Excellent**

- **Authentication**: Dual-path auth (JWT stateless + API token DB lookup) with
  denylist
- **Rate limiting**: DB-backed sliding window with local fallback (fail-closed
  for auth endpoints). Approved raw SQL exception in `auth/rate_limit.rs` --
  fully parameterised, atomic upsert.
- **Password hashing**: Argon2id with OWASP parameters (19 MiB, 2 iterations) at
  `auth/password.rs:31`
- **Refresh tokens**: SHA-256 hashed in DB, HttpOnly/Secure/SameSite=Strict
  cookies, rotation on use
- **Token denylist**: In-memory per-instance with JTI + user-level revocation
- **Device flow**: 32-byte crypto random device codes, consonant-only user codes
  (prevents offensive words)
- **CSRF protection**: SameSite=Strict cookies, Bearer token auth
- **Input validation**: Password length limits (8-1024), proper UUID parsing, URL
  validation
- **WebSocket rate limiting**: Per-connection message rate limiter + per-IP
  connection rate limiter
- **CA key store**: Debug impl redacts all key material; `CaPublicSnapshot`
  (`lib.rs:56`) verified to exclude private keys -- `CaKeyStore` (`lib.rs:81`)
  is a separate, non-Clone struct
- **MQTT credentials**: Credential-bearing MQTT messages
  (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are delivered
  locally only and never written to the outbox table
  (`notification_service.rs:42-51`) -- prevents plaintext credential persistence
  in the DB

## High Availability

**Rating: Good**

- **Rate limiting**: DB-backed (`api_rate_limits` table) with atomic upsert --
  shared across all instances. Local in-memory fallback (`FALLBACK_LIMITS` at
  `middleware/rate_limit.rs:106`) activates only when DB is unreachable;
  per-instance only but fail-closed prevents bypass.
- **Cross-controller notifications**: Outbox pattern via `controller_events`
  table. `EventPoller` on each instance polls every 1s
  (`event_poller.rs:59`) for events from other controllers (filtered by
  `source_controller_id != self` at `event_poller.rs:107`). Delivery retries
  with `MAX_DELIVERY_RETRIES = 3` (`event_poller.rs:19`). Event cleanup after
  24h (`event_poller.rs:21`).
- **Service connection registry**: Per-instance `ServiceConnectionRegistry`
  tracks locally connected WebSocket services. Cross-instance delivery happens
  via the event poller reading the outbox.
- **OIDC/Device flows**: All state persisted to DB -- HA-safe. A device flow
  initiated on instance A can be polled/approved on instance B.
- **Settings sync**: Version-counter-based invalidation (`settings_version`
  table) polled every 30s. Changes on one instance are visible to others within
  30s.
- **MQTT lease coordination**: Uses DB transactions (`TransactionTrait`) for
  lease assignment (`mqtt_lease_coordinator.rs:93`, `mqtt_lease_coordinator.rs:488`)
  -- safe for concurrent access from multiple instances.

## Code Quality

**Rating: Excellent**

- Extensive test coverage: 34 files contain tests
- Error types properly defined per boundary (`AuthError`, `ServiceWsError`,
  `AgentWsError`, etc.)
- Consistent use of `error_response()` helper for JSON error responses
- `AuthFailure` enum avoids `clippy::result_large_err` on the auth middleware
  hot path

## Coding Standards Compliance

**All Passed**

- No `#[allow()]` anywhere
- No `unsafe` anywhere
- No `panic!()` in production code (only in test code: `ocsp.rs:509`,
  `auth/rate_limit.rs:252`)
- Production `unwrap()`/`.expect()` only on safe patterns:
  - `routes/agent_ws.rs:50` -- `LazyLock` with `.expect()` on hardcoded semver
    constant `"0.0.1"`
  - `settings.rs:53` -- `.parse().unwrap()` on hardcoded `"[::]:8443"`
  - `settings.rs:218` -- `.expect("valid default HTTPS addr")` on same constant
  - `ocsp.rs:260` -- `dt.year() as u16` (safe for current dates; see finding
    below)
  - `ocsp.rs:267` -- `.expect("valid datetime")` on `OffsetDateTime::now_utc()`
    components (always valid for current dates)
  - `auth/refresh_cookie.rs:19,26` -- `.unwrap_or_else()` with safe empty
    fallback
  - `middleware/rate_limit.rs:115` --
    `Mutex::lock().unwrap_or_else(|poisoned| poisoned.into_inner())` (approved
    pattern, even safer than plain `.unwrap()`)
  - `middleware/rate_limit.rs:142` -- `.unwrap_or(now)` safe fallback
- Raw SQL only in approved rate limiter (fully parameterised)
- Error handling consistently uses `thiserror` + `rootcause`

## HA-Specific Findings

**0 Critical, 0 High, 2 Medium, 1 Low**

### MEDIUM: Token denylist is in-memory per-instance

**Location:** `auth/token_denylist.rs`

The code documents this: *"Cross-instance revocation relies on the natural JWT
expiry (15 min)"*. When a user logs out on instance A, their JWT remains valid
on instance B for up to 15 minutes. For deployments requiring immediate
cross-instance revocation, a DB-backed denylist or shared cache would be needed.

### MEDIUM: `rotate_refresh_token()` lacks transaction wrapping

**Location:** `auth/session.rs`

In a multi-instance scenario, if two instances process a rotation request for the
same token simultaneously, both could read the same session. The hash-based
lookup and sequential update make collision unlikely but not impossible.

Other multi-step mutations in the codebase properly use
`TransactionTrait::begin()`:

- User registration: `routes/auth.rs:76`
- OIDC callback: `routes/oidc_auth.rs:416`
- OIDC registration: `routes/oidc_auth.rs:789`
- Service merge: `routes/services.rs:481`
- Software items: `routes/software_items.rs:287`
- MQTT lease coordination: `mqtt_lease_coordinator.rs:93`,
  `mqtt_lease_coordinator.rs:488`

### LOW: Local fallback rate limiter is per-instance

**Location:** `middleware/rate_limit.rs`

When the DB is unavailable, the in-memory `FALLBACK_LIMITS` map is not shared
across instances. During a DB outage, an attacker distributing requests across N
instances gets N x the rate limit. Mitigated by the fail-closed mode (only
applies when DB is unreachable, which is rare).

## Additional Findings (Non-HA)

**0 Critical, 0 High, 1 Medium, 3 Low**

### MEDIUM: `ocsp.rs:260` -- year cast `dt.year() as u16`

Could silently truncate for dates far in the future (year > 65535). Practically
safe for OCSP responses in the 2020s-2030s, but a defensive check or
`try_into()` would be more robust.

```rust
der::DateTime::new(
    dt.year() as u16,
    dt.month() as u8,
    // ...
)
.expect("valid datetime"),
```

### LOW: `AppState` could benefit from sub-grouping

The `AppState` struct has 22 fields. While each is necessary and
well-documented, grouping related fields into sub-structs (e.g., `AuthState`,
`PkiState`) would improve readability.

### LOW: Test boilerplate is duplicated across modules

Test boilerplate for creating `AppState` is duplicated across multiple test
modules (`lib.rs`, `require_auth.rs`, `resolve_ip.rs`, `services.rs`, etc.). A
shared test fixture module could reduce this.

### LOW: `AuthMethod` fallback could log a warning

**Location:** `auth/session.rs:86`, `auth/session.rs:132`

`AuthMethod::from_session(...).unwrap_or(AuthMethod::Password)` silently falls
back to `Password` if the stored auth method string is unrecognised. This is
safe but could mask data corruption; a `tracing::warn!` on fallback would aid
debugging.
