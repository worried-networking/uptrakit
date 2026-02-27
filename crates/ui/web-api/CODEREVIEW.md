# CODEREVIEW — uptrakit-web-api

## Summary

`uptrakit-web-api` is the largest and most complex crate in the workspace (~98 `.rs` source files, approximately 1.2 MB of Rust). It implements the full HTTP/WebSocket API server: route handlers (80+ REST endpoints), WebSocket lifecycle management for agents, MQTT services, and SSH agents, OIDC authentication flows, JWT/session management, PKI operations, MQTT lease coordination, cross-controller event delivery, and database query helpers. It is a library crate consumed exclusively by `uptrakit-controller`.

The crate demonstrates several architectural strengths: a well-designed builder pattern for `AppState`, strong security primitives throughout the auth subsystem, consistent use of typed permission extractors, and a clean `CaPublicSnapshot`/`CaKeyStore` split that keeps private key material isolated. The primary concerns are concentrated in two areas: a cluster of High-severity security gaps in the authentication flow (unverified OIDC email, missing JWT audience claim, in-memory-only token revocation), and code quality issues in the largest route handlers where complexity has grown well beyond maintainable bounds. The previously identified N+1 query patterns in `list_update_history`, `load_item_hosts`, and the autodiscovery ignore-list loop have been resolved using batch loading with subqueries and `HashMap` lookups.

---

## Architecture

### Strengths

- **`AppState` builder pattern** — `src/lib.rs:234-493`. `AppStateBuilder` enforces that all required fields are set before construction; `AppStateBuildError` names the first missing field. Partial state cannot escape at compile time. The `plugin_ops` field defaults to the real `PluginRegistry`, making test injection a one-call override.
- **`CaPublicSnapshot` / `CaKeyStore` split** — `src/lib.rs:52-156`. Public CA data is freely cloneable and shareable. Private key material is isolated in `CaKeyStore` (not `Clone`, not `Debug`), distributed only to the three consumers that legitimately need it (OCSP, CRL, cert signer). The `split_snapshot` function ensures consistent construction.
- **`CaKeyStore` `Debug` redaction** — `src/lib.rs:98-112`. Every key field prints `"[REDACTED]"`. Verified by a dedicated test at `src/lib.rs:1284-1303`.
- **Dual-router design** — `src/lib.rs:773-989`. `build_router` (HTTPS, full middleware stack) and `build_pki_router` (plain HTTP, PKI endpoints only) are clearly separated. The PKI router intentionally omits `resolve_proxy_headers` with an explanatory comment.
- **OIDC feature-gating** — `Cargo.toml:10-12` and throughout `src/lib.rs`. OIDC types, flow stores, and routes are completely absent from the binary when the `oidc` feature is disabled. Feature-conditioned fields in `AppState` and `AppStateBuilder` are consistently paired.
- **`PluginOps` abstraction** — `src/lib.rs:213-215`. Plugin operations injected as `Arc<dyn PluginOps>`, decoupling all route handlers from the concrete `PluginRegistry`. Enables mock injection in tests without a running plugin ecosystem.
- **Middleware layering order** — `src/lib.rs:947-960`. `resolve_proxy_headers` → `rate_limit_auth` → `resolve_ip` → `request_log`. Applied in reverse execution order as required by Axum/Tower. Proxy header stripping happens before rate limiting to prevent header-spoofed rate limit bypass.
- **OIDC state stores as separate per-concern types** — `OidcFlowStore`, `OidcRegistrationStore`, `OidcTokenExchangeStore`, `AccountLinkStore`. Each store is scoped to a single step in the OIDC flow, preventing cross-step state confusion.
- **`ServiceConnectionRegistry.send()` non-blocking** — `src/service_connections.rs`. Read lock acquired, sender cloned, lock dropped before the async send. No lock held across await points.
- **Event poller cursor safety** — `src/event_poller.rs:88`. Cursor starts at `max_id - 100` to avoid missing events between startup and first poll. Cursor only advances past events that are successfully delivered or permanently skipped, preventing silent message loss.

### Issues

**[SEVERITY: Low]** No `rust-version` MSRV set anywhere in the workspace

`AGENTS.md` documents `rust-version = "1.91"` but no crate declares it. If this crate uses edition 2024 features, build failures on older toolchains have no documented expectation.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/routes/settings_auth.rs:96-102` — Non-additive feature flag: `#[cfg(not(feature = "oidc"))]` provides a semantic guard

When OIDC is disabled, this block prevents disabling password auth. Refactor so the default path rejects and the positive `#[cfg(feature = "oidc")]` path conditionally allows.

**[SEVERITY: Low]** `src/middleware/rate_limit.rs:228-236` — Non-additive feature flag: `#[cfg(not(feature = "oidc"))]` in test expectations

Should use `let mut expected = vec![...]` and extend conditionally inside `#[cfg(feature = "oidc")]`.

**[SEVERITY: Low]** `src/lib.rs:941-944` — Non-additive feature flag: `#[cfg(not(feature = "swagger-ui"))]` for fallback OpenAPI JSON route

The plain JSON route should be registered unconditionally; Swagger UI should be an additive overlay.

---

## Security & Safety

### Strengths

- **Argon2id with OWASP parameters** — `src/auth/password.rs:32-40`. 19 MiB memory cost, 2 iterations. Directly matches OWASP recommended minimums for interactive authentication.
- **JWT denylist with two revocation modes** — `src/auth/token_denylist.rs`. JTI-level revocation for single-token logout; user-level revocation (`until` timestamp) for credential rotation/compromise scenarios. Five unit tests cover boundary conditions including purge semantics and "latest timestamp wins" for successive deny calls.
- **Refresh token rotation in DB transaction with replay protection** — `src/auth/session.rs:109-183`. Rotation is atomic; a replayed refresh token cannot produce two valid sessions.
- **DB-backed rate limiter, fail-closed** — `src/auth/rate_limit.rs`. `fail_closed: true` causes a DB error to reject rather than allow. Applied at two layers: HTTP auth endpoints (middleware) and WebSocket connection/auth-failure paths (`src/routes/service_ws/mod.rs`).
- **Reverse-proxy header spoofing mitigation** — `src/middleware/resolve_proxy_headers.rs:56-62`. Cert and forwarded headers stripped from untrusted peer IPs before being re-set from trusted proxy headers.
- **All tokens stored as SHA-256 hashes** — `src/auth/token.rs:17-22`. Plaintext tokens never persisted; a DB dump reveals no replayable credentials.
- **Typed permission extractors** — `src/middleware/permission.rs:35-110`. One struct per `Permission` variant, generated by `permission_extractor!` macro. Authorization is enforced at the type level; handler signatures form an auditable access-control inventory. Nine variants, all tested.
- **Refresh cookie hardening** — `src/auth/refresh_cookie.rs`. HttpOnly, Secure, SameSite=Strict, path-scoped to `/api/v1/auth`. Not accessible to JavaScript.
- **PKCE enforced on all OIDC flows** — `src/routes/oidc_auth.rs:168`, `308`. `pkce_verifier` stored with the pending flow record and consumed at callback; code injection into a stolen authorization URL is blocked.
- **Zero `unsafe` blocks** in production code across the entire crate.
- **`SecretString` at all API input boundaries** — OIDC tokens, device-flow codes, API token responses use `SecretString`; raw bytes are not retained after the response is serialized.

### Issues

**[SEVERITY: Medium]** `src/auth/token_denylist.rs:15` (acknowledged in code comment) — In-memory denylist provides no cross-instance revocation

The comment explicitly acknowledges the gap: "Cross-instance revocation relies on the natural JWT expiry (15 min)." For a system managing infrastructure access, a 15-minute window after revocation during which a stolen token remains valid on peer instances is a material risk. A DB-backed `revoked_tokens` table checked at validation time (or a Redis pub/sub invalidation) would close this gap.

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:363-374` — `unwrap_or(1)` during OIDC registration check silently swallows DB errors

```rust
.count(state.db())
.await
.unwrap_or(1)
> 0;
```

A DB outage returns `1` (treating the user as "existing"), which either blocks legitimate new registrations silently or skips the token-required path for a brand-new user. Should be propagated as an error with a redirect to `/login?error=oidc_internal_error`.

**[SEVERITY: Medium]** `src/middleware/require_auth.rs:114-116` — Permission fetch failure returns empty permissions

`get_user_permissions(...).await.unwrap_or_default()` on DB failure gives the authenticated user an empty permission set, causing every subsequent resource access to return 403 rather than 500. This masks DB connectivity problems as authorization denials and is difficult to distinguish from a legitimately unprivileged user.

**[SEVERITY: Low]** `src/middleware/rate_limit.rs:121` — `FALLBACK_LIMITS.lock().unwrap()` in `check_local_fallback`

Mutex poisoning (possible if a thread panics while holding the lock) would panic this function, permanently disabling the in-memory rate-limit fallback for the process lifetime. Use `.unwrap_or_else(|e| e.into_inner())` to recover from poisoning, or replace the `std::sync::Mutex` with `tokio::sync::Mutex` for consistency with the async context.

**[SEVERITY: Low]** Multiple sites in `src/routes/oidc_auth.rs` — `generate_secure_token()` fallback to UUID on failure

```rust
generate_secure_token().unwrap_or_else(|_| generate_uuid().to_string())
```

Found at lines 388, 535, 584, 1221. UUID v4 has 122 bits of entropy versus the intended 256 bits of `generate_secure_token`. A CSPRNG failure is extremely unlikely but should propagate as an error rather than silently downgrade to weaker randomness for security tokens (exchange codes, registration codes, link tokens).

#### 2026-02-24 Review

**[SEVERITY: Low]** `src/middleware/resolve_proxy_headers.rs:256-258` — CA CN comparison uses non-constant-time string equality

The `==` operator short-circuits, but the compared values (CA CNs) are not confidential, making exploitability very low.

---

## Code Quality

### Strengths

- **Two-pass `deserialize_service_msg`** — `src/routes/service_ws/protocol.rs`. Step 1 extracts only the sequence number (hard fail on malformed JSON). Step 2 validates sequence (hard fail on mismatch). Step 3 performs full deserialization (soft fail for unknown future message types). The three-phase contract is clearly documented and allows replay-protection to remain accurate even when the full payload cannot be parsed.
- **`MessageRateLimiter` with injected clock for testing** — `src/routes/service_ws/protocol.rs`. `set_window_start` is `#[cfg(test)]`-only; tests directly manipulate the window start to avoid real wall-clock sleeps.
- **Uniform error propagation** — `bail!`, `report!`, `context_to`, `impl_report_conversion!` used consistently throughout. No `Report::new()` anti-pattern, no `Result<T, String>`.
- **Zero `#[allow(dead_code)]` or `#[allow(unused)]` annotations** anywhere in the crate.
- **`WS_MESSAGE_RATE_LIMIT` / `WS_MESSAGE_RATE_WINDOW` named constants** — `src/routes/service_ws/protocol.rs`. WebSocket rate-limit parameters are named and documented, not magic numbers.
- **`MAX_WS_MESSAGE_SIZE` and `ANONYMOUS_TIMEOUT` named** — `src/routes/service_ws/connection.rs`. Domain-meaningful values with doc comments.
- **`APPROVAL_POLL_INTERVAL` named constant** — `src/routes/service_ws/handler/mod.rs`. Enrolled-loop poll interval is explicit and documented.
- **`MAX_UPDATE_OUTPUT_BYTES` named constant** — `src/routes/service_ws/handler/mod.rs`. Output cap is named and the cap-enforcement logic is well-documented.
- **`model_to_config` isolation in lease coordinator** — `src/mqtt_lease_coordinator.rs:687-712`. The conversion from DB model to wire type is a single private function, not repeated inline across all callers.

#### 2026-02-24 Review

- **`CaKeyStore` `Debug` redaction with dedicated test verification.** `src/lib.rs:98-112` — Manually redacts every key field to `"[REDACTED]"` with a verification test at `src/lib.rs:1284-1303`.

### Issues

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:66` and `src/routes/oidc_auth.rs:352` — `unwrap_or_default()` on DB queries silently returns empty results

`OidcProvider::find().all(...).await.unwrap_or_default()` at line 66 returns an empty provider list on DB error, causing `auth_methods` to report "no OIDC providers configured" rather than a server error. At line 352, `count(...).unwrap_or(1)` treats a DB error as "link exists", gating the user out of the OIDC flow. Both should log with `tracing::warn!` or propagate as errors.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/queries/plugin_configs.rs:155-158` — Duplicated `is_unique_name_violation` uses brittle string matching

Same `msg.contains("unique") || msg.contains("duplicate")` pattern as `autodiscovery.rs:673-676`. Should be consolidated into `uptrakit-shared-db` using backend-specific error codes.

**[SEVERITY: Medium]** `src/settings.rs:53` — `unwrap()` on socket address parsing in `Default` impl

`DEFAULT_HTTPS_ADDR.parse().unwrap()` is not an approved exception per AGENTS.md. Should use `expect()` with a reason or parse at compile time.

**[SEVERITY: Low]** `src/notification_service.rs:46-63` — `msg.clone()` on every `send()` and `broadcast()` call

The outbox write only needs serialized JSON, which could be computed first, avoiding a full message clone.

**[SEVERITY: Low]** `src/notification_service.rs:153` — `event.message_json.clone()` during backlog delivery

`from_value` takes ownership; since the event is consumed, destructuring would eliminate the clone.

**[SEVERITY: Low]** `src/queries/plugin_configs.rs:151-152` — `unreachable!()` in `unwrap_or_else` creates a hidden panic path

Relies on an invariant not enforced by the type system. Should return a proper error.

---

## Tests

### Strengths

- **`permission_extractor!` macro fully tested** — `src/middleware/permission.rs:116-209`. Six tests cover: missing auth extension → 401, correct permission → pass, wrong permission → 403, no permissions → 403, multiple permissions with one match → pass, and `new()` constructor bypass semantics.
- **`MessageRateLimiter` unit-tested with clock injection** — `src/routes/service_ws/protocol.rs` (tests module). No real wall-clock sleep; window start is directly manipulated via the `#[cfg(test)]` helper.
- **`deserialize_service_msg` three-path coverage** — `src/routes/service_ws/mod.rs` (tests module). Tests cover unknown message type → `Ok(None)`, malformed JSON → `Err`, sequence mismatch → `Err(SequenceValidation)`.
- **`record_service_activity` DB tests** — `src/routes/service_ws/protocol.rs` (tests module). In-memory SQLite verifies IP update and last-seen-at semantics for both `Some(ip)` and `None` cases.
- **`MqttLeaseCoordinator` well-covered** — `src/mqtt_lease_coordinator.rs:714-905`. Four integration tests using in-memory SQLite: new client leased, no local service, already leased, batch assignment skips already-leased clients.
- **`EventPoller` cursor behavior tested** — `src/event_poller.rs:384-427`. Safety margin test verifies cursor = max_id - 100; stale-event-skip test verifies events created before service connect time are skipped without delivery.
- **`TokenDenylist` comprehensively tested** — `src/auth/token_denylist.rs:104-179`. Five tests including purge semantics, boundary conditions for `iat == until`, and "latest timestamp wins" semantics for successive deny calls.
- **`base_url_from_headers` unit tests** — `src/routes/oidc_auth.rs:1261-1289`. Three cases: Origin preferred over Host, Host fallback, missing both returns None.
- **Router integration tests** — `src/lib.rs:1140-1281`. Tower `oneshot` tests verify healthz, CA cert response, 404 handling, `ConnectInfo<SocketAddr>` injection for both main and PKI routers, and trusted proxy IP resolution.
- **Security-sensitive paths tested** — JWT wrong secret, denylist revocation, OIDC state one-time-use, device-flow consume, session double-approve, rate-limit window reset.

#### 2026-02-24 Review

- **`is_mqtt_tenant_message` test comprehensively covers credential-bearing variant filtering.** `src/notification_service.rs:273-314` — Exercises all three credential-bearing variants.
- **Backlog delivery test exercises both positive and negative filtering.** `src/notification_service.rs:316-399` — Validates eligible messages are delivered and ineligible types are filtered.
- **`skips_non_matching_capability_backlog` verifies cross-capability filtering.** `src/notification_service.rs:401-440` — Confirms SQL condition correctly filters by capability.
- **Lease coordinator tests cover all three outcome branches with DB verification.** `src/mqtt_lease_coordinator.rs:782-890`.
- **Rate limiter test suite covers seven distinct scenarios.** `src/auth/rate_limit.rs:174-362` — Including window expiry and key isolation.

### Issues

**[SEVERITY: High]** Route handlers in `src/routes/` have no inline unit tests for the majority of business-logic paths

The following route files have zero `#[cfg(test)]` coverage: `hosts.rs`, `agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`, `oidc_providers.rs`, `server_cert.rs`, `settings_auth.rs`, `ocsp.rs`. Given the complexity of handlers like `oidc_callback` (413 lines), the absence of tests for individual sub-flows (token exchange path, role-sync path, registration-required redirect) means regressions are only caught at integration level.

**[SEVERITY: High]** `src/auth/rate_limit.rs:256` — Rate-limit test manually backdates DB rows instead of time-mocking

The test directly issues raw SQL to set `attempt_at` to a past timestamp to simulate window expiry. This couples the test to the internal DB column name; a column rename silently produces wrong SQL that appears to succeed but tests the wrong behavior. The root cause is that `RateLimitStore` calls `OffsetDateTime::now_utc()` directly instead of accepting an injectable clock. Use `tokio::time::Instant` or a `Clock` trait to allow `#[tokio::test(start_paused = true)]` + `tokio::time::advance`.

**[SEVERITY: Medium]** `src/routes/auth.rs:458` and `src/middleware/require_auth.rs:202` — `test_state(db)` / `NoopCertSigner` construction duplicated

The same `NoopCertSigner` struct and full `AppState` construction are verbatim-duplicated across at minimum these two modules and `src/lib.rs:1032`. A shared `test_helpers` module would eliminate this duplication and make it easier to add new `AppState` fields without hunting for all test construction sites.

**[SEVERITY: Medium]** `src/queries/` — Several query modules lack unit tests

`src/queries/scheduled_tasks.rs`, `src/queries/services.rs`, `src/queries/autodiscovery.rs`, `src/queries/plugin_configs.rs`, and `src/queries/update_history.rs` have no inline tests. The N+1 and full-scan issues identified in the Database section would be far easier to detect and prevent regression with query-level tests using in-memory SQLite.

**[SEVERITY: Medium]** `oidc_callback` has no unit tests despite 413 lines and 7 code paths

All seven `OidcUserResolution` branches in `oidc_callback` are untested at the unit level. The registration-required redirect path, the `LinkViaOidcRequired` branch, the first-user detection, and the `sync_oidc_roles` invocation have no automated coverage. Each branch involves distinct DB interactions and redirect construction.

**[SEVERITY: Low]** `src/auth/authentication.rs` tests cover only `AuthenticationSettings` and `navigate_json_path`/`extract_mapped_roles`

`resolve_oidc_user` — the most complex function in the auth module with 7 distinct return paths — has no tests. The orphaned-link fallthrough, the `LinkViaOidcRequired` detection, and the deactivated-user short-circuit are entirely untested.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/notification_service.rs:396,437` — `tokio::time::timeout(50ms)` in tests without `start_paused = true`

Two backlog delivery tests use real wall-clock timeouts. Should use `start_paused = true` with `tokio::time::advance`.

**[SEVERITY: Medium]** `src/mqtt_lease_coordinator.rs:724` and 16 other modules — `test_db()` / `setup_test_db()` helper duplicated across 17+ modules

Beyond the documented `test_state(db)` duplication, the `test_db()` function is duplicated in 17+ modules. A shared `test_helpers` module would reduce duplication.

**[SEVERITY: Low]** `src/notification_service.rs:261-271` — `server_restarting_is_local_only` test asserts only enum construction, not behavioral intent

Should test that `svc.broadcast(msg)` does NOT write to the outbox, verifying the stated design intent.

---

## High Availability

### Strengths

- **`broadcast_server_restarting_scattered`** — `src/service_connections.rs`. Reconnect notifications are spread over a configurable jitter window to prevent thundering-herd reconnects after a controller restart.
- **`ServiceConnectionRegistry.send()` does not hold a lock across await** — Read lock is acquired, sender cloned, lock released, then the async send executes. No deadlock risk under high connection load.
- **Event poller advances cursor only past successfully delivered events** — `src/event_poller.rs:102-183`. A delivery failure stops the batch at the failing event and retries from that point. After `MAX_DELIVERY_RETRIES` (3) failures the event is permanently skipped. This prevents a single bad event from blocking all subsequent delivery indefinitely.
- **`MqttLeaseCoordinator` uses `INSERT ON CONFLICT DO NOTHING`** — `src/mqtt_lease_coordinator.rs:141-161`. Concurrent assignment attempts are idempotent at the DB level; only one instance wins the lease.
- **Enrolled-loop approval polling decoupled from ping frequency** — `src/routes/service_ws/handler/mod.rs`. A dedicated `APPROVAL_POLL_INTERVAL` (5 seconds) drives DB polls for status changes, independent of whether the service sends pings. A silent service still receives timely approval/rejection.
- **Cancellation token propagated to WebSocket connection loops** — The unified `handle_authenticated_loop` and `handle_enrolled_loop` in `service_ws/handler/mod.rs` select on `cancel_token.cancelled()`, enabling a new connection for the same service to supersede the old one immediately via `CloseReason::Superseded`.

#### 2026-02-24 Review

- **Settings distributed atomically via watch channel with write serialization.** `src/settings.rs:61-87` — Dual version counters enable efficient cross-instance invalidation polling.
- **Service connection registry handles reconnection deduplication with CancellationToken.** `src/service_connections.rs:18-36` — Old connections are cancelled via `CloseReason::Superseded`.
- **Credential-bearing MQTT messages excluded from outbox.** `src/notification_service.rs:40-52` — Prevents plaintext credential persistence.
- **Event poller cursor advancement is strictly monotonic and failure-safe.** `src/event_poller.rs:102-183`.

### Issues

**[SEVERITY: Medium]** `src/event_poller.rs:59` — `retry_counts: HashMap<i64, u8>` grows unboundedly when cursor is stuck

If the event cursor cannot advance (e.g., the target service never connects), `retry_counts` accumulates entries for every event in the stuck window. The `retain` cleanup on line 180 only fires when `new_cursor` advances. A long outage with many events produces unbounded map growth. Add a hard cap (e.g., 10,000 entries) or a TTL-based eviction that does not depend on cursor progress.

**[SEVERITY: Low]** `src/service_connections.rs:312-319` — `broadcast_server_restarting_scattered` spawns unbounded tasks

Each agent receives a separate `tokio::spawn`'d task with a random delay. With thousands of connected agents, this briefly creates thousands of tasks competing for the event loop. A bounded `tokio::sync::Semaphore` with a reasonable concurrency limit (e.g., 256) would smooth the burst without meaningfully delaying shutdown notification.

**[SEVERITY: Low]** `src/routes/service_ws/handler/mod.rs` — First approval-poll tick consumed immediately

```rust
let mut approval_poll = tokio::time::interval(APPROVAL_POLL_INTERVAL);
approval_poll.tick().await; // skip immediate first tick
```

Consuming the first tick with `.await` suspends the enrolled loop before entering the main `tokio::select!`. During the initial 5-second wait, any push message (approval/rejection) sitting in `push_rx` is not processed. For a fast-approval scenario (API call arrives between enrollment and enrollment loop start), the agent waits the full poll interval. Prefer `approval_poll.set_missed_tick_behavior(MissedTickBehavior::Delay)` and move the first-tick skip inside the select arm.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/event_poller.rs:59` — Fixed 1-second poll interval with no configurability or adaptive behavior

Every controller instance polls `controller_events` once per second regardless of activity. Should be configurable or adaptive.

**[SEVERITY: Low]** `src/middleware/rate_limit.rs:114-152` — In-memory fallback rate limiter uses `std::sync::Mutex` with no poisoning recovery and no size bound

Use `.unwrap_or_else(|e| e.into_inner())` for poisoning recovery and add a hard cap on entries.

**[SEVERITY: Low]** `src/notification_service.rs:107-177` — Backlog delivery replays up to 500 events sequentially with no timeout

Should have a per-event timeout or overall backlog delivery budget.

---

## Database

### Strengths

- **UUID v7 primary keys** — Time-ordered inserts avoid hot-spot contention on clustered indexes. Used throughout entity definitions.
- **`TenantScoped` trait** — Compile-time tenant filtering; tenant data leakage is structurally impossible through typed paths.
- **Transactions used for all multi-step mutations** — `oidc_callback`, `oidc_complete_registration`, `enroll_*`, `merge_service` all begin explicit transactions. Race conditions on first-user detection are explicitly addressed with counts inside the transaction.
- **Soft-delete partial unique indexes** — `uq_plugin_configs_active_name WHERE deactivated_at IS NULL`, `uq_software_items_active_name WHERE deactivated_at IS NULL`. Deactivated entities do not block name reuse.
- **`lock_exclusive()` before mutation in `merge_service`** — Prevents concurrent merge operations on the same service record.
- **`INSERT ON CONFLICT DO NOTHING` for lease deduplication** — `src/mqtt_lease_coordinator.rs:141-161`. Concurrent assignment is safe at the DB level.

#### 2026-02-24 Review

- **Batch plugin config loading in `list_ignore_rules`.** `src/queries/autodiscovery.rs:103-117` — Collects IDs, single `is_in` query, HashMap for O(1) lookup.
- **`load_plugins` uses JOIN instead of N+1.** `src/queries/software_items.rs:94-124`.

### Issues

**[SEVERITY: Medium]** Missing index: `update_history` has no index on `created_at`

`list_update_history` orders by `created_at DESC` with pagination, but there is no index on this column. The query degrades to a full table scan as update history grows. Add `CREATE INDEX idx_update_history_created_at ON update_history (created_at DESC)`.

**[SEVERITY: Medium]** Missing index: `host_software_items` has no index on `software_item_id` alone

The composite primary key starts with `host_id`. Queries filtering by `software_item_id` alone (e.g., finding all hosts for a given software item) cannot use the primary key and require a full table scan. Add a non-unique index on `software_item_id`.

**[SEVERITY: Medium]** Missing index: `mqtt_leases` has no index on `tenant_id`

Queries filtering leases by tenant cannot use any index beyond the primary key. Add an index on `tenant_id`.

**[SEVERITY: Medium]** `src/queries/autodiscovery.rs:673-676` — `is_unique_violation()` uses string matching on error messages

Checking uniqueness violations by matching substrings in error message strings is brittle and backend-specific (different messages for SQLite vs PostgreSQL vs MySQL). Use SeaORM's typed error variants or a helper that inspects the underlying `sqlx::Error::Database` error code instead.

**[SEVERITY: Low]** `api_tokens` table has no `expires_at` column

API tokens are valid indefinitely once issued. A compromised token that was never explicitly revoked remains valid forever. Add an optional `expires_at` column and enforce expiry at validation time.

**[SEVERITY: Low]** `sessions` table has no composite `(user_id, expires_at)` index

"Find active sessions for user" is a common query path (user detail view, session revocation). A composite index on `(user_id, expires_at)` would serve this filter efficiently.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/queries/autodiscovery.rs:36-75` — TOCTOU race in `create_or_ignore_ignore_rule`

Check-then-insert without transaction. The unique violation handler should return `Ok(())`, not propagate the error.

**[SEVERITY: Medium]** `src/queries/autodiscovery.rs:600-610` — Unbounded `host_software_items` scan in `process_one_discovery` Phase 2

Missing index on `(plugin_config_id, package_identifier)`.

**[SEVERITY: Medium]** `src/queries/update_history.rs:148-150` — Output not loaded for list_update_history

Returns empty output for records using the newer `update_output_lines` storage, unlike the detail endpoint which correctly falls back.

**[SEVERITY: Low]** `src/queries/plugin_configs.rs:87-94` — `find_raw_active_config` swallows DB errors via `.ok().flatten()`

Transient DB issues are indistinguishable from "config does not exist".

**[SEVERITY: Low]** `src/queries/software_items.rs:85-91` — `count_linked_hosts` swallows DB errors via `.unwrap_or(0)`

DB outage causes items to report zero linked hosts silently.

---

## Coding Standards

### Strengths

- **Edition 2024** — Consistent with workspace standard.
- **`SecretString` at all API input boundaries** — Password, token, OIDC secret fields in `web-api-types` and across all route handlers.
- **Permission extractors consistently applied** — 70+ endpoints protected. Handler signatures form an auditable access-control inventory.
- **No `Result<T, String>` anti-pattern** — All error types are `thiserror`-derived with typed variants.
- **No `StatusCode` numeric literals** — All comparisons use `StatusCode::*` variants or `.is_*()` helpers.
- **`FromStr` with typed errors** — Used correctly for `AlertSeverity`, `Permission`, etc.
- **Zero `#[allow(clippy::...)]` in the entire codebase** — All previously allowed lints have been resolved.

### Issues

**[SEVERITY: Medium]** `src/routes/services.rs:282` and `src/routes/hosts.rs:141` — Soft-delete `DELETE` endpoints return `200 OK` with body instead of `204 No Content`

REST convention: `DELETE` that has no meaningful response body should return `204`. Returning `200` with a body is inconsistent with the other delete endpoints in this crate that correctly return `204`, and inconsistent with what REST clients expect. Standardize to `204 No Content` or rename to `POST /deactivate` if a body is required.

**[SEVERITY: Medium]** `src/routes/autodiscovery.rs:154,159` — `create_autodiscovery_ignore` returns `201 Created` for both new and pre-existing records

When the ignore entry already exists, the endpoint returns `201 Created`. Standard REST: `201` for new creation, `200` (or `204`) for an idempotent update/pre-existing resource. Return `200 OK` when the record is found to already exist.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/routes/settings_auth.rs:96`, `src/lib.rs:941` — `#[cfg(not(feature))]` blocks undocumented

These are pragmatic uses but each should carry an inline comment explaining why the pattern is necessary.

---

## Extensibility

### Strengths

- **`PluginOps` trait in `AppState`** — `src/lib.rs:213-215`. Route handlers are fully decoupled from the concrete `PluginRegistry`. Adding a new plugin requires zero changes to web-api route code.
- **OIDC feature-gate** — The entire OIDC subsystem (15+ files) is compilable out. Deployments that do not need OIDC are not burdened with its code surface.
- **`swagger-ui` feature gate** — `Cargo.toml:13`. Swagger UI can be excluded from production builds.
- **`db-sqlite` / `db-postgres` / `db-mysql` feature gates** — `Cargo.toml:14-17`. DB backend is selected at compile time, not at runtime, enabling minimal binary sizes.
- **`ServiceConnectionRegistry` type-erased service dispatch** — `broadcast_by_capability`, `send`, `is_connected` work uniformly across agent, MQTT, and SSH-agent connection types. Adding a new service role requires only extending the registry's capability-based routing.
- **Event poller's `deliver_event` is forward-compatible** — Unknown `target_capability` values produce a broadcast (safe default) rather than a hard error. New capabilities introduced by newer controller versions are handled gracefully by older peer instances.

#### 2026-02-24 Review

- **Sequence validation decoupled from full deserialization.** `src/routes/service_ws/protocol.rs` — Two-phase parse enables forward-compatible message handling for unknown message types.

### Issues

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:873` — `fake_claims` reverse-role-mapping ignores nested claim paths

The role sync reconstruction only places values at the first dot-separated segment of `role_claim_path`. If the provider is configured with `role_claim_path = "realm.roles"`, the reconstructed claims place values at `realm` not at `realm.roles`. `sync_oidc_roles` then calls `navigate_json_path` which expects the full nested path. The reconstruction is semantically incorrect for any nested claim path. Either store the original OIDC claim values (not the local role names) in the pending registration/link store, or fix the reconstruction to build the full nested JSON structure.

**[SEVERITY: Low]** `src/routes/oidc_auth.rs` — No mechanism to add custom OIDC scopes beyond the `scopes` column

Additional claim retrieval (groups, department, cost center) requires custom scopes. The current implementation splits `plugin.scopes` on whitespace and adds each as a separate `Scope`. This works but is the only extensibility point for claims enrichment. There is no documented path for operators to add custom claims processors without code changes.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `src/routes/service_ws/protocol.rs` — `controller_capabilities()` is a hardcoded array

Missing a `Capability` variant means the controller silently disables it. Should auto-generate from all typed variants or carry an invariant comment.
