# Code Review: uptrakit-web-api

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-web-api` is the largest crate (~38K LoC, ~105 `.rs` files) implementing the full
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

Updated on 2026-03-02 with findings for the batch updates feature
(`routes/update_batches.rs`, `queries/update_batches.rs`, `batch_progress_broadcaster.rs`),
the refactored update trigger pipeline (`queries/update_triggers.rs`), and SSE batch progress
streaming. The N+1 query patterns in `find_outdated_items_for_host` and
`find_outdated_hosts_for_item` have been fixed via batch-loading. The Telegram webhook secret
comparison now uses SHA-256 + constant-time `ct_eq` to eliminate the timing side-channel.
The non-atomic batch completion race (`maybe_complete_batch` three separate COUNT queries + UPDATE
outside a transaction) has been fixed by wrapping the entire function in a DB transaction with a
terminal-state guard. The SSE batch progress and update output streams now integrate
`CancellationToken` and exit cleanly during graceful server shutdown. The tenant isolation
bypass in `load_host_agents` (`ServiceHost::find()` without join through `service::Entity`)
has been fixed via `tenant_db.find_via_tenant_join::<service_host::Entity, service::Entity>(...)`
with a regression test (`load_host_agents_filters_by_tenant`). The same bypass in
`validate_update_preconditions` and `trigger_all_host_package_updates_for_host` (service lookup
missing `.filter(service::Column::TenantId.eq(tenant_id))`) has been fixed. The
`AuthCleanupExecutor` now wraps all DELETE statements in a single transaction.

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

**[HIGH]** `src/lib.rs:1-24` -- At ~38K LoC, this crate contains auth, middleware, routes,
queries, settings, MQTT coordination, NATS transport, OCSP, PKI, notifications, and update
output broadcasting. Consider extracting auth, settings, and MQTT coordination into shared
crates.

**[HIGH]** `src/app_state.rs:37-96` -- `AppState` has 26 public fields. Most have `pub`
visibility. PKI, notification, and credential fields should have restricted visibility with
accessor methods.

**[MEDIUM]** `src/router.rs:17-209` -- OpenAPI `#[openapi(...)]` lists every path and schema
explicitly (~200 lines). Adding a new endpoint requires modifying three places: route module,
router function, and OpenAPI annotation.

**[MEDIUM]** `src/batch_progress_broadcaster.rs` -- Instance-local HashMap. Multi-instance
deployments will not deliver SSE events cross-instance.

**[MEDIUM]** `src/queries/update_batches.rs:563-579` -- `list_batches` loads ALL child
update_history records to compute status counts. A GROUP BY aggregation would be more
efficient.

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

**[MEDIUM]** `src/routes/update_batches.rs:355,365` -- `.unwrap_or_default()` silently
swallows DB errors loading host/item names for SSE replay.

**[LOW]** `src/routes/update_batches.rs:64-118,142-200` -- `trigger_host_batch_update` and
`trigger_item_batch_update` share near-identical second half.

**[LOW]** `src/queries/plugin_configs.rs:151-152` -- `unreachable!()` in `unwrap_or_else`
creates a hidden panic path.

**[INFO]** `src/routes/service_ws/handler/updates.rs:534-537` -- Wildcard `_` arm on
`UpdateFinalStatus` maps to `Failed` without `tracing::warn!`.

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

**[HIGH]** `src/batch_progress_broadcaster.rs:64-66` -- Instance-local only. SSE clients on
instance A get no live events for batches processed on instance B.

**[MEDIUM]** `src/batch_progress_broadcaster.rs:77-79` -- Orphaned channel leak if batch
never reaches terminal state.

**[MEDIUM]** `src/nats_transport.rs:162-164` -- Fixed 1-second NATS retry without jitter or
backoff.

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

**[MEDIUM]** `src/routes/update_batches.rs:93,176` -- `batch_type` as bare string literal. No
typed constant. Typo would silently create novel batch type.

**[MEDIUM]** `src/queries/update_triggers.rs:91-94` -- `actor_type` as raw `&str` with no
compile-time enforcement.

**[MEDIUM]** `src/routes/oidc_auth.rs:873` -- `fake_claims` reverse-role-mapping ignores nested
claim paths. If `role_claim_path = "realm.roles"`, reconstruction places values at `realm` not
`realm.roles`.

**[LOW]** `src/routes/oidc_auth.rs` -- No mechanism to add custom OIDC scopes beyond the
`scopes` column. No documented path for operators to add custom claims processors without code
changes.

## Tests

### Strengths

- `src/auth/jwt.rs:105-231` -- Six tests covering encode/decode round-trip, wrong secret
  rejection, legacy tokens lacking `aud`/`iss` fields, OIDC provider ID embedding, and
  deterministic key construction. Directly validates the security boundary that rejects
  cross-deployment token reuse.
- `src/auth/token_denylist.rs` -- Five tests including purge semantics, monotonic `iat_cutoff`
  advancement, and boundary-condition coverage. Already noted in the Code Quality section.
- `src/auth/device_flow.rs:258-429` -- Ten tests with an in-memory SQLite DB covering create,
  pending status, approve (with code normalisation), double-approve, consume one-time-use,
  consume-while-pending, not-found, cleanup, expiry detection, and user-code format invariants.
  Both success and failure paths tested.
- `src/auth/rate_limit.rs` -- Seven rate-limiter tests with a real SQLite DB covering window
  expiry and key isolation. No `start_paused = true` used, confirmed by the explicit comment at
  line 293 explaining the correct choice.
- `src/auth/oidc_state.rs:476-` -- OIDC flow stores (insert/take idempotency, expiry,
  single-use) tested with in-memory SQLite.
- `src/auth/authentication.rs:367-598` -- Nine tests for `navigate_json_path` and
  `extract_mapped_roles`: nested paths, missing intermediates, empty path, array/string claim
  values, and unmapped claims. Pure logic; no DB or tokio runtime needed.
- `src/middleware/permission.rs` -- Six tests for the `permission_extractor!` macro covering
  missing auth, correct permission, wrong permission, no permissions, multi-permission match,
  and the `new()` bypass constructor.
- `src/middleware/require_auth.rs:184-` -- Tests exercise the `require_auth` middleware end-
  to-end via Tower `oneshot`: valid JWT accepted, missing token → 401, revoked JTI → 401, OIDC
  JWT with missing provider ID rejected.
- `src/routes/services.rs` -- Two integration tests for `merge_service`: connected-target
  conflict returns 409; valid merge deactivates source and transfers identity. Handler called
  directly, DB verified after.
- `src/routes/service_ws/protocol.rs` -- Three-path `deserialize_service_msg` test and clock-
  injected `MessageRateLimiter` tests — no wall-clock sleep.
- `src/mqtt_lease_coordinator.rs` -- Four integration tests with in-memory SQLite covering all
  three `LeaseOutcome` branches plus DB state verification.
- `src/event_delivery.rs:218-316` -- Tests for `parse_capability_str` (all known values plus
  unknown) and three `deliver_event` routing scenarios: broadcast, service-targeted-not-
  connected, and capability-targeted.
- `src/routes/auth.rs` -- `logout_revokes_own_token` and `logout_rejects_other_user_token`
  test cross-user token revocation. Both call the handler directly with a wired-up DB.
- `src/routes/oidc_auth.rs:1229-1258` -- Three unit tests for `base_url_from_headers`: Origin
  preferred over Host, Host fallback, and both-missing returns `None`.
- `src/routes/server_cert.rs:209-301` -- Five tests for `build_server_tls_config`: valid cert
  bundle, invalid cert PEM, invalid key PEM, invalid CA PEM, and multiple CAs in bundle.
- No `start_paused = true` appears anywhere in the crate — confirmed by full-codebase grep.
  All time-sensitive tests use either DB backdating helpers (`expire_flow`) or injected clocks
  (`MessageRateLimiter`), consistent with the project rules.
- `src/notification_service.rs:174-256` -- Four tests: local-only delivery, credential-bearing
  variants blocked from NATS publication, `send` returns `false` for unconnected service,
  `broadcast` with empty registry does not panic.

### Issues

**[CRITICAL]** `src/routes/update_batches.rs` (532 lines) -- Zero test coverage. All 5 batch
route handlers untested.

**[CRITICAL]** `src/queries/update_batches.rs` (699 lines) -- Zero test coverage. Entire
batch query layer untested.

**[CRITICAL]** `src/queries/update_triggers.rs` (436 lines) -- Zero test coverage. Refactored
update trigger pipeline untested.

**[CRITICAL]** `src/routes/oidc_auth.rs:221-625` -- `oidc_callback` is 404 lines with at
least 10 distinct code paths (provider-error redirect, missing-params redirect, state-expired,
state-DB-error, provider-gone, base-URL-missing, PKCE-build-failure, code-exchange-failure,
invite-mode pre-check with 3 sub-branches, and 7 `OidcUserResolution` match arms). The only
test in this file covers the 4-line helper `base_url_from_headers`. No test exercises any
branch of the main callback dispatch — `LinkedUser` session creation, `NewUser` first-user
owner-role promotion, `LinkViaPasswordRequired` pending-link storage, `Deactivated` redirect,
`NotAllowed` redirect, or `EmailNotVerified` redirect. A regression in any branch is invisible
until a live OIDC login attempt.

**[HIGH]** `src/notifications/dispatcher.rs` (306 lines) -- `dispatch_loop`, `merge_smtp_into_config`,
and the SMTP-merge notification path have zero tests. The `merge_smtp_into_config` function
contains conditional field insertion with a hardcoded default port (`unwrap_or(587)`) at
line 71 that is entirely untested. The dispatch loop itself — rule matching, scope filtering,
channel config decryption, SMTP merge, and delivery — runs inside a `tokio::spawn` with no
observable output, making it impossible to verify correct end-to-end notification delivery
without at least one integration test.

**[HIGH]** `src/routes/software_items.rs` (1264 lines, no `#[cfg(test)]`) -- The largest
route file in the crate has no inline tests. It contains `trigger_update`, `trigger_version_check`,
`assign_hosts`, and `update_software_item` — each with multi-step DB mutations, plugin dispatch,
and cross-service messaging. The error-mapping helper `query_error_to_response` and the
`TriggerUpdateCommandExecutor` async-trait implementation are also untested.

**[HIGH]** `src/routes/notifications.rs` (686 lines, no `#[cfg(test)]`) -- `test_channel` (the
"send a test notification" endpoint) calls `merge_smtp_into_config_pub` then constructs a
`DeliveryMessage` and dispatches it. No test verifies that the SMTP merge is applied before
delivery, that an unsupported channel type is rejected with 400, or that a DB error during
channel load returns 500. This is a user-visible correctness risk for the notification feature.

**[HIGH]** `src/routes/service_ws/handler/mod.rs` (810 lines) and
`src/routes/service_ws/handler/updates.rs` (926 lines) -- The enrolled loop, authenticated
loop, and all update-lifecycle handlers (`handle_update_started`, `handle_update_output`,
`handle_update_result`, `deliver_pending_updates`) have no tests. These functions contain
multiple branches: output truncation at `MAX_UPDATE_OUTPUT_BYTES`, sequence validation,
ownership validation across service-host links, and `UpdateFinalStatus` mapping. The WebSocket
handler files (`connection.rs`, `handler/messages.rs`, `handler/mqtt.rs`, `handler/discovery.rs`,
`handler/renewal.rs`) are also all untested.

**[HIGH]** `src/notifications/events.rs` -- Tests don't cover `BatchUpdateCompleted` or
`BatchUpdatePartiallyCompleted` event variants.

**[HIGH]** `src/notifications/message_builder.rs` -- Tests don't cover batch notification
message templates.

**[MEDIUM]** `src/queries/autodiscovery.rs` (1846 lines, no `#[cfg(test)]`) -- The largest
query file in the crate with no tests. Functions `process_discovery_results`,
`create_default_plugin_configs_for_target`, and `discard_pending_items` contain complex
multi-step DB mutations that are difficult to verify without an in-memory SQLite fixture.
`process_discovery_results` in particular has nested loops and conditional config creation
that warrants at minimum a happy-path DB test.

**[MEDIUM]** `src/queries/notifications.rs` (463 lines), `src/queries/enrollment_tokens.rs`
(194 lines), `src/queries/services.rs` (396 lines), `src/queries/plugin_configs.rs`
(286 lines), `src/queries/scheduled_tasks.rs` (165 lines) -- All have zero inline tests.

**[MEDIUM]** `src/routes/settings_mqtt.rs:58-79` -- `resolve_connection_status` calls
`OffsetDateTime::now_utc()` directly to decide whether a heartbeat is stale. The function
is pure (takes `model` and `heartbeat_at`) but has no tests. Three distinct paths —
disabled-client short-circuit, no-heartbeat returns offline, stale-heartbeat returns offline,
fresh-heartbeat returns persisted status — are all unexercised. The function should accept an
injected `now: OffsetDateTime` parameter and be tested with fixed timestamps, following the
same clock-injection pattern already used in `MessageRateLimiter`.

**[MEDIUM]** `src/routes/auth.rs`, `src/middleware/require_auth.rs`, `src/lib.rs`,
`src/routes/services.rs`, `src/middleware/resolve_ip.rs`, `src/auth/device_flow.rs`,
`src/auth/session.rs`, `src/auth/rate_limit.rs`, `src/mqtt_lease_coordinator.rs`,
`src/queries/hosts.rs`, `src/auth/oidc_state.rs` -- Eleven modules each define their own
`test_db()` / `setup_test_db()` and/or `NoopCertSigner`. The full-AppState construction
(`test_state`) is duplicated verbatim in at least five modules (`src/routes/auth.rs:484`,
`src/middleware/require_auth.rs:207`, `src/lib.rs:74`, `src/routes/services.rs:456`,
`src/middleware/resolve_ip.rs:138`). A single `src/test_helpers.rs` module (feature-gated
`#[cfg(test)]`) would eliminate ~300 lines of copy-paste, make future `AppState` field additions
a one-place change, and prevent test-only infrastructure drift between modules.

**[MEDIUM]** `src/routes/device_auth.rs` (219 lines, no `#[cfg(test)]`) -- `device_auth_poll`
contains a three-branch status match where the `Authorized` branch itself has two nested DB
operations (consume flow, create API token). The one-time-use semantics of `consume` are
already tested in `auth/device_flow.rs`, but the route-level integration — mapping
`DeviceFlowError::NotFound` to 404, `AlreadyAuthorized` to 409, the successful API token
creation and response serialization — is entirely untested.

**[LOW]** `src/auth/authentication.rs` -- `resolve_oidc_user` (the 7-branch resolution
function documented in the Code Quality section) is tested only via `extract_mapped_roles` and
`navigate_json_path` helpers that it calls. The resolution branches themselves — orphaned-link
fallthrough, `LinkViaOidcRequired` detection, `Deactivated` short-circuit, and `NewUser`
auto-creation with default role assignment — require DB-backed tests. None exist.

**[LOW]** `src/notification_service.rs:180-188` -- `server_restarting_is_local_only` asserts
only that the message variant exists (`matches!`). The documented design intent is that
`ServiceConnectionRegistry::broadcast_server_restarting_scattered()` must be called instead of
`NotificationService::broadcast()`. The test should assert that `svc.broadcast(ServerRestarting)` —
if called — does NOT write to the NATS outbox; currently it only confirms the enum arm compiles.

## Consistency

### Strengths

- All list endpoints whose collections can grow unboundedly (`/hosts`, `/services`,
  `/plugin-configs`, `/enrollment-tokens`, `/notifications/channels`, `/notifications/rules`,
  `/update-history`, `/software-items`, `/autodiscovery/ignores`) uniformly return
  `PaginatedResponse<T>` with `page`/`per_page` query parameters. The `PaginatedResponse`
  wrapper and `PaginationParams` extractor are consistently re-exported from each route module,
  ensuring a uniform API shape across 10+ list endpoints.
- All create endpoints that introduce a new durable resource return HTTP 201: `hosts` (via
  enrollment), `services` (via enrollment), `api_tokens.rs:51`, `enrollment_tokens.rs:96`,
  `notifications.rs:69` (channel), `notifications.rs:368` (rule), `plugin_configs.rs:89`,
  `oidc_providers.rs:109`, `autodiscovery.rs:79` (new rule). HTTP 200 is used only for
  idempotent creates (the "already exists" path in `create_autodiscovery_ignore`).
- Soft-delete operations are consistently implemented with `deactivated_at` (not `deleted_at`)
  across every entity: hosts, services, plugin_configs, software_items, autodiscovery items,
  notification channels, and OIDC providers. The partial unique index pattern
  (`WHERE deactivated_at IS NULL`) is uniform. See `src/queries/hosts.rs:245`,
  `src/queries/services.rs:199`, `src/queries/plugin_configs.rs:281`.
- Every route file uses `crate::error_response::error_response(StatusCode::X, "message")` as
  the sole mechanism for error responses. There is no route handler that constructs a manual
  `Json(ErrorBody {...})` or uses a different error helper. This uniformity makes it safe to
  change the error wire format in one place.
- All protected endpoints carry `extensions(("x-required-permission" = json!("...")))` in their
  `#[utoipa::path]` annotation. Cross-referencing against the `permission_extractor!` macro
  exhausts the list without gaps. The permission strings match the typed extractor names
  (`CanViewHosts` → `"view_hosts"`, etc.).

### Issues

**[MEDIUM]** `src/routes/oidc_providers.rs:126` and `src/routes/settings_mqtt.rs:87`
(vs all other list endpoints) -- `list_providers` returns `Vec<OidcProviderResponse>` (flat
array, no pagination) and `list_mqtt_settings` returns `Vec<MqttClientResponse>` (flat array),
while every other collection endpoint (`/hosts`, `/services`, `/plugin-configs`,
`/enrollment-tokens`, `/notifications/channels`, `/notifications/rules`, `/software-items`,
`/update-history`, `/autodiscovery/ignores`) returns `PaginatedResponse<T>`. The OIDC provider
list is tenant-scoped and realistically bounded, but the inconsistency is visible in the OpenAPI
schema and forces client-side code to handle two response shapes for lists. Preferred pattern:
wrap in `PaginatedResponse<T>` for uniformity.

**[MEDIUM]** `src/routes/settings_mqtt.rs:408,454,619,640` (vs `src/routes/hosts.rs:84`,
`src/routes/services.rs:87`, `src/routes/enrollment_tokens.rs:167`) -- All four 404 responses
in `settings_mqtt.rs` use the bare message `"Not found"` instead of a descriptive entity-aware
message like `"MQTT client not found"`. Every other route file uses entity-specific 404
messages (`"Host not found"`, `"Service not found"`, `"Enrollment token not found"`). The bare
message makes client-side error disambiguation harder when multiple request parameters could
independently cause a 404. Preferred pattern: `"MQTT client not found"` to match the rest of
the API.

**[MEDIUM]** `src/routes/scheduler.rs` -- All four endpoints diverge from established patterns:
`"Failed to ..."` error messages (not `"Internal server error"`), flat `Vec<T>` without
pagination, bare `Json(resp)` without explicit `StatusCode::OK`.

**[MEDIUM]** `src/routes/update_batches.rs:355-365` -- SSE replay `.unwrap_or_default()`
consistent with `update_history` SSE but both silently swallow DB errors.

## Database

### Strengths

- `src/tenant_db.rs:31-78` -- `TenantDb::find`, `find_by_id`, `update_many`, `delete_many`,
  and `find_via_tenant_join` provide a complete, type-safe CRUD surface that never exposes the
  raw `DatabaseConnection` to route handlers. The five methods cover all standard ORM operations;
  no route handler needs to bypass the tenant guard to perform standard CRUD.
- `src/queries/update_history.rs:82-210` -- `list_update_history` executes five well-chosen bulk
  queries and assembles results entirely in application memory, with zero per-record round-trips:
  one for records, one for host names, one for software item names, one for output lines for the
  streamed-output subset, and one subquery filter. The `INTO SUBQUERY` pattern at line 90-97
  avoids loading all host IDs for the tenant into Rust memory before applying the filter.
- `src/queries/hosts.rs:109-152` -- `list_hosts` batch-loads service-host links and service
  records in two additional queries and assembles the `agents` array in a HashMap-join, not a
  per-host loop. This is the correct pattern for one-to-many associations on paginated lists.
- `src/queries/services.rs:213-256` -- `deactivate_service` executes all three mutations
  (soft-delete service, revoke certificates, bump revocation version) inside a single explicit
  transaction. Certificate revocation cannot be left in a partial state relative to the service's
  `deactivated_at`.
- `src/queries/services.rs:264-395` -- `merge_service` uses `lock_exclusive()` on both the
  source and target rows before the merge, preventing a concurrent approval of the same source
  from racing with the merge. The service host link copy uses `ON CONFLICT DO NOTHING`,
  making the link transfer idempotent.
- `src/queries/mqtt_software_states.rs:39-191` -- `load_software_states_for_tenant` executes
  exactly four bulk queries (items, host-software-item links, active hosts, active updates) with
  no N+1 patterns and uses a `FromQueryResult` projection to avoid loading unreferenced columns
  from the large `host_software_items` row.
- `src/queries/update_triggers.rs:140-293` -- `trigger_update_for_host` performs all seven
  prerequisite checks (item active, host active, host assigned, agent exists, agent approved,
  no active update, execute plugin assigned) before writing, so the `update_history` insert
  only occurs after all validation has passed. No partial state is written on validation failure.
- `src/queries/plugin_configs.rs:121-135` -- `find_raw_active_config_txn` accepts
  `&impl ConnectionTrait` rather than `&TenantDb`, enabling reuse inside transactions. This is
  the correct abstraction for shared query helpers that need to participate in the caller's
  transaction.

### Issues

**[MEDIUM]** `src/queries/scheduled_tasks.rs:155-162` -- `trigger_scheduled_task` performs a
read-then-write in two separate statements without a transaction. The initial
`find_by_id::<scheduled_task::Entity, _>(id).one(tenant_db.db())` existence check at line
145-152 confirms tenant ownership. But the subsequent `scheduled_task::Entity::update_many()`
at line 156 filters only by `Column::Id.eq(id)` — it has no `tenant_id` guard. If the task is
deleted between the check and the update (TOCTOU), `rows_affected` will be 0 and the function
returns `Ok(false)`. More critically, the `update_many` does not re-apply the tenant filter,
so a race where a different tenant's task ID collides (unlikely with UUID v7 but theoretically
possible) would update an out-of-scope row. Adding
`.filter(scheduled_task::Column::TenantId.eq(tenant_db.tenant_id))` to the `update_many`
call would make the update self-contained and correct under all concurrent conditions.

**[MEDIUM]** `src/queries/notifications.rs:247-283` -- `update_rule` cannot clear the
`host_id`, `software_item_id`, or `plugin_type` scope filters once set. Lines 271-278 only
apply updates when `req.host_id.is_some()` / `req.software_item_id.is_some()` /
`req.plugin_type.is_some()`. This means a user who scoped a rule to a specific host cannot
un-scope it to apply to all hosts without deleting and recreating the rule. The project's
documented pattern for nullable update fields is `Option<serde_json::Value>` where
`Value::Null` signals "clear this field" and `None` signals "leave unchanged". The current
`Option<Uuid>` / `Option<String>` types cannot express this three-way distinction.

**[MEDIUM]** `src/queries/discovery_allowlist.rs:97-127` and lines 216-250 --
`add_tenant_allowlist_entry` and `add_host_allowlist_entry` perform a read-then-insert without
a transaction (select existing, early return, then insert). Two concurrent requests to add the
same `plugin_type` to the same allowlist can both pass the existence check and both proceed to
insert, creating a duplicate entry. Neither the entity definition nor the migration creates a
unique constraint on `(tenant_id, plugin_type)` / `(tenant_id, host_id, plugin_type)`, so the
DB cannot prevent the race. The result is duplicate entries visible in `list_*_allowlist`
responses. The idempotency comment in the function doc is incorrect: the function is only
idempotent if calls are serialized. Fix: wrap in a transaction and handle the unique constraint
violation, or (if a unique constraint is added) use `INSERT OR IGNORE` / `ON CONFLICT DO NOTHING`.

**[LOW]** `src/queries/update_history.rs:213-255` -- `get_update_history` performs three
sequential awaited queries: `UpdateHistory::find_by_id`, `tenant_db.find_by_id::<host::Entity>`,
and `SoftwareItem::find_by_id`. The first two are necessary (the second enforces tenant scope),
but the third (`SoftwareItem::find_by_id` for the name) could be eliminated by storing the
software item name directly on the `update_history` record at creation time, or by joining the
host and software item tables in a single query. As written, a single detail-view request
issues three sequential round-trips to the DB.

**[LOW]** `src/queries/enrollment_tokens.rs:44-68` -- `create_enrollment_token` uses
`expect("capability list should serialize to JSON")` on the `serde_json::to_string` result
at line 50. While the comment is accurate (a `Vec<String>` is always JSON-serializable), this
is a hidden panic in a production query path. Use `map_err` / `?` to propagate this as a
`DbErr::Custom`, consistent with all other serialization error handling in the query layer.

**[MEDIUM]** Missing index on `update_history(host_id, software_item_id, status)` for
`validate_update_preconditions`.

**[LOW]** `src/queries/update_batches.rs:617-651` -- Host/software item lookups in
`get_batch_with_items` lack tenant filter (defense-in-depth).

**[LOW]** `src/queries/notifications.rs:323-332` -- `find_log_by_action_token` takes a raw
`&sea_orm::DatabaseConnection` rather than a `TenantDb`. The `notification_log` table has a
`tenant_id` column and is `TenantScoped`. This function bypasses the tenant filter entirely,
meaning the action-token lookup during notification acknowledgment is not scoped to the
requesting tenant. A malicious user who guesses a valid `action_token` UUID from another
tenant's notification log would be able to retrieve and act on it. Pass `&TenantDb` and use
`tenant_db.find()` to enforce scoping.

## Maintainability

### Strengths

- `src/settings.rs` -- `Settings` and `SettingsSnapshot` are well-documented with field-level
  doc comments explaining the watch-channel architecture. The `SETTINGS_KEYS` constant gives
  readers a single list of all persisted setting keys.
- `src/middleware/` -- Every middleware file has a module-level doc comment explaining what it
  does and why. The permission macro is fully documented with examples.
- `src/queries/` directory separates database concerns from HTTP routing. Query functions have
  consistent naming (`find_*`, `list_*`, `create_*`) and most have doc comments.
- `src/app_state.rs` -- All 26 `AppState` fields carry inline doc comments explaining their
  purpose, which partially compensates for the wide public surface.

### Issues

**[HIGH]** `src/queries/autodiscovery.rs` -- 1,848 lines in single query module.

**[HIGH]** `src/queries/software_items.rs` -- 1,376 lines mixing CRUD, assignment, and
version-check.

**[HIGH]** `src/routes/oidc_auth.rs:221-634` -- `oidc_callback` is 413 lines with 7 distinct
code paths and no inline doc comments explaining the `OidcUserResolution` state machine. A
reader navigating the function encounters `OidcUserResolution::ExistingUser`,
`NewUserRegistration`, `LinkedExistingUser`, `LinkedNewUser`, `AccountLinkRequired`, etc.,
without any comment explaining the resolution logic or the conditions that lead to each branch.
The seven branches are untested (noted elsewhere) and also undocumented. At minimum, a
function-level doc comment describing the state machine would allow future contributors to
modify the OIDC flow correctly.

**[HIGH]** `src/middleware/tenant_context.rs:16` -- A `TODO` comment marks multi-tenancy
implementation as future work: "TODO: When multi-tenancy is enabled, re-add X-Tenant-Id header
processing". This represents a hard-coded architectural constraint that is not surfaced in any
issue tracker reference or timeline. The TODO should reference a tracking issue and note what
the current single-tenant assumption means for the codebase (all 70+ endpoints implicitly
operate on `default_tenant_id`).

**[MEDIUM]** `src/queries/autodiscovery.rs:1-1846` -- The largest query file at 1,846 lines
contains six distinct query functions across two concerns: ignore-rule management (lines 1-170)
and discovery-result processing (lines 171-1846). The discovery processing half includes
several large private helpers (`find_or_create_software_items`, `process_discovery_results`,
`assign_plugin_configs`) that could be extracted into a `src/queries/autodiscovery/` submodule.
The mixed concerns make it harder to audit changes to either concern in isolation.

**[MEDIUM]** `src/routes/update_batches.rs:93,176` -- Batch type strings should be constants
or enum.

**[MEDIUM]** `src/queries/update_batches.rs` -- 699 lines with zero unit tests.

**[MEDIUM]** `src/router.rs` -- 684 lines. OpenAPI path registration extremely verbose.

**[MEDIUM]** `src/routes/service_ws/handler/updates.rs` -- 925 lines. Update lifecycle in
single file.

**[MEDIUM]** `src/app_state.rs:37-100` -- `AppState` has 26 public fields. Fields such as
`crl_pem_cache`, `ca_rotation_trigger`, `rustls_config`, and `revocation_notify` are
infrastructure concerns that route handlers should not access directly. Exposing them as `pub`
fields means any future handler author can use them without restriction. Accessor methods
(`fn crl_pem(&self) -> String`, `fn trigger_ca_rotation(&self)`) with appropriate visibility
would express the intended usage contract.

**[LOW]** `src/router.rs:17-209` -- The OpenAPI `#[openapi(...)]` macro lists every path and
schema explicitly in ~200 lines. Every new endpoint requires three synchronized edits: the
route handler file, the router function, and the OpenAPI annotation list. A missed update in
the OpenAPI list silently omits the endpoint from generated docs without a compile error.

**[LOW]** `src/settings_store.rs:17` -- `JWT_KEY_LENGTH: usize = 64` is a magic constant with
no doc comment explaining that 64 bytes corresponds to a 512-bit HMAC key. The constant is used
only in `generate_or_load_jwt_key`, which is the correct location, but without a comment a
reader cannot verify whether 64 is bytes or hex characters.
