# CODEREVIEW — Workspace

## Summary

The Uptrakit backend is a well-structured Rust workspace of 24 crates spanning four clearly separated domains: `core/` (binaries), `plugins/` (pluggable detection and update drivers), `shared/` (libraries), and `ui/` (HTTP API and CLI). The codebase consistently applies Rust 2024 edition and resolver version 3 across every crate, uses workspace-pinned dependency versions for all major libraries, and leans on strong type-system patterns — typed permission extractors, `SecretString` at API boundaries, `Zeroizing<>` on key material — that enforce security invariants at compile time rather than by convention. The release profile is production-hardened (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`), and the overall dependency DAG is sound with one well-defined layering violation.

The mTLS configuration in `uptrakit-controller` uses `.allow_unauthenticated()`, which is intentional for reverse-proxy deployments — this is documented in `pki.rs` and `mtls_acceptor.rs`.

The extensibility seam for plugins is well-designed at the `Plugin` trait and `register_plugins!` macro level. Discovery-capable plugin support is now fully registry-driven — `discovery_plugins()` is auto-generated from the macro, and `create_plugin_for_discovery` is macro-generated too; no manual sync is needed. Package-identifier validation is handled through `PluginRegistry::validate_package_identifier`, completing the plugin extensibility story.

---

## Architecture

### Strengths

- `Cargo.toml` (workspace root): `resolver = "3"` and `edition = "2024"` are set once at workspace root; all 24 crates inherit via `edition.workspace` — no per-crate drift possible.
- `Cargo.toml:13-60` (`[workspace.dependencies]`): All major external dependencies — `tokio`, `axum`, `sea-orm`, `rustls`, `serde`, `time`, `aws-lc-rs`, `rcgen`, `uuid`, `rand`, and 25+ more — are pinned here with exact version ranges and default-feature overrides. Per-crate declarations only specify `features = [...]`, eliminating duplicate version declarations across 24 crates.
- `Cargo.toml:62-67` (`[profile.release]`): `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true` — production-hardened single-binary output with minimal attack surface.
- `[workspace.package]` carries `license`, `authors`, `repository`, and `version` so every crate can use `*.workspace = true`, ensuring metadata consistency across all published artifacts.
- The four-domain layout (`core/`, `plugins/`, `shared/`, `ui/`) enforces a natural dependency gradient: plugins never import from `ui/`, binaries import from `shared/` and `ui/`, `shared/` libraries only import each other in a defined order. This makes cross-cutting concerns auditable.
- `uptrakit-shared-types/Cargo.toml:9-13`: `sea-orm` and `openapi` features correctly gate optional ORM and OpenAPI dependencies — the correct pattern for workspace-wide shared type crates.
- `Cargo.toml` (workspace root): `[workspace.lints]` enforces `warnings = "deny"` and `clippy::all = "deny"` across all 26 crates via `[lints] workspace = true`.

### Issues

**[SEVERITY: Low]** `Cargo.toml` (workspace root) — No `rust-version` MSRV declared

`AGENTS.md:109` states "Some specify `rust-version = \"1.91\"`", but zero crates in the workspace actually set `rust-version`. The workspace `[workspace.package]` section has no `rust-version` field. Without an MSRV declaration, `cargo check` on any Rust version will succeed, and edition-2024 features may silently break older toolchains used by downstream packagers or CI matrix jobs. The correct fix is a single `rust-version = "1.85"` (or whichever minimum is validated) in `[workspace.package]` and updating AGENTS.md to reflect the actual state.

#### 2026-02-24 Review

**[SEVERITY: Medium]** Multiple files across `crates/core/controller/` and `crates/ui/web-api/` — 11 `#[cfg(not(feature = "..."))]` usages across 5 files violate the additive feature flag rule

AGENTS.md states that "Feature flags must be additive (no `#[cfg(not(feature = "..."))]`)." A workspace-wide search identifies 11 occurrences across 5 source files, plus 2 `#[cfg_attr(not(feature), allow(...))]` usages:

| File | Lines | Feature | Purpose |
|------|-------|---------|---------|
| `crates/core/controller/src/db/config.rs` | 23, 48, 53, 58 | `db-sqlite`, `db-postgres`, `db-mysql` | Error bail for unsupported DB schemes |
| `crates/core/controller/src/db/config.rs` | 13, 42-45 | `db-sqlite`, `db-*` | `#[cfg_attr(not(...), allow(...))]` |
| `crates/core/controller/src/cli.rs` | 120 | `embed-frontend` | Remove `--static-dir` CLI arg |
| `crates/core/controller/src/startup.rs` | 584, 907 | `embed-frontend` | Remove static-dir resolution |
| `crates/ui/web-api/src/routes/settings_auth.rs` | 96 | `oidc` | Error when disabling password auth |
| `crates/ui/web-api/src/middleware/rate_limit.rs` | 228 | `oidc` | Test expectation list |
| `crates/ui/web-api/src/lib.rs` | 941 | `swagger-ui` | Fallback OpenAPI JSON route |

All can be refactored to use positive `#[cfg(feature = "...")]` gates only.

**[SEVERITY: Low]** `CODEREVIEW.md:23` vs `AGENTS.md` codebase layout — Workspace CODEREVIEW documents "binaries import from `shared/` and `ui/`" but AGENTS.md layout implies a stricter four-tier DAG

The AGENTS.md should include explicit dependency direction statements to eliminate ambiguity about the permitted dependency directions between the four domains.

---

## Security & Safety

### Strengths

- `crates/shared/crypto/src/lib.rs`: AES-256-GCM with `aws-lc-rs` (FIPS-validated primitives), 96-bit random nonce per encryption, `Zeroizing<[u8; 32]>` master key stored in `OnceLock` — never serialized or logged.
- `crates/ui/web-api/src/auth/password.rs:32-40`: Argon2id with OWASP Interactive-tier parameters (19 MiB memory, 2 iterations). Return type is `SecretString` — plaintext password is not retained beyond the hash call.
- `crates/ui/web-api/src/middleware/permission.rs`: `permission_extractor!` macro generates typed Axum extractors for all 9 permission levels. Authorization is auditable by function signature alone; no `has_permission()` calls in handler bodies.
- `crates/ui/web-api/src/auth/token_denylist.rs` and `crates/ui/web-api/src/auth/session.rs:109-183`: JWT denylist at JTI and user-id granularity; refresh token rotation in a DB transaction with replay detection.
- `crates/ui/web-api/src/middleware/resolve_proxy_headers.rs:56-62`: Reverse proxy header spoofing mitigation — cert and forwarded headers stripped from connections not originating from a trusted proxy IP.
- `crates/shared/web-api-types/`: All password, token, and secret response fields use `secrecy::SecretString`, enforcing the no-secrets-in-logs invariant at the type level across all API response structs.
- `crates/core/controller/src/pki.rs`: mTLS with `WebPkiClientVerifier` + CRL verification; CA private keys stored AES-256-GCM encrypted in DB and held as `Zeroizing<String>` in memory.
- Zero `unsafe` blocks in production code across all 24 crates.

### Issues

**[SEVERITY: Medium]** `crates/shared/crypto/src/lib.rs:244-251` — `EncryptedString::new` silently degrades to plaintext when master key is absent

When `MASTER_KEY` is not initialised, `EncryptedString::new` logs `tracing::warn!` and stores the plaintext value. There is no startup guard or environment assertion that prevents a production deployment from running in this mode. OIDC client secrets, provider API keys, and CA private key material would all be stored cleartext in the database with no machine-readable indicator beyond a transient log line. This is a workspace-level policy concern: `uptrakit-service-sdk` initializes the application lifecycle, and the master-key check belongs there as a hard startup failure, not a soft warning in the crypto library.

**[SEVERITY: Medium]** `crates/ui/web-api/src/auth/jwt.rs:101` — JWT validation uses `Validation::default()` with no audience claim

No `aud` claim is required during token validation. A JWT issued for one deployment environment will be accepted by another if the HMAC signing key is shared or accidentally reused. There is also no `iss` or `sub` format validation. This is a workspace-level concern because the JWT signing key file path and generation are handled in `uptrakit-service-sdk` and `uptrakit-directories` — the key lifecycle spans crates, so the validation policy must be defined and enforced consistently.

**[SEVERITY: Medium]** `crates/ui/web-api/src/auth/token_denylist.rs:15` — In-memory JWT denylist provides no cross-instance revocation

The denylist is per-process. A revoked token remains valid on all other running instances for up to the JWT TTL (15 minutes). The code comment at line 15 acknowledges this. For a system managing infrastructure access with potential privilege escalation via compromised tokens, a 15-minute window is operationally significant. A shared DB-backed denylist — consistent with the existing refresh token and rate-limit tables — would close this gap. This is a workspace-level HA and security intersection: the multi-instance architecture described in `docs/development/cross-controller-comm.md` is undermined by per-process revocation state.

**[SEVERITY: Low]** `crates/shared/directories/src/lib.rs:829,837` — Test-only `unsafe` uses unguarded `std::env::set_var` in potentially parallel tests

Two `unsafe` blocks manipulate environment variables without a `Mutex` guard. `std::env::set_var` is not thread-safe when other threads concurrently call `std::env::var`. Under `cargo nextest` (which runs tests in parallel by default), this is a data race. Tests should either use a `Mutex<()>` guard or be marked `#[serial]`.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `crates/ui/web-api/src/auth/jwt.rs:38-61` — Legacy file-based `JwtManager::load_or_generate` writes signing key to disk without at-rest encryption

Although the controller has migrated to DB-based JWT key storage, the file-based method remains as a public API. After migration, the plaintext key persists on disk indefinitely. The file should be deleted after successful migration, and the method should be marked `#[deprecated]` or `pub(crate)`.

---

## Code Quality

### Strengths

- Zero `#[allow(dead_code)]` annotations across all 24 crates — dead code is removed, not suppressed.
- Zero `#[allow(clippy::...)]` suppressions in the entire workspace, consistent with the AGENTS.md invariant that no approved exceptions exist.
- `crates/core/controller/src/durations.rs`: All domain-significant durations (shutdown timeout, stale claim window, cert renewal threshold) are named constants with doc-comments. No magic numeric literals for time values in the controller.
- Uniform error propagation via `rootcause` (`bail!`, `report!`, `context_to`, `impl_report_conversion!`) with no `Report::new()` anti-pattern and no `Result<T, String>` in library boundaries.
- `crates/ui/web-api/src/lib.rs:98`: `CaKeyStore` `Debug` implementation manually redacts all key fields to `[REDACTED]` — secrets cannot leak via `{:?}` formatting, log macros, or `dbg!()`.
- `crates/ui/web-api/src/startup.rs`: Discrete startup phases use distinct typed structs (`ReconciledSettings`, `ValidatedConfig`, `PkiRuntime`) — partially initialized state cannot be passed to functions expecting fully initialized state.

### Issues

**[SEVERITY: High]** `crates/ui/web-api/src/routes/oidc_auth.rs:224-637` — `oidc_callback` is 413 lines with at least 7 levels of nesting

This single function mixes HTTP client construction, PKCE code exchange, ID token validation, claims extraction, registration checks, user resolution, role synchronization, and session creation. Cyclomatic complexity exceeds 10 by a substantial margin. The workspace-level concern is that OIDC is gated behind a feature flag, and all of this logic is untested at the unit level — the complexity makes it resistant to targeted unit testing. Decomposing into `exchange_code_for_token`, `validate_id_token`, `resolve_or_create_user`, and `create_session_and_redirect` would bring each function within the complexity budget and make the OIDC feature flag boundary auditable.

**[SEVERITY: Medium]** `crates/ui/web-api/src/routes/oidc_auth.rs:873-906` and `:1088-1124` — Identical 33-line role reverse-mapping block duplicated verbatim

Both `oidc_complete_registration` and `oidc_link` reconstruct "fake claims" using identical logic including the same `// first path segment only` limitation comment. A shared `fn build_claims_for_role_sync(provider, mapped_roles) -> serde_json::Value` would eliminate the duplication and ensure the limitation is fixed in one place.


#### 2026-02-24 Review

##### Strengths

- **Zero `#[allow(dead_code)]` annotations across all 24 crates.** No crate uses `#[allow(dead_code)]` to suppress warnings. All unused code has been removed.
- **Zero `#[allow(clippy::...)]` suppressions in the entire workspace.** All previously allowed lints have been resolved via parameter structs, `FromStr` implementations, or dead code removal.

##### Issues

---

## Tests

### Strengths

- `crates/shared/command/src/executor.rs:333,351`, `crates/shared/service-sdk/src/cert_handler.rs:385-411`, `crates/core/controller/src/scheduler/executors/ca_rotation_check.rs:103-139`: `#[tokio::test(start_paused = true)]` used correctly for all time-dependent tests — consistent with AGENTS.md invariant requiring virtual time.
- `crates/core/controller/tests/reverse_proxy/`: All Docker integration tests carry `#[ignore = "Docker integration test — requires Docker: ..."]` with exact runbook invocation commands, satisfying the AGENTS.md requirement for documented ignore reasons.
- In-process SQLite via SeaORM for database tests — full schema semantics, zero inter-test state leakage, no external process dependency for the majority of tests.
- Security-sensitive paths have targeted test coverage: JWT wrong-secret rejection, denylist revocation, OIDC state one-time-use, device-flow consumption, session double-approve, rate-limit window reset.
- `crates/ui/web-api/src/auth/rate_limit.rs` and `crates/shared/service-sdk/src/backoff.rs`: `MessageRateLimiter` and backoff logic tested via `MockApiServer` (`httpmock`) with typed endpoint builders — consistent mock pattern across all consumer crates.

### Issues

**[SEVERITY: High]** `crates/core/controller/src/scheduler/mod.rs:265` — Real `sleep(150ms)` in a non-paused scheduler cancellation test

`scheduler_run_exits_on_cancellation` uses `tokio::time::sleep(Duration::from_millis(150))` without `start_paused = true`. This burns real wall-clock time and is sensitive to scheduler latency under load. This directly violates the AGENTS.md invariant: "All time-dependent tests must use virtual time via `#[tokio::test(start_paused = true)]`." The exception for DB-connection tests does not apply here — no SeaORM pool is involved. The test should use `start_paused = true` with `tokio::time::advance`.

**[SEVERITY: High]** `crates/core/controller/tests/reverse_proxy/nginx_ocsp.rs:46,164,297` — Real `sleep(1 second)` inside Docker integration tests, fragile on slow CI hosts

Three test functions unconditionally sleep one second waiting for Nginx OCSP responder initialization. On a loaded CI host this window is insufficient; on a fast host it is wasteful. The correct pattern is a retry loop polling `/healthz` (or equivalent readiness probe) with a configurable timeout and a short poll interval, consistent with the restart-scattered and exponential-backoff patterns already present in the production code.

**[SEVERITY: Medium]** Workspace-wide — `test_state(db)` / `test_db()` / `NoopCertSigner` construction duplicated across test modules

Full `AppState` construction with `NoopCertSigner` is duplicated verbatim in at least `crates/ui/web-api/src/routes/auth.rs:458` and `crates/ui/web-api/src/middleware/require_auth.rs:202`. A shared `crates/ui/web-api/src/tests/helpers.rs` module exposing `test_app_state(db: DatabaseConnection) -> AppState` would eliminate duplication and ensure that new `AppState` fields added in the future are not silently missing in test variants.

**[SEVERITY: Medium]** `crates/ui/cli/`: CLI integration tests cover only `hosts`, `services`, `software_items` command groups

`auth`, `scheduler`, `plugin_configs`, `settings`, `autodiscovery`, `history`, `update`, and `check` command groups have no integration-level test coverage against `MockApiServer`. The AGENTS.md invariant requires covering success and failure paths for new logic; these commands have existing logic that has never been exercised at the integration level.

**[SEVERITY: Medium]** `crates/shared/openapi-client/src/lib.rs:687-885` — Retry-backoff tests do not verify delay durations

Eight backoff tests run with `start_paused = false` and assert only that operations eventually succeed. The actual delay values — the functional correctness of the backoff algorithm — are never asserted. Switching to `start_paused = true` and asserting the elapsed virtual time after each attempt would validate the exponential growth, jitter range, and cap behaviour.

**[SEVERITY: Medium]** Route handlers in `crates/ui/web-api/src/routes/` — Multiple handlers have no inline unit tests

`hosts.rs`, `agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`, `oidc_providers.rs`, `server_cert.rs`, `settings_auth.rs`, and `ocsp.rs` carry no `#[cfg(test)]` module. Given that Axum handlers can be tested cheaply via Tower `oneshot` calls with an in-memory SQLite state (as `auth.rs` already demonstrates), the gap is a coverage deficit rather than a testing infrastructure gap.

**[SEVERITY: Low]** `crates/core/controller/tests/reverse_proxy/nginx_ocsp.rs:424` — `reserve_port()` has a TOCTOU race

The helper binds a port, reads the port number, drops the listener, then starts the OCSP responder on that port. Between the drop and the bind there is a window where another process can claim the port. The correct pattern is to keep the listener alive and pass the bound `TcpListener` directly to the responder if the API supports it, or to use socket activation.

#### 2026-02-24 Review

##### Issues

**[SEVERITY: High]** Workspace-wide (273+ `#[tokio::test]` across 56 `.rs` files) — Systemic violation of `start_paused = true` invariant across nearly all async tests

Of 295 total `#[tokio::test]` annotations, only 9 across 4 files use `start_paused = true`. The remaining 273+ tests run with real wall-clock time. Even tests that appear time-insensitive today become flaky when future refactors introduce timeouts. Fixing requires a bulk annotation pass, ideally enforced by a CI grep gate.

**[SEVERITY: Medium]** Workspace-wide — No CI gate or lint enforces the `#[tokio::test(start_paused = true)]` invariant

Without a CI check, the invariant is documentation-only and will continue to be violated by new code.

---

## High Availability

### Strengths

- `crates/core/controller/src/scheduler/claim.rs:22-37`: `try_claim` uses a single `UPDATE WHERE locked_by IS NULL` — optimistic locking without a separate SELECT, TOCTOU-free.
- `crates/core/controller/src/scheduler/claim.rs:101-126`: `recover_stale_claims` reclaims tasks locked for more than 10 minutes, providing automatic crash recovery.
- `crates/shared/service-sdk/src/backoff.rs:29-43`: Exponential backoff with jitter (base 2 s, cap 60 s, ~25% jitter) used for WebSocket reconnection — prevents thundering-herd on controller restart.
- `crates/core/controller/src/tasks.rs:76-83`: `broadcast_server_restarting_scattered` spreads reconnect jitter over a configurable window — no synchronized stampede.
- `crates/ui/web-api/src/service_connections.rs:211-221`: `send()` acquires read lock, clones sender, drops lock before the async send — no lock held across an `.await` point.
- `crates/core/controller/src/tasks.rs:36-37`: `CancellationToken` child tokens propagated to all background tasks — clean cooperative shutdown.

### Issues

**[SEVERITY: High]** `crates/core/controller/src/scheduler/mod.rs:153` — Scheduler executes tasks sequentially with no timeout and no cancellation-point between tasks

`executor.execute(&task).await` is called with no timeout wrapper and no check of the `CancellationToken` during execution. A network-blocked `VersionCheckExecutor` or `ServiceCertCheckExecutor` will stall the entire scheduler poll cycle. Because the token is only checked in the outer `tokio::select!` between interval ticks, shutdown cannot interrupt a running task. The stale-claim recovery window (10 minutes) means a hung task blocks the entire scheduler for up to 10 minutes. Each task execution should be wrapped in `tokio::time::timeout` with a per-task budget, and the cancellation token should be passed into the executor so long-running tasks can cooperate with shutdown.

**[SEVERITY: High]** `crates/ui/web-api/src/mqtt_lease_coordinator.rs:533-576` — `reconcile_mqtt_clients` silently overwrites an active peer's MQTT lease

The reconciliation function updates `instance_id` and `heartbeat_at` without any notification to the previous holder. Two controller instances can both believe they hold the same MQTT lease simultaneously, producing duplicate MQTT connections, conflicting status update messages, and broken QoS guarantees. Reconciliation should only claim a lease whose heartbeat has exceeded the stale threshold and must update the holder atomically (e.g., `UPDATE WHERE instance_id = :previous_id AND heartbeat_at < :stale_threshold`).

**[SEVERITY: Medium]** `crates/core/mqtt/src/mqtt_client.rs:253-254` — MQTT reconnect uses a fixed 5-second delay with no backoff

Unlike WebSocket reconnection (which uses the `backoff.rs` exponential backoff with jitter), MQTT reconnection retries at a flat 5-second interval indefinitely. During an extended broker outage, all MQTT clients hammer the broker at 5-second intervals with no circuit-breaker. The same `backoff.rs` helper already used for WebSocket connections should be applied here for consistency.

**[SEVERITY: Medium]** `crates/core/controller/src/tasks.rs:98-104` — 5-second shutdown timeout may be insufficient for `release_all_claims` under a slow database

`BACKGROUND_TASK_SHUTDOWN_TIMEOUT = 5s` covers the `release_all_claims` DB write. Under a slow or temporarily unavailable database, the write is abandoned after 5 seconds, claims are not released, and the next controller instance must wait the full 10-minute stale recovery window before resuming scheduled work. The timeout should be configurable, or the `release_all_claims` call should carry its own shorter internal deadline with a logged warning on timeout.

**[SEVERITY: Medium]** `crates/shared/service-sdk/src/event_loop.rs:244-246` — `tick().await` inside `handle_service_settings` suspends the event loop

A new `Interval` is created and its first tick immediately consumed with `.await`. This suspends the event loop while waiting, blocking incoming WebSocket frame reads and ping responses. The initialization should be restructured so the interval is returned to the outer event-loop driver without any intermediate `.await` in the settings handler.

#### 2026-02-24 Review

##### Strengths

- **User-level token revocation uses latest-timestamp guard.** `crates/ui/web-api/src/auth/token_denylist.rs:59-66` — `deny_user` uses `if until > *entry` to prevent concurrent revocation calls from narrowing the window.
- **MQTT event delivery uses targeted routing via `mqtt_client_index`.** `crates/ui/web-api/src/event_poller.rs:262-305` — Routes tenant-specific messages to the specific MQTT service instance holding that client, avoiding unnecessary broadcasts.

---

## Database

### Strengths

- UUID v7 primary keys throughout all entities — time-ordered, index-friendly, no hot-spot write contention.
- `crates/shared/db/src/`: `TenantScoped` trait makes tenant-scoped queries structurally impossible to omit on typed query paths — tenant data leakage is a compile-time error, not a runtime risk.
- Partial (filtered) unique indexes on `software_items` (`WHERE deactivated_at IS NULL`) and `plugin_configs` — prevents name collisions among active records while allowing post-soft-delete re-creation.
- Every foreign key has an explicit `ON DELETE` action — no implicit cascade surprises.
- `m20260209_000001_initial.rs`: `down()` drops all tables in correct reverse FK order — migrations are fully reversible.
- Referential integrity CHECK constraint on `sessions`: `auth_method != 'oidc' OR oidc_provider_id IS NOT NULL` — enforced at the DB layer, not the application layer.
- Transactions used consistently for all multi-step mutations; `lock_exclusive()` acquired before `merge_service` to prevent concurrent modification.

### Issues

**[SEVERITY: Medium]** `crates/shared/db/src/entity/oidc_provider.rs:89` — Soft-delete column named `deleted_at` instead of `deactivated_at`

All other 7 soft-deletable entities (`tenants`, `users`, `hosts`, `services`, `software_items`, `plugin_configs`, `ca_certificates`) use `deactivated_at`. The `oidc_providers` entity uses `deleted_at`. This inconsistency prevents a generic soft-delete utility from working across all entities and confuses developers who expect a uniform column name. A migration renaming the column is the correct fix.

**[SEVERITY: Medium]** `crates/shared/db/src/entity/update_history.rs:28` — Dual output storage: `output` column and `update_output_lines` child table

`get_update_history` checks `if record.output.is_empty()` and conditionally loads from the child table. This dual-path design means a partially migrated database (records with neither `output` populated nor `update_output_lines` rows) would silently return empty output with no error. There is no DB constraint enforcing which storage path is canonical.

**[SEVERITY: Medium]** `crates/core/agent-ssh/src/db/entity/ssh_host.rs:72-73` — Timestamp columns stored as `INTEGER` epoch seconds instead of typed TIMESTAMP

`created_at` and `updated_at` are `i64` Unix seconds rather than `time::OffsetDateTime` columns used by every other entity. DB tooling (migrations, generic timestamp queries, `TenantScoped` helpers) will silently miss these columns, and sub-second precision is lost. Aligning with the rest of the codebase requires a migration on the SSH agent's local SQLite database.

**[SEVERITY: Low]** Missing indexes identified across multiple tables: `update_history` has no index on `created_at` despite `ORDER BY created_at DESC` in the list query; `host_software_items` has no index on `software_item_id` alone (the composite PK starts with `host_id`); `mqtt_leases` has no index on `tenant_id`; `sessions` has no composite `(user_id, expires_at)` index for active-session lookups. `tenants.slug` and `users.email` each have both a `string_uniq()` SeaORM constraint and an explicit `Index::create()` call — duplicate indexes waste write overhead.

**[SEVERITY: Low]** `api_tokens` table — No `expires_at` column

API tokens have no expiry mechanism. A forgotten token remains valid indefinitely. At minimum, an optional `expires_at` column should be supported; for security-sensitive infrastructure access, a mandatory maximum TTL should be enforced.

#### 2026-02-24 Review

##### Strengths

- **CHECK constraint on sessions table enforces OIDC provider requirement at DB level.** `crates/core/controller/src/migration/m20260209_000001_initial.rs:474-478` — `CHECK(auth_method != 'oidc' OR oidc_provider_id IS NOT NULL)`.
- **`TenantDb` extractor provides compile-time tenant filtering.** `crates/ui/web-api/src/tenant_db.rs:29-51` — Covers SELECT, UPDATE, and DELETE operations uniformly.

##### Issues

**[SEVERITY: Medium]** `crates/core/controller/src/migration/m20260209_000001_initial.rs:615-621` — Raw SQL in migration seed uses `CURRENT_TIMESTAMP` which behaves differently across backends

The `settings_version` seed uses `execute_unprepared` with `CURRENT_TIMESTAMP`. Should be converted to use the query builder with `Expr::current_timestamp()` for backend-agnostic behavior.

**[SEVERITY: Low]** `crates/core/controller/src/migration/m20260209_000001_initial.rs:1942-1994` — Scheduled task seeding uses loop without batch insert

Issues N_tenants x 6 individual INSERT statements. Should use a single multi-row INSERT for consistency.

---

## Coding Standards

### Strengths

- All 24 crates use `edition = "2024"` inherited from `[workspace.package]` — no edition fragmentation.
- `SecretString` used consistently at all HTTP API input and output boundaries across `uptrakit-web-api-types` — compliant with AGENTS.md invariant 6 (no secrets in logs).
- `bail!` / `report!` / `context_to` / `impl_report_conversion!` pattern is uniform; `Report::new()` anti-pattern is absent; `Result<T, String>` is absent from all library boundaries.
- No `StatusCode` numeric literal comparisons anywhere in the codebase — all comparisons use `StatusCode::*` variants and `.is_*()` helpers, consistent with AGENTS.md invariant 17.
- `FromStr` pattern is textbook-correct throughout: typed `Parse{TypeName}Error`, `impl FromStr`, and `s.parse::<MyType>()` at call sites (`AlertSeverity`, `ParseAlertSeverityError` in `system_alerts.rs`).
- 70+ endpoints carry `x-required-permission` OpenAPI extension annotations with values that match `Permission::as_str()` serialization — auditable authorization coverage by static inspection.

### Issues

**[SEVERITY: High]** `crates/ui/web-api/src/routes/api_tokens.rs:19-31` and `crates/ui/web-api/src/routes/auth.rs:339,666` — User-identity endpoints missing `x-required-permission` OpenAPI extension

`create_api_token`, `list_api_tokens`, `revoke_api_token`, `logout`, and `me` use `Extension<AuthenticatedUser>` directly (appropriate for user-scoped endpoints not governed by the RBAC permission model), but none carry a `x-required-permission` annotation. AGENTS.md invariant 18 requires every protected endpoint to carry the matching annotation. User-identity endpoints should carry a documented sentinel value (e.g., `"authenticated"`) so the OpenAPI spec is consistent and automated permission-audit tooling does not treat them as unprotected.

**[SEVERITY: Medium]** `crates/ui/web-api/src/routes/hosts.rs:141`, `services.rs:282`, `services.rs:483` — Three `DELETE` endpoints return `200 OK` with a body

Soft-delete endpoints that use the `DELETE` HTTP method should return `204 No Content`. Returning `200 OK` with a response body on a `DELETE` is inconsistent with the REST conventions applied elsewhere in the API (other delete endpoints correctly return `204`) and violates the principle of least surprise for API consumers. The fix is either to return `204` with no body, or to rename the endpoint to `POST /{id}/deactivate` if a body is semantically necessary.

**[SEVERITY: Medium]** `crates/ui/web-api/src/routes/autodiscovery.rs:154,159` — `create_autodiscovery_ignore` returns `201 Created` for pre-existing records

The idempotent upsert path returns `201` regardless of whether a new row was inserted or an existing row was found. Standard REST convention: return `201 Created` for newly created resources, `200 OK` for pre-existing ones. The handler should distinguish between insert and no-op outcomes and set the status code accordingly.

#### 2026-02-24 Review

##### Strengths

- **Zero `anyhow` usage and zero `Result<T, String>` in library boundaries.** All 24 crates use `rootcause`/`thiserror` consistently.
- **All `Mutex::lock().unwrap()` usages comply with the approved exception.** 4 occurrences across 2 files, all on `Mutex::lock()`.

##### Issues

**[SEVERITY: Medium]** `crates/shared/web-api-types/src/permissions.rs:9` — `Permission` enum lacks `#[non_exhaustive]`

The enum has 9 variants and will grow with new features. Adding `#[non_exhaustive]` now is non-breaking; deferring forces simultaneous downstream updates.

**[SEVERITY: Low]** 7 public enums in `uptrakit-web-api-types` lack `#[non_exhaustive]`

`AlertSeverity`, `TriggerUpdateStatus`, `UpdateStatus`, `RegistrationMode`, `SystemdAction`, `DockerComposeAction`, `PredefinedHook` are all public enums without `#[non_exhaustive]` that could gain variants.

---

## Extensibility

### Strengths

- `crates/plugins/infrastructure/core/src/traits.rs:22-98`: `Plugin` trait uses default implementations for all optional methods — adding a new plugin requires implementing only the methods relevant to its capability set.
- `crates/plugins/infrastructure/registry/src/registry.rs:43-135`: `register_plugins!` macro generates all dispatch boilerplate — adding a new plugin type is a one-line change in the macro invocation plus a dependency entry.
- `crates/shared/wire/src/lib.rs:32` and `crates/shared/types/src/plugin_types.rs:17`: `#[non_exhaustive]` on `PluginCapability`, `ServiceMessage`, `ControllerMessage`, and related wire enums — downstream consumers cannot write exhaustive matches that break on new variants.
- `crates/shared/wire/src/lib.rs:71`: `Capability::Other(String)` forward-compatibility catch-all — unknown capabilities from newer peers are preserved and excluded from intersection without causing a deserialization error.
- `crates/shared/wire/src/close_reason.rs:49`: `CloseReason::Unknown(String)` mirrors the same forward-compat pattern for close reason codes.
- `crates/shared/service-sdk/src/lifecycle.rs:76-160`: `ServiceHandler` trait externalizes the entire service-specific surface — new service roles plug in without modifying the SDK lifecycle machinery.
- `crates/plugins/infrastructure/core/src/secrets.rs:9-17`: `SecretMasking` trait with no-op defaults — plugins that have no secrets do not need to implement masking.

### Issues

**[SEVERITY: Medium]** `crates/plugins/infrastructure/registry/src/registry.rs:151-156` — Platform-specific plugins compiled unconditionally into all agent binaries

`HomebrewPlugin` (macOS-only) and `ProxmoxHelperScriptsPlugin` (Proxmox-VE-only) are compiled into every agent binary regardless of target platform. A Linux agent will accept a `HomebrewPlugin` configuration and fail only when the `brew` executable is absent at runtime. Platform-specific plugins should be gated with `#[cfg(target_os = "macos")]` or behind optional Cargo features in `uptrakit-plugin-infrastructure-registry`, with the macro conditionally including them.

**[SEVERITY: Medium]** `crates/shared/wire/src/lib.rs:214-234` — `ServiceMessage` and `ControllerMessage` mix agent-specific and MQTT-specific variants

MQTT-specific wire variants are deserializable on agent connections and vice versa. Implementors of `ServiceHandler` must mentally classify each variant to understand which are relevant to their capability set. A per-capability message type (achieved through enums that each contain only the variants relevant to that capability) or a discriminated wrapper would make the extension surface explicit for new service roles.

**[SEVERITY: Medium]** `crates/shared/service-sdk/src/lifecycle.rs:79,89` — `ServiceHandler` is not object-safe due to associated constants; no documentation or `where Self: Sized` guards

`DIR_NAME` and `SERVICE_LABEL` are `const` items on the trait. Object safety requires that no associated constants be present without a `where Self: Sized` bound. If anyone attempts `Box<dyn ServiceHandler>` or `Arc<dyn ServiceHandler>`, the compiler error is non-obvious. The trait should either document that it is not intended as a trait object, or add `where Self: Sized` to the constant items to produce a clear diagnostic.

**[RESOLVED]** ~~`EnrollPayload.service_type` deprecation is undocumented in code~~

The `ServiceType` enum and `EnrollPayload.service_type` field have been removed. Service identity is now determined by `BTreeSet<Capability>` advertised during enrollment and capability negotiation. The controller infers the service role from the agreed capability set. Enrollment uses a single `register()` call (replacing the former `register_agent`/`register_mqtt`/`register_ssh_agent` triple), and routing uses `broadcast_by_capability` instead of the former `broadcast_by_type`.

#### 2026-02-24 Review

No workspace-level extensibility findings. All extensibility findings are in crate-specific files.
