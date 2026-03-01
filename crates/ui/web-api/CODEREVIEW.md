# Code Review: uptrakit-web-api

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-web-api` is the largest crate (~32K LoC, ~98 `.rs` files) implementing the full
HTTP/WebSocket API server: 80+ REST endpoints, WebSocket lifecycle management, OIDC authentication
flows, JWT/session management, PKI operations, MQTT lease coordination, cross-controller event
delivery, and database query helpers. It is a library crate consumed by `uptrakit-controller`.

The crate demonstrates strong security primitives (Argon2id, JWT denylist, typed permission
extractors, PKCE for OIDC), a well-designed `TenantDb` abstraction for tenant isolation, and
clean `AppState` builder pattern. Key concerns: the crate's overall size approaching "god crate" territory, route
handlers in `src/routes/` lacking inline unit tests for the majority of business-logic paths,
and several query modules without test coverage. The OIDC registration DB error masking
(`unwrap_or(1)`), `count_linked_hosts` DB error swallowing, `Report::new()` macro violations,
invalid UUID query parameter handling, HTTP status code violations on soft-delete and
idempotent-create endpoints, the `require_auth.rs` permission-fetch fallback, the TOCTOU race
in `create_or_ignore_ignore_rule`, the empty output returned by `list_update_history` for
records using the `update_output_lines` storage path, the rate-limit test DB backdating,
the `find_raw_active_config` `.ok().flatten()` DB error swallowing, the
`notification_service.rs` 50 ms timeout tests missing `start_paused`, and the
`broadcast_server_restarting_scattered` untracked `tokio::spawn` tasks
have all been fixed. The `generate_secure_token()` UUID fallback (OIDC auth now returns
HTTP 500 on RNG failure instead of silently downgrading entropy), and the `lookup_by_secret`
bearer-secret query without a `service_id` filter (now narrowed via optional URL query
parameter for defence-in-depth during the enrollment window) have been fixed. The
in-memory-only token denylist (revocations now persisted to `revoked_token_jtis`/
`revoked_token_users` DB tables and propagated to peer instances via
`ControllerMessage::TokenRevoked` over NATS), and the OIDC registration code exposed in
redirect URL query parameters (now transmitted via URL hash fragment) have been fixed.

## Architecture

### Strengths

- `src/app_state.rs:114-387` -- Builder pattern for `AppState` with exhaustive field checks
  catches missing configuration at startup. `AppStateBuildError` names the first missing field.
  `plugin_ops` field defaults to real `PluginRegistry`, making test injection a one-call
  override.
- `src/lib.rs:52-156` -- `CaPublicSnapshot` / `CaKeyStore` split isolates private key material.
  `CaKeyStore` is not `Clone`, `Debug` redacts all key fields to `[REDACTED]`.
- `src/tenant_db.rs:14-103` -- `TenantDb` extractor combining database access with tenant
  scoping provides type-safe tenant-filtered queries (`find`, `find_by_id`, `update_many`,
  `delete_many`, `find_via_tenant_join`).
- `src/queries/mod.rs:1-15` -- Clean separation between routes (HTTP concerns) and queries
  (database concerns).
- `src/cert_signer.rs:1-41` -- `AgentCertSigner` trait abstracts certificate signing, enabling
  `NoopCertSigner` test doubles.
- `src/settings.rs:73-114` -- `tokio::sync::watch` for lock-free reads with write mutex for
  serialized updates. `SettingsSnapshot` provides atomic reads.
- `src/router.rs:295-527` -- Router with separate authenticated and public route groups,
  middleware layering (resolve_ip -> rate_limit -> resolve_proxy_headers).
- `src/lib.rs:773-989` -- Dual-router design: `build_router` (HTTPS, full middleware stack)
  and `build_pki_router` (plain HTTP, PKI endpoints only) are clearly separated. The PKI
  router intentionally omits `resolve_proxy_headers` with explanatory comment.
- Middleware layering order correct: `resolve_proxy_headers` -> `rate_limit_auth` ->
  `resolve_ip` -> `request_log`. Proxy header stripping before rate limiting prevents
  header-spoofed rate limit bypass.
- OIDC feature-gating is complete: types, flow stores, and routes absent when `oidc` disabled.
  OIDC state stores as separate per-concern types (`OidcFlowStore`, `OidcRegistrationStore`,
  `OidcTokenExchangeStore`, `AccountLinkStore`), each scoped to one flow step.
- `PluginOps` trait decouples route handlers from concrete `PluginRegistry`.
- `src/event_poller.rs:88` -- Event poller cursor starts at `max_id - 100` to avoid missing
  events between startup and first poll. Cursor only advances past successfully delivered or
  permanently skipped events.
- UUID v7 primary keys throughout entity definitions for time-ordered inserts.
- `TenantScoped` trait provides compile-time tenant filtering. Transactions used for all
  multi-step mutations (`oidc_callback`, `oidc_complete_registration`, `enroll_*`,
  `merge_service`). `lock_exclusive()` before mutation in `merge_service`.
- Soft-delete partial unique indexes (`WHERE deactivated_at IS NULL`) prevent duplicate names
  among active records without conflicting with soft-deleted rows.
- `INSERT ON CONFLICT DO NOTHING` for lease deduplication in MQTT coordinator.
- Batch plugin config loading in `list_ignore_rules` and JOIN-based `load_plugins` eliminate
  N+1 patterns.

### Issues

**[HIGH]** `src/lib.rs:1-24` -- At ~32K LoC, this crate contains auth, middleware, routes,
queries, settings, MQTT coordination, NATS transport, OCSP, PKI, notifications, and update
output broadcasting. Consider extracting auth, settings, and MQTT coordination into shared
crates.

**[HIGH]** `src/app_state.rs:37-96` -- `AppState` has 22+ public fields. Most have `pub`
visibility. PKI, notification, and credential fields should have restricted visibility with
accessor methods.

**[MEDIUM]** `src/router.rs:17-209` -- OpenAPI `#[openapi(...)]` lists every path and schema
explicitly (~200 lines). Adding a new endpoint requires modifying three places: route module,
router function, and OpenAPI annotation.

**[LOW]** `api_tokens` table has no `expires_at` column. API tokens valid indefinitely once
issued. A compromised token that was never explicitly revoked remains valid forever.

## Security and Safety

### Strengths

- `src/auth/password.rs:32-40` -- Argon2id with OWASP parameters (19 MiB, 2 iterations).
- `src/auth/token_denylist.rs` -- JTI-level and user-level revocation with `iat` cutoff.
  Five unit tests cover boundary conditions including purge semantics and "latest timestamp
  wins".
- `src/auth/session.rs:109-183` -- Refresh token rotation in DB transaction with replay
  detection.
- `src/auth/rate_limit.rs` -- DB-backed rate limiter, fail-closed. Applied to all auth
  endpoints and WebSocket connection/auth-failure paths.
- `src/middleware/resolve_proxy_headers.rs:56-62` -- Cert and forwarded headers stripped from
  untrusted peer IPs. Certificate issuer CN verified against known CAs.
- `src/auth/token.rs:17-22` -- All tokens stored as SHA-256 hashes, never plaintext.
- `src/middleware/permission.rs:35-110` -- `permission_extractor!` macro generates typed
  extractors for all 9 permission levels.
- `src/auth/refresh_cookie.rs` -- HttpOnly, Secure, SameSite=Strict, path-scoped.
- `src/routes/oidc_auth.rs:168,308` -- PKCE enforced on all OIDC flows. Single-use CSRF state
  with 10-minute TTL. Email verification enforced. PKCE verifiers encrypted at rest.
- `src/routes/service_ws/connection.rs:31` -- 1 MB max WebSocket message size. 30-second
  anonymous timeout. Per-IP connection rate limiting (30/60s). Auth failure rate limiting
  (10/300s). Sequence number validation. Per-connection message rate limiting (50/s).
- Zero `unsafe` blocks.
- `SecretString` at all API input boundaries.

### Issues

**[LOW]** `src/auth/token.rs:17-22` -- API tokens hashed with unsalted SHA-256. Per-token salt
would strengthen defense-in-depth.

**[LOW]** `src/middleware/resolve_proxy_headers.rs:256-258` -- CA CN comparison uses
non-constant-time string equality. The compared values (CA CNs) are not confidential, making
exploitability very low.

## Code Quality

### Strengths

- `src/routes/service_ws/protocol.rs` -- Two-pass `deserialize_service_msg`: extract sequence,
  validate, then full deserialize. Unknown message types handled gracefully.
- `src/routes/service_ws/protocol.rs` -- `MessageRateLimiter` with injected clock for testing.
- `src/lib.rs:98-112` -- `CaKeyStore` Debug manually redacts all key fields, verified by test
  at `src/lib.rs:1284-1303`.
- Uniform `bail!` / `report!` / `context_to` error propagation throughout.
- Zero `#[allow(dead_code)]` or `#[allow(unused)]` annotations.
- Named constants for all timing values: `WS_MESSAGE_RATE_LIMIT`, `MAX_WS_MESSAGE_SIZE`,
  `ANONYMOUS_TIMEOUT`, `APPROVAL_POLL_INTERVAL`, `MAX_UPDATE_OUTPUT_BYTES`.
- `src/mqtt_lease_coordinator.rs:687-712` -- `model_to_config` isolation; conversion from DB
  model to wire type in single private function.
- `src/middleware/permission.rs:116-209` -- `permission_extractor!` macro fully tested. Six
  tests cover: missing auth extension -> 401, correct permission -> pass, wrong permission ->
  403, no permissions -> 403, multiple permissions with one match -> pass, `new()` bypass.
- `MessageRateLimiter` unit-tested with clock injection — no real wall-clock sleep.
- `deserialize_service_msg` three-path test coverage: unknown message ->
  `Ok(Some(ServiceMessage::Unknown))`, malformed JSON -> `Err`, sequence mismatch ->
  `Err(SequenceValidation)`.
- `record_service_activity` DB tests with in-memory SQLite verify IP update and last-seen-at.
- `MqttLeaseCoordinator` well-covered — four integration tests using in-memory SQLite.
- `EventPoller` cursor behavior tested — safety margin and stale-event-skip verified.
- `TokenDenylist` comprehensively tested — five tests including purge and boundary conditions.
- `base_url_from_headers` unit tests — three cases: Origin preferred, Host fallback, missing
  both.
- Router integration tests — Tower `oneshot` tests verify healthz, CA cert, 404, ConnectInfo
  injection, trusted proxy IP.
- Security-sensitive paths tested: JWT wrong secret, denylist revocation, OIDC state one-time-
  use, device-flow consume, session double-approve, rate-limit window reset.
- Rate limiter test suite covers seven distinct scenarios including window expiry and key
  isolation.
- `is_mqtt_tenant_message` test comprehensively covers credential-bearing variant filtering.
- Lease coordinator tests cover all three outcome branches with DB verification.

### Issues

**[HIGH]** Route handlers in `src/routes/` have no inline unit tests for the majority of
business-logic paths. The following route files have zero `#[cfg(test)]` coverage: `hosts.rs`,
`agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`, `oidc_providers.rs`, `server_cert.rs`,
`settings_auth.rs`, `ocsp.rs`. Given the complexity of handlers like `oidc_callback`
(413 lines), the absence of tests for individual sub-flows means regressions are only caught
at integration level.

**[MEDIUM]** `src/routes/auth.rs:458` and `src/middleware/require_auth.rs:202` --
`test_state(db)` / `NoopCertSigner` construction duplicated across at minimum these two modules
and `src/lib.rs:1032`. A shared `test_helpers` module would eliminate duplication.

**[MEDIUM]** `src/mqtt_lease_coordinator.rs:724` and 16 other modules -- `test_db()` /
`setup_test_db()` helper duplicated across 17+ modules. Shared `test_helpers` module would
reduce duplication.

**[MEDIUM]** `src/queries/` -- Several query modules lack unit tests:
`scheduled_tasks.rs`, `services.rs`, `autodiscovery.rs`, `plugin_configs.rs`, and
`update_history.rs` have no inline tests.

**[MEDIUM]** `oidc_callback` has no unit tests despite 413 lines and 7 code paths. All seven
`OidcUserResolution` branches are untested at the unit level.

**[LOW]** `src/notification_service.rs:46-63` -- `msg.clone()` on every `send()` and
`broadcast()` call. Could compute serialized JSON first.

**[LOW]** `src/queries/plugin_configs.rs:151-152` -- `unreachable!()` in `unwrap_or_else`
creates a hidden panic path.

**[LOW]** `src/auth/authentication.rs` -- `resolve_oidc_user` (most complex function in auth
module with 7 return paths) has no tests. Orphaned-link fallthrough, `LinkViaOidcRequired`
detection, and deactivated-user short-circuit are entirely untested.

**[LOW]** `src/notification_service.rs:261-271` -- `server_restarting_is_local_only` test
asserts only enum construction, not behavioral intent. Should test that `svc.broadcast(msg)`
does NOT write to the outbox.

## High Availability

### Strengths

- `src/service_connections.rs:211-221` -- `send()` acquires read lock, clones sender, drops lock
  before async send. No lock held across await.
- `src/service_connections.rs:125-133` -- Connection deduplication: re-registering same
  service_id cancels old connection via `CloseReason::Superseded`.
- `src/update_output_broadcaster.rs:18` -- Bounded broadcast channels (256 capacity) with
  graceful lag handling.
- `src/notification_service.rs:166-179` -- Credential-bearing messages filtered from NATS
  outbox, preventing plaintext credential persistence.
- `src/auth/token_denylist.rs:78-91` -- Token denylist handles monotonic revocation with
  `iat_cutoff` advancement.
- Settings use `watch` channel for atomic snapshot publishing with serialized writes. Dual
  version counters enable efficient cross-instance invalidation polling.
- `src/event_poller.rs:102-183` -- Event poller advances cursor only past successfully
  delivered events. After `MAX_DELIVERY_RETRIES` (3) failures the event is permanently skipped.
- `src/mqtt_lease_coordinator.rs:141-161` -- `INSERT ON CONFLICT DO NOTHING` makes concurrent
  assignment idempotent.
- Enrolled-loop approval polling decoupled from ping frequency via dedicated
  `APPROVAL_POLL_INTERVAL` (5 seconds).
- Cancellation token propagated to WebSocket connection loops via `cancel_token.cancelled()`.
- `broadcast_server_restarting_scattered` spreads notifications over jitter window.

### Issues

**[MEDIUM]** `src/update_output_broadcaster.rs:80-96` -- `send_line` holds write lock for the
entire operation. A read lock with per-entry interior mutability would allow concurrent sends to
different updates.

**[LOW]** `src/routes/service_ws/handler/mod.rs` -- First approval-poll tick consumed with
`.await`, blocking the enrolled loop for 5 seconds before entering the main `select!`.

**[LOW]** `src/notification_service.rs:107-177` -- Backlog delivery replays up to 500 events
sequentially with no timeout.

## Coding Standards

### Strengths

- `SecretString` at all API input boundaries. Typed permission extractors consistently applied.
- No `Result<T, String>` anti-pattern. No `StatusCode` numeric literals.
- `FromStr` with typed errors used correctly.
- Zero `#[allow(clippy::...)]` suppressions.
- Edition 2024 consistent with workspace standard.
- 70+ endpoints protected via typed permission extractors. Handler signatures form an auditable
  access-control inventory.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `PluginOps` trait in `AppState` decouples routes from `PluginRegistry`. Adding a new plugin
  requires zero changes to web-api code.
- OIDC feature-gate compiles out the entire OIDC subsystem. `swagger-ui` and DB backend features
  similarly gated.
- `ServiceConnectionRegistry` type-erased dispatch works uniformly across agent types.
- Event poller's `deliver_event` is forward-compatible with unknown capabilities.
- Sequence validation decoupled from full deserialization enables forward-compatible message
  handling.

### Issues

**[MEDIUM]** `src/routes/oidc_auth.rs:873` -- `fake_claims` reverse-role-mapping ignores nested
claim paths. If `role_claim_path = "realm.roles"`, reconstruction places values at `realm` not
`realm.roles`.

**[LOW]** `src/routes/oidc_auth.rs` -- No mechanism to add custom OIDC scopes beyond the
`scopes` column. No documented path for operators to add custom claims processors without code
changes.
