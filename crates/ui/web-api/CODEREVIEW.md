# CODEREVIEW — uptrakit-web-api

## Summary

`uptrakit-web-api` is the largest and most complex crate in the workspace (~98 `.rs` source files, approximately 1.2 MB of Rust). It implements the full HTTP/WebSocket API server: route handlers (80+ REST endpoints), WebSocket lifecycle management for agents, MQTT services, and SSH agents, OIDC authentication flows, JWT/session management, PKI operations, MQTT lease coordination, cross-controller event delivery, and database query helpers. It is a library crate consumed exclusively by `uptrakit-controller`.

The crate demonstrates several architectural strengths: a well-designed builder pattern for `AppState`, strong security primitives throughout the auth subsystem, consistent use of typed permission extractors, and a clean `CaPublicSnapshot`/`CaKeyStore` split that keeps private key material isolated. The primary concerns are concentrated in three areas: a cluster of High-severity security gaps in the authentication flow (unverified OIDC email, missing JWT audience claim, in-memory-only token revocation), critical database query performance issues (multiple N+1 patterns and full-table scans that will not scale), and code quality issues in the largest route handlers where complexity has grown well beyond maintainable bounds.

---

## Architecture

### Strengths

- **`AppState` builder pattern** — `src/lib.rs:234-493`. `AppStateBuilder` enforces that all required fields are set before construction; `AppStateBuildError` names the first missing field. Partial state cannot escape at compile time. The `provider_ops` field defaults to the real `ProviderRegistry`, making test injection a one-call override.
- **`CaPublicSnapshot` / `CaKeyStore` split** — `src/lib.rs:52-156`. Public CA data is freely cloneable and shareable. Private key material is isolated in `CaKeyStore` (not `Clone`, not `Debug`), distributed only to the three consumers that legitimately need it (OCSP, CRL, cert signer). The `split_snapshot` function ensures consistent construction.
- **`CaKeyStore` `Debug` redaction** — `src/lib.rs:98-112`. Every key field prints `"[REDACTED]"`. Verified by a dedicated test at `src/lib.rs:1284-1303`.
- **Dual-router design** — `src/lib.rs:773-989`. `build_router` (HTTPS, full middleware stack) and `build_pki_router` (plain HTTP, PKI endpoints only) are clearly separated. The PKI router intentionally omits `resolve_proxy_headers` with an explanatory comment.
- **OIDC feature-gating** — `Cargo.toml:10-12` and throughout `src/lib.rs`. OIDC types, flow stores, and routes are completely absent from the binary when the `oidc` feature is disabled. Feature-conditioned fields in `AppState` and `AppStateBuilder` are consistently paired.
- **`ProviderOps` abstraction** — `src/lib.rs:213-215`. Provider operations injected as `Arc<dyn ProviderOps>`, decoupling all route handlers from the concrete `ProviderRegistry`. Enables mock injection in tests without a running provider ecosystem.
- **Middleware layering order** — `src/lib.rs:947-960`. `resolve_proxy_headers` → `rate_limit_auth` → `resolve_ip` → `request_log`. Applied in reverse execution order as required by Axum/Tower. Proxy header stripping happens before rate limiting to prevent header-spoofed rate limit bypass.
- **OIDC state stores as separate per-concern types** — `OidcFlowStore`, `OidcRegistrationStore`, `OidcTokenExchangeStore`, `AccountLinkStore`. Each store is scoped to a single step in the OIDC flow, preventing cross-step state confusion.
- **`ServiceConnectionRegistry.send()` non-blocking** — `src/service_connections.rs`. Read lock acquired, sender cloned, lock dropped before the async send. No lock held across await points.
- **Event poller cursor safety** — `src/event_poller.rs:88`. Cursor starts at `max_id - 100` to avoid missing events between startup and first poll. Cursor only advances past events that are successfully delivered or permanently skipped, preventing silent message loss.

### Issues

**[SEVERITY: High]** `Cargo.toml:22-23` — `chrono` and `cron` are not in `[workspace.dependencies]`

`chrono = { version = "0.4", ... }` and `cron = "0.15"` are declared inline in this crate and again in `crates/core/controller/Cargo.toml:50-51`. During the SeaORM RC series, patch versions of `chrono` could diverge between the two declaration sites. Workspace-pinning eliminates this risk.

**[SEVERITY: Medium]** `Cargo.toml:46` — `base64 = "0.22"` not in workspace dependencies

`base64` is used by the web-api crate but declared inline rather than pinned via `[workspace.dependencies]`. A separate consumer adding `base64` independently risks version misalignment.

**[SEVERITY: Low]** No `rust-version` MSRV set anywhere in the workspace

`AGENTS.md` documents `rust-version = "1.91"` but no crate declares it. If this crate uses edition 2024 features, build failures on older toolchains have no documented expectation.

---

## Security & Safety

### Strengths

- **Argon2id with OWASP parameters** — `src/auth/password.rs:32-40`. 19 MiB memory cost, 2 iterations. Directly matches OWASP recommended minimums for interactive authentication.
- **JWT denylist with two revocation modes** — `src/auth/token_denylist.rs`. JTI-level revocation for single-token logout; user-level revocation (`until` timestamp) for credential rotation/compromise scenarios. Five unit tests cover boundary conditions including purge semantics and "latest timestamp wins" for successive deny calls.
- **Refresh token rotation in DB transaction with replay protection** — `src/auth/session.rs:109-183`. Rotation is atomic; a replayed refresh token cannot produce two valid sessions.
- **DB-backed rate limiter, fail-closed** — `src/auth/rate_limit.rs`. `fail_closed: true` causes a DB error to reject rather than allow. Applied at two layers: HTTP auth endpoints (middleware) and WebSocket connection/auth-failure paths (`src/routes/service_ws.rs:306-327`, `:348-382`).
- **Reverse-proxy header spoofing mitigation** — `src/middleware/resolve_proxy_headers.rs:56-62`. Cert and forwarded headers stripped from untrusted peer IPs before being re-set from trusted proxy headers.
- **All tokens stored as SHA-256 hashes** — `src/auth/token.rs:17-22`. Plaintext tokens never persisted; a DB dump reveals no replayable credentials.
- **Typed permission extractors** — `src/middleware/permission.rs:35-110`. One struct per `Permission` variant, generated by `permission_extractor!` macro. Authorization is enforced at the type level; handler signatures form an auditable access-control inventory. Nine variants, all tested.
- **Refresh cookie hardening** — `src/auth/refresh_cookie.rs`. HttpOnly, Secure, SameSite=Strict, path-scoped to `/api/v1/auth`. Not accessible to JavaScript.
- **PKCE enforced on all OIDC flows** — `src/routes/oidc_auth.rs:168`, `308`. `pkce_verifier` stored with the pending flow record and consumed at callback; code injection into a stolen authorization URL is blocked.
- **Zero `unsafe` blocks** in production code across the entire crate.
- **`SecretString` at all API input boundaries** — OIDC tokens, device-flow codes, API token responses use `SecretString`; raw bytes are not retained after the response is serialized.

### Issues

**[SEVERITY: High]** `src/auth/authentication.rs:115` — OIDC `email_verified` claim silently discarded

```rust
email_verified: _,
```

The field is destructured and immediately ignored. The caller (`src/routes/oidc_auth.rs:435`) passes `email_verified` from the ID token claims, but `resolve_oidc_user` never consults it. A misconfigured or malicious OIDC provider asserting an unverified email address (e.g., an attacker-controlled IdP with a lookalike email) will proceed to account creation or matching. The fix is to check `if email_verified == Some(false) { return Ok(OidcUserResolution::EmailNotVerified); }` as the first guard inside `resolve_oidc_user`.

**[SEVERITY: High]** `src/routes/server_cert.rs:199` — mTLS uses `.allow_unauthenticated()`

`WebPkiClientVerifier::builder(...).allow_unauthenticated()` permits TLS connections from clients that present no certificate at all. The agent WebSocket trust boundary then relies entirely on application-layer bearer-secret or service-ID checks rather than transport-layer PKI enforcement. A network-adjacent attacker can establish a TLS session and probe the anonymous enrollment path without any certificate. Correct production posture is `.build()` (unauthenticated clients rejected at TLS handshake).

**[SEVERITY: Medium]** `src/auth/jwt.rs:101` — JWT validation uses `Validation::default()` with no audience check

```rust
jsonwebtoken::decode::<AccessTokenClaims>(token, &self.decoding_key, &Validation::default())
```

`Validation::default()` does not enforce `aud`. A token minted for a staging environment is accepted in production if the same HMAC key is shared. Additionally, `iss` and `sub` format are not validated. At minimum, `validation.set_audience(&["uptrakit-controller"])` should be added and `iss` should be set to the controller's own hostname.

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

---

## Code Quality

### Strengths

- **Two-pass `deserialize_service_msg`** — `src/routes/service_ws.rs:141-162`. Step 1 extracts only the sequence number (hard fail on malformed JSON). Step 2 validates sequence (hard fail on mismatch). Step 3 performs full deserialization (soft fail for unknown future message types). The three-phase contract is clearly documented and allows replay-protection to remain accurate even when the full payload cannot be parsed.
- **`MessageRateLimiter` with injected clock for testing** — `src/routes/service_ws.rs:34-70`. `set_window_start` is `#[cfg(test)]`-only; the test at line 1228 directly manipulates the window start to avoid real wall-clock sleeps.
- **Uniform error propagation** — `bail!`, `report!`, `context_to`, `impl_report_conversion!` used consistently throughout. No `Report::new()` anti-pattern, no `Result<T, String>`.
- **Zero `#[allow(dead_code)]` or `#[allow(unused)]` annotations** anywhere in the crate.
- **`WS_MESSAGE_RATE_LIMIT` / `WS_MESSAGE_RATE_WINDOW` named constants** — `src/routes/service_ws.rs:29-31`. WebSocket rate-limit parameters are named and documented, not magic numbers.
- **`MAX_WS_MESSAGE_SIZE` and `ANONYMOUS_TIMEOUT` named** — `src/routes/service_ws.rs:797-801`. Domain-meaningful values with doc comments.
- **`APPROVAL_POLL_INTERVAL` named constant** — `src/routes/agent_ws.rs:635`. Agent enrolled-loop poll interval is explicit and documented.
- **`MAX_UPDATE_OUTPUT_BYTES` named constant** — `src/routes/agent_ws.rs:48`. Output cap is named and the cap-enforcement logic (conditional `UPDATE` + `rows_affected == 0` guard) is well-documented.
- **`model_to_config` isolation in lease coordinator** — `src/mqtt_lease_coordinator.rs:687-712`. The conversion from DB model to wire type is a single private function, not repeated inline across all callers.

### Issues

**[SEVERITY: High]** `src/routes/service_ws.rs:610-614` — Magic value `120` repeated three times with a wildcard arm that silently accepts future service types

```rust
service_entity::ServiceType::Agent => Some(120),
service_entity::ServiceType::Mqtt => None,
service_entity::ServiceType::SshAgent => Some(120),
_ => Some(120),
```

`120` should be extracted to `AGENT_SHUTDOWN_TIMEOUT_SECS` in `durations.rs`. More critically, the `_ => Some(120)` wildcard silently assigns a shutdown timeout to any future `ServiceType` variant added to the enum, rather than forcing the developer to make an explicit decision. Remove the wildcard; the compiler will flag new variants exhaustively.

**[SEVERITY: High]** `src/routes/agent_ws.rs:453` — Non-exhaustive wildcard on `UpdateFinalStatus` match

```rust
UpdateFinalStatus::Failed | _ => update_history::UpdateStatus::Failed,
```

Any new `UpdateFinalStatus` variant (e.g., `Cancelled`, `TimedOut`) silently maps to `Failed`. This produces incorrect update history records without any compile-time warning. Make the match exhaustive.

**[SEVERITY: High]** `src/routes/oidc_auth.rs:224-637` — `oidc_callback` is 413 lines with 7+ nesting levels

The function simultaneously handles: parameter validation, state store lookup, provider loading, OIDC discovery (again — see duplication below), PKCE token exchange, ID token validation, claims extraction, registration-mode pre-check, transaction management, user resolution (5 `OidcUserResolution` branches), role sync, session creation, and redirect construction. This is the second-largest function in the entire codebase. Maximum nesting depth exceeds 7 `match`/`if let` levels, making control flow difficult to audit. Decompose into: `exchange_code_for_token`, `validate_id_token`, `resolve_or_create_user`, and `create_session_and_redirect`.

**[SEVERITY: High]** `src/routes/oidc_auth.rs:873-906` and `src/routes/oidc_auth.rs:1088-1124` — Identical "fake claims" role reverse-mapping block duplicated verbatim

Both `oidc_complete_registration` and `oidc_link` contain the same ~30-line block that reverse-maps local role names to OIDC claim values, constructs a synthetic `serde_json::Map`, and calls `sync_oidc_roles`. Even the comment "Set at the first path segment for simplicity" is identical. The "first path segment only" limitation (line 887/1106) is a known semantic restriction that only exists in one place and could silently diverge. Extract: `fn build_fake_claims_for_sync(provider: &OidcProvider, mapped_roles: &[String]) -> serde_json::Value`.

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:142-165` and `src/routes/oidc_auth.rs:272-296` — OIDC client construction via discovery duplicated between `oidc_authorize` and `oidc_callback`

~20 identical lines of `IssuerUrl::new` + `CoreProviderMetadata::discover_async` + `CoreClient::from_provider_metadata` + `set_redirect_uri`. Error handling is inconsistent: `oidc_authorize` returns `BAD_GATEWAY` (correct), while `oidc_callback` redirects to `/login?error=oidc_discovery_failed` (inconsistent semantics for what is an outbound HTTP failure). Extract `async fn build_oidc_client(provider, redirect_url) -> Result<CoreClient, Response>`.

**[SEVERITY: Medium]** `src/routes/service_ws.rs:973-1189` — `enroll_agent`, `enroll_mqtt`, `enroll_ssh_agent` share an identical 47-line post-enrollment shape

All three functions execute the same sequence: map status to wire enum, build `EnrolledPayload`, serialize and send, log, check `approved`, conditionally send `ApprovedPayload`. Differences are only in the result type and log message. The three functions are 67, 62, and 62 lines respectively, of which ~47 lines are identical. Extract `fn send_enrollment_result(service_id, enrollment_secret, status, sink, out_seq) -> Option<(Uuid, bool)>`.

**[SEVERITY: Medium]** `src/routes/service_ws.rs:616-623` — Magic values `300` and `15` for ping interval defaults

```rust
service_entity::ServiceType::Agent | service_entity::ServiceType::SshAgent => 300u32,
service_entity::ServiceType::Mqtt => 15u32,
```

These are domain-significant heartbeat intervals (agent: 5 minutes, MQTT: 15 seconds). Name them `AGENT_DEFAULT_PING_INTERVAL_SECS` and `MQTT_DEFAULT_PING_INTERVAL_SECS` in `durations.rs`.

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:66` and `src/routes/oidc_auth.rs:352` — `unwrap_or_default()` on DB queries silently returns empty results

`OidcProvider::find().all(...).await.unwrap_or_default()` at line 66 returns an empty provider list on DB error, causing `auth_methods` to report "no OIDC providers configured" rather than a server error. At line 352, `count(...).unwrap_or(1)` treats a DB error as "link exists", gating the user out of the OIDC flow. Both should log with `tracing::warn!` or propagate as errors.

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:751` and `src/routes/oidc_auth.rs:995` — Session creation four-step pattern duplicated three times

The pattern of `SessionService::new(...)`, `create_refresh_token(...)`, `get_user_permissions(...)`, `create_access_token(...)`, and building `AuthResponse` is repeated identically in `oidc_exchange` (lines 668-733), `oidc_complete_registration` (lines 913-980), and `oidc_link` (lines 1128-1193). Extract `async fn create_oidc_session(state, user_id, provider_id) -> Response`.

**[SEVERITY: Low]** `src/routes/agent_ws.rs:1217-1220` — Discovery-capable provider types hardcoded as a slice literal

```rust
let discovery_types: &[ProviderType] = &[
    ProviderType::Homebrew,
    ProviderType::ProxmoxHelperScripts,
];
```

This is a second source of truth that diverges from `ProviderType::supports_discovery()` in `shared/types`. When a new discovery-capable provider is added, this slice must also be updated manually. A missed update silently prevents discovery for the new provider type.

---

## Tests

### Strengths

- **`permission_extractor!` macro fully tested** — `src/middleware/permission.rs:116-209`. Six tests cover: missing auth extension → 401, correct permission → pass, wrong permission → 403, no permissions → 403, multiple permissions with one match → pass, and `new()` constructor bypass semantics.
- **`MessageRateLimiter` unit-tested with clock injection** — `src/routes/service_ws.rs:1227-1237`. No real wall-clock sleep; window start is directly manipulated via the `#[cfg(test)]` helper.
- **`deserialize_service_msg` three-path coverage** — `src/routes/service_ws.rs:1199-1225`. Tests cover unknown message type → `Ok(None)`, malformed JSON → `Err`, sequence mismatch → `Err(SequenceValidation)`.
- **`record_service_activity` DB tests** — `src/routes/service_ws.rs:1290-1325`. In-memory SQLite verifies IP update and last-seen-at semantics for both `Some(ip)` and `None` cases.
- **`MqttLeaseCoordinator` well-covered** — `src/mqtt_lease_coordinator.rs:714-905`. Four integration tests using in-memory SQLite: new client leased, no local service, already leased, batch assignment skips already-leased clients.
- **`EventPoller` cursor behavior tested** — `src/event_poller.rs:384-427`. Safety margin test verifies cursor = max_id - 100; stale-event-skip test verifies events created before service connect time are skipped without delivery.
- **`TokenDenylist` comprehensively tested** — `src/auth/token_denylist.rs:104-179`. Five tests including purge semantics, boundary conditions for `iat == until`, and "latest timestamp wins" semantics for successive deny calls.
- **`base_url_from_headers` unit tests** — `src/routes/oidc_auth.rs:1261-1289`. Three cases: Origin preferred over Host, Host fallback, missing both returns None.
- **Router integration tests** — `src/lib.rs:1140-1281`. Tower `oneshot` tests verify healthz, CA cert response, 404 handling, `ConnectInfo<SocketAddr>` injection for both main and PKI routers, and trusted proxy IP resolution.
- **Security-sensitive paths tested** — JWT wrong secret, denylist revocation, OIDC state one-time-use, device-flow consume, session double-approve, rate-limit window reset.

### Issues

**[SEVERITY: High]** Route handlers in `src/routes/` have no inline unit tests for the majority of business-logic paths

The following route files have zero `#[cfg(test)]` coverage: `hosts.rs`, `agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`, `oidc_providers.rs`, `server_cert.rs`, `settings_auth.rs`, `ocsp.rs`. Given the complexity of handlers like `oidc_callback` (413 lines), the absence of tests for individual sub-flows (token exchange path, role-sync path, registration-required redirect) means regressions are only caught at integration level.

**[SEVERITY: High]** `src/auth/rate_limit.rs:256` — Rate-limit test manually backdates DB rows instead of time-mocking

The test directly issues raw SQL to set `attempt_at` to a past timestamp to simulate window expiry. This couples the test to the internal DB column name; a column rename silently produces wrong SQL that appears to succeed but tests the wrong behavior. The root cause is that `RateLimitStore` calls `OffsetDateTime::now_utc()` directly instead of accepting an injectable clock. Use `tokio::time::Instant` or a `Clock` trait to allow `#[tokio::test(start_paused = true)]` + `tokio::time::advance`.

**[SEVERITY: Medium]** `src/routes/auth.rs:458` and `src/middleware/require_auth.rs:202` — `test_state(db)` / `NoopCertSigner` construction duplicated

The same `NoopCertSigner` struct and full `AppState` construction are verbatim-duplicated across at minimum these two modules and `src/lib.rs:1032`. A shared `test_helpers` module would eliminate this duplication and make it easier to add new `AppState` fields without hunting for all test construction sites.

**[SEVERITY: Medium]** `src/queries/` — Several query modules lack unit tests

`src/queries/scheduled_tasks.rs`, `src/queries/services.rs`, `src/queries/autodiscovery.rs`, `src/queries/provider_configs.rs`, and `src/queries/update_history.rs` have no inline tests. The N+1 and full-scan issues identified in the Database section would be far easier to detect and prevent regression with query-level tests using in-memory SQLite.

**[SEVERITY: Medium]** `oidc_callback` has no unit tests despite 413 lines and 7 code paths

All seven `OidcUserResolution` branches in `oidc_callback` are untested at the unit level. The registration-required redirect path, the `LinkViaOidcRequired` branch, the first-user detection, and the `sync_oidc_roles` invocation have no automated coverage. Each branch involves distinct DB interactions and redirect construction.

**[SEVERITY: Low]** `src/auth/authentication.rs` tests cover only `AuthenticationSettings` and `navigate_json_path`/`extract_mapped_roles`

`resolve_oidc_user` — the most complex function in the auth module with 7 distinct return paths — has no tests. The orphaned-link fallthrough, the `LinkViaOidcRequired` detection, and the deactivated-user short-circuit are entirely untested.

---

## High Availability

### Strengths

- **`broadcast_server_restarting_scattered`** — `src/service_connections.rs`. Reconnect notifications are spread over a configurable jitter window to prevent thundering-herd reconnects after a controller restart.
- **`ServiceConnectionRegistry.send()` does not hold a lock across await** — Read lock is acquired, sender cloned, lock released, then the async send executes. No deadlock risk under high connection load.
- **Event poller advances cursor only past successfully delivered events** — `src/event_poller.rs:102-183`. A delivery failure stops the batch at the failing event and retries from that point. After `MAX_DELIVERY_RETRIES` (3) failures the event is permanently skipped. This prevents a single bad event from blocking all subsequent delivery indefinitely.
- **`MqttLeaseCoordinator` uses `INSERT ON CONFLICT DO NOTHING`** — `src/mqtt_lease_coordinator.rs:141-161`. Concurrent assignment attempts are idempotent at the DB level; only one instance wins the lease.
- **Enrolled-loop approval polling decoupled from ping frequency** — `src/routes/agent_ws.rs:635`, `669-670`. A dedicated `APPROVAL_POLL_INTERVAL` (5 seconds) drives DB polls for status changes, independent of whether the agent sends pings. A silent agent still receives timely approval/rejection.
- **Cancellation token propagated to WebSocket connection loops** — Both `handle_agent_authenticated` and `run_agent_enrolled_loop` select on `cancel_token.cancelled()`, enabling a new connection for the same agent to supersede the old one immediately via `CloseReason::Superseded`.

### Issues

**[SEVERITY: High]** `src/mqtt_client_store.rs:235-263` — N+1 sequential DB updates in `update_mqtt_clients_status`

For each MQTT client ID in the list, a separate `SELECT + UPDATE` is issued serially. A batch of 20 MQTT clients produces 40 DB round-trips. Additionally, there is no wrapping transaction: a partial failure leaves some clients `Offline` and some in the previous state, creating permanent status inconsistency. Replace with a single `UPDATE mqtt_clients SET connection_status = ? WHERE id IN (...)`.

**[SEVERITY: High]** `src/mqtt_lease_coordinator.rs:533-576` — `reconcile_mqtt_clients` silently takes over a live peer's lease without notification

When `existing.instance_id != instance_id` (line 545-555), the function overwrites the `instance_id` and `heartbeat_at` without sending a `TenantRevoked` message to the prior holder or removing the prior holder from the in-memory registry. The result: two controller instances simultaneously believe they hold the same MQTT lease — duplicate MQTT connections, conflicting client-status messages, and broken QoS delivery. The prior holder must be notified and its registry entry cleaned up before the takeover is committed.

**[SEVERITY: Medium]** `src/event_poller.rs:59` — `retry_counts: HashMap<i64, u8>` grows unboundedly when cursor is stuck

If the event cursor cannot advance (e.g., the target service never connects), `retry_counts` accumulates entries for every event in the stuck window. The `retain` cleanup on line 180 only fires when `new_cursor` advances. A long outage with many events produces unbounded map growth. Add a hard cap (e.g., 10,000 entries) or a TTL-based eviction that does not depend on cursor progress.

**[SEVERITY: Medium]** `src/routes/agent_ws.rs:937-1010` — `deliver_pending_updates` issues N sequential DB queries per pending update

For each pending update record the function issues separate sequential queries for: software item, host-software-item link, provider config, and host. With M pending updates that is 4M sequential round-trips. Batch the pending-updates delivery: load all software items, all links, all provider configs, and all hosts in four single queries using `is_in`, then join in memory.

**[SEVERITY: Low]** `src/service_connections.rs:312-319` — `broadcast_server_restarting_scattered` spawns unbounded tasks

Each agent receives a separate `tokio::spawn`'d task with a random delay. With thousands of connected agents, this briefly creates thousands of tasks competing for the event loop. A bounded `tokio::sync::Semaphore` with a reasonable concurrency limit (e.g., 256) would smooth the burst without meaningfully delaying shutdown notification.

**[SEVERITY: Low]** `src/routes/agent_ws.rs:669-670` — First approval-poll tick consumed immediately

```rust
let mut approval_poll = tokio::time::interval(APPROVAL_POLL_INTERVAL);
approval_poll.tick().await; // skip immediate first tick
```

Consuming the first tick with `.await` suspends the enrolled loop before entering the main `tokio::select!`. During the initial 5-second wait, any push message (approval/rejection) sitting in `push_rx` is not processed. For a fast-approval scenario (API call arrives between enrollment and enrollment loop start), the agent waits the full poll interval. Prefer `approval_poll.set_missed_tick_behavior(MissedTickBehavior::Delay)` and move the first-tick skip inside the select arm.

---

## Database

### Strengths

- **UUID v7 primary keys** — Time-ordered inserts avoid hot-spot contention on clustered indexes. Used throughout entity definitions.
- **`TenantScoped` trait** — Compile-time tenant filtering; tenant data leakage is structurally impossible through typed paths.
- **Transactions used for all multi-step mutations** — `oidc_callback`, `oidc_complete_registration`, `enroll_*`, `merge_service` all begin explicit transactions. Race conditions on first-user detection are explicitly addressed with counts inside the transaction.
- **Soft-delete partial unique indexes** — `uq_provider_configs_active_name WHERE deactivated_at IS NULL`, `uq_software_items_active_name WHERE deactivated_at IS NULL`. Deactivated entities do not block name reuse.
- **`lock_exclusive()` before mutation in `merge_service`** — Prevents concurrent merge operations on the same service record.
- **`INSERT ON CONFLICT DO NOTHING` for lease deduplication** — `src/mqtt_lease_coordinator.rs:141-161`. Concurrent assignment is safe at the DB level.

### Issues

**[SEVERITY: High]** `src/queries/update_history.rs:78-83` — Full host table scan for tenant scoping

`tenant_host_ids()` loads ALL host rows for the tenant into application memory as a `Vec<Uuid>`, then passes them as an `IN (...)` clause. With thousands of hosts, this performs a full table scan on every call, transfers unbounded data across the DB connection, and risks exceeding driver-level parameter limits (SQLite: 32,766 per query; PostgreSQL: 65,535 per prepared statement). Replace with a JOIN or correlated subquery: `WHERE host_id IN (SELECT id FROM hosts WHERE tenant_id = ?)`.

**[SEVERITY: High]** `src/queries/software_items.rs:126-178` — N+1 in `load_item_hosts`

For N hosts linked to a software item: 1 query for all `host_software_item` links + N individual `find_by_id(host_id)` + N individual `find_by_id(provider_config_id)` = 1+2N queries. This function is called from `get_software_item`, `assign_hosts`, and `update_host_assignment`. A software item assigned to 50 hosts produces 101 queries. Replace with a single JOIN across `host_software_items`, `hosts`, and `provider_configs`.

**[SEVERITY: High]** `src/queries/update_history.rs:146-151` — N+1 in `list_update_history`

For each record in a page of 20, `resolve_host_name` and `resolve_software_item_name` each issue individual DB queries: 40 extra round-trips per page. Collect all `host_id` and `software_item_id` values from the page, batch-load with `is_in(ids)`, then join in memory before constructing the response.

**[SEVERITY: High]** `src/queries/autodiscovery.rs:567-580` — Ignore-list `COUNT(*)` inside per-item loop

`process_one_discovery` issues a `COUNT(*)` query against `autodiscovery_ignores` per discovered item. For 200 discovered packages, this is 200 DB round-trips. Load the entire ignore list for the tenant into a `HashSet<String>` before entering the loop, then check membership in O(1).

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

---

## Coding Standards

### Strengths

- **Edition 2024** — Consistent with workspace standard.
- **`SecretString` at all API input boundaries** — Password, token, OIDC secret fields in `web-api-types` and across all route handlers.
- **Permission extractors consistently applied** — 70+ endpoints protected. Handler signatures form an auditable access-control inventory.
- **No `Result<T, String>` anti-pattern** — All error types are `thiserror`-derived with typed variants.
- **No `StatusCode` numeric literals** — All comparisons use `StatusCode::*` variants or `.is_*()` helpers.
- **`FromStr` with typed errors** — Used correctly for `AlertSeverity`, `Permission`, etc.
- **Only one `#[allow(clippy::...)]` in the entire codebase** — `src/queries/autodiscovery.rs:554` (addressed below).

### Issues

**[SEVERITY: High]** `src/routes/api_tokens.rs:19-31` and `src/routes/auth.rs:339,666` — `x-required-permission` annotation missing on user-identity endpoints

`create_api_token`, `list_api_tokens`, `revoke_api_token`, `logout`, and `me` use raw `Extension<AuthenticatedUser>` directly (appropriate for user-scoped resources) but lack the `x-required-permission` OpenAPI extension. The OpenAPI spec consequently omits the permission requirement from generated clients, and the automated permission-coverage check cannot verify these endpoints. Add `x-required-permission: "self"` or an equivalent sentinel value to document that these endpoints require only authentication, not a specific resource permission.

**[SEVERITY: High]** `src/queries/autodiscovery.rs:554` — `#[allow(clippy::too_many_arguments)]` violates AGENTS.md

AGENTS.md states: "There are currently no approved exceptions." `process_one_discovery` takes 8 arguments. Fix: introduce a `ProcessDiscoveryArgs<'a>` struct grouping `package_identifier`, `name`, `installed_version`, `provider_type_str`, and other related parameters. After restructuring, the argument count will drop below Clippy's default threshold of 7 and the suppression can be removed.

**[SEVERITY: Medium]** Pervasive `Path<String>` + manual `uuid::Uuid::parse_str` pattern — 43 occurrences across 10 route files

Axum natively supports `Path(id): Path<Uuid>` which performs the same parse and returns a typed 422 on failure. All 43 occurrences use the manual pattern (e.g., `hosts.rs:77`, `services.rs:98`, `software_items.rs:146`, `api_tokens.rs:111`, `provider_configs.rs:106`). Replace with typed extraction throughout: eliminates boilerplate, produces consistent error responses, and removes 43 instances of hand-written UUID validation.

**[SEVERITY: Medium]** `src/routes/services.rs:282` and `src/routes/hosts.rs:141` — Soft-delete `DELETE` endpoints return `200 OK` with body instead of `204 No Content`

REST convention: `DELETE` that has no meaningful response body should return `204`. Returning `200` with a body is inconsistent with the other delete endpoints in this crate that correctly return `204`, and inconsistent with what REST clients expect. Standardize to `204 No Content` or rename to `POST /deactivate` if a body is required.

**[SEVERITY: Medium]** `src/routes/autodiscovery.rs:154,159` — `create_autodiscovery_ignore` returns `201 Created` for both new and pre-existing records

When the ignore entry already exists, the endpoint returns `201 Created`. Standard REST: `201` for new creation, `200` (or `204`) for an idempotent update/pre-existing resource. Return `200 OK` when the record is found to already exist.

**[SEVERITY: Low]** All utoipa path-parameter annotations declare `String` type instead of `Uuid` — 43 occurrences

```rust
("id" = String, Path, description = "...")
```

The generated OpenAPI schema should declare `format: uuid`. Update all 43 path param annotations to `Uuid` type. This affects every endpoint with UUID path params and makes the generated client code produce UUID-typed parameters rather than raw strings.

---

## Extensibility

### Strengths

- **`ProviderOps` trait in `AppState`** — `src/lib.rs:213-215`. Route handlers are fully decoupled from the concrete `ProviderRegistry`. Adding a new provider requires zero changes to web-api route code.
- **OIDC feature-gate** — The entire OIDC subsystem (15+ files) is compilable out. Deployments that do not need OIDC are not burdened with its code surface.
- **`swagger-ui` feature gate** — `Cargo.toml:13`. Swagger UI can be excluded from production builds.
- **`db-sqlite` / `db-postgres` / `db-mysql` feature gates** — `Cargo.toml:14-17`. DB backend is selected at compile time, not at runtime, enabling minimal binary sizes.
- **`ServiceConnectionRegistry` type-erased service dispatch** — `broadcast_by_type`, `send`, `is_connected` work uniformly across agent, MQTT, and SSH-agent connection types. Adding a new service type requires only extending the registry's service-type routing.
- **Event poller's `deliver_event` is forward-compatible** — Unknown `target_service_type` values produce a broadcast (safe default) rather than a hard error. New service types introduced by newer controller versions are handled gracefully by older peer instances.

### Issues

**[SEVERITY: High]** `src/routes/agent_ws.rs:1217-1220` — Discovery-capable provider types hardcoded as a slice literal

```rust
let discovery_types: &[ProviderType] = &[
    ProviderType::Homebrew,
    ProviderType::ProxmoxHelperScripts,
];
```

This is a third source of truth for discovery capability, duplicating `ProviderType::supports_discovery()` in `shared/types` and `create_provider_for_discovery` in `providers/registry`. A new provider with discovery capability requires updates in all three locations. A missed update silently prevents autodiscovery without any compile-time feedback. The canonical source should be `Provider::capabilities()` on each provider implementation; `agent_ws.rs` should query the registry for providers that return `ProviderCapability::SoftwareDiscovery` rather than maintaining its own list.

**[SEVERITY: High]** `src/queries/software_items.rs:329-332` — Package identifier validation uses raw string comparison

```rust
if config.provider_type == "homebrew" {
```

Provider-specific identifier constraints are hardcoded as raw string matches in the query layer. This bypasses the `Provider` trait entirely and creates a new scattered location for provider-specific knowledge. New providers with identifier constraints must add branches here. Move validation logic to a `fn validate_package_identifier(&self, value: &str) -> Result<()>` method on the `Provider` trait, callable from the query layer through `ProviderOps`.

**[SEVERITY: Medium]** `src/routes/oidc_auth.rs:873` — `fake_claims` reverse-role-mapping ignores nested claim paths

The role sync reconstruction only places values at the first dot-separated segment of `role_claim_path`. If the provider is configured with `role_claim_path = "realm.roles"`, the reconstructed claims place values at `realm` not at `realm.roles`. `sync_oidc_roles` then calls `navigate_json_path` which expects the full nested path. The reconstruction is semantically incorrect for any nested claim path. Either store the original OIDC claim values (not the local role names) in the pending registration/link store, or fix the reconstruction to build the full nested JSON structure.

**[SEVERITY: Low]** `src/routes/oidc_auth.rs` — No mechanism to add custom OIDC scopes beyond the `scopes` column

Additional claim retrieval (groups, department, cost center) requires custom scopes. The current implementation splits `provider.scopes` on whitespace and adds each as a separate `Scope`. This works but is the only extensibility point for claims enrichment. There is no documented path for operators to add custom claims processors without code changes.
