# CODEREVIEW — Workspace

## Summary

The Uptrakit backend is a well-structured Rust workspace of 24 crates spanning four clearly separated domains: `core/` (binaries), `providers/` (pluggable detection and update drivers), `shared/` (libraries), and `ui/` (HTTP API and CLI). The codebase consistently applies Rust 2024 edition and resolver version 3 across every crate, uses workspace-pinned dependency versions for all major libraries, and leans on strong type-system patterns — typed permission extractors, `SecretString` at API boundaries, `Zeroizing<>` on key material — that enforce security invariants at compile time rather than by convention. The release profile is production-hardened (`lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`), and the overall dependency DAG is sound with one well-defined layering violation.

The primary architectural concern at workspace level is that `uptrakit-crypto`, a foundational crate that any agent-side consumer will eventually need, unconditionally pulls in `sea-orm`. This couples a cryptographic primitive to the ORM layer and contradicts the clean split that `uptrakit-shared-types` already achieves through its `sea-orm` feature flag. A related concern is that `sea-orm-migration` is declared inline in two crates at an RC version, creating version-drift risk between it and the workspace-pinned `sea-orm` during the RC series. Beyond dependency hygiene, the workspace has no `[workspace.lints]` table, so only one of the 24 crates (`uptrakit-internal-wire`) actually enforces `warnings = "deny"` and `clippy::all = "deny"` — the remaining 23 rely solely on CI configuration.

Several high-severity operational issues span the workspace. The `AGENTS.md` invariant document references a `crates/shared/core/` (`uptrakit-core`) crate that does not exist, which misleads any agent or developer reading the canonical layout map. The child-process orphan problem in `uptrakit-command` is a documented gap that affects every binary that runs updates — orphaned `apt`, `brew`, or custom script processes hold system locks until manually killed. And the mTLS configuration in `uptrakit-controller` uses `.allow_unauthenticated()`, so the transport-layer PKI boundary that mTLS is meant to provide is not actually enforced; agent trust rests entirely on application-layer checks.

The extensibility seam for providers is well-designed at the `Provider` trait and `register_providers!` macro level but fractures in three places outside that seam: `ProviderType::supports_discovery()` in `uptrakit-shared-types` duplicates capability knowledge that `Provider::capabilities()` already owns; `create_provider_for_discovery` in the registry bypasses the macro entirely; and package-identifier validation is split between the provider `validate()` method and raw string comparisons in a query helper. Any new discovery-capable provider must touch all three locations and will get a runtime error, not a compile error, if any is missed. Addressing these cross-cutting concerns would raise the baseline from a solid prototype-quality codebase to one ready for production multi-tenant operation.

---

## Architecture

### Strengths

- `Cargo.toml` (workspace root): `resolver = "3"` and `edition = "2024"` are set once at workspace root; all 24 crates inherit via `edition.workspace` — no per-crate drift possible.
- `Cargo.toml:13-60` (`[workspace.dependencies]`): All major external dependencies — `tokio`, `axum`, `sea-orm`, `rustls`, `serde`, `time`, `aws-lc-rs`, `rcgen`, `uuid`, `rand`, and 25+ more — are pinned here with exact version ranges and default-feature overrides. Per-crate declarations only specify `features = [...]`, eliminating duplicate version declarations across 24 crates.
- `Cargo.toml:62-67` (`[profile.release]`): `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, `strip = true` — production-hardened single-binary output with minimal attack surface.
- `[workspace.package]` carries `license`, `authors`, `repository`, and `version` so every crate can use `*.workspace = true`, ensuring metadata consistency across all published artifacts.
- The four-domain layout (`core/`, `providers/`, `shared/`, `ui/`) enforces a natural dependency gradient: providers never import from `ui/`, binaries import from `shared/` and `ui/`, `shared/` libraries only import each other in a defined order. This makes cross-cutting concerns auditable.
- `uptrakit-shared-types/Cargo.toml:9-13`: `sea-orm` and `openapi` features correctly gate optional ORM and OpenAPI dependencies — the correct pattern for workspace-wide shared type crates.
- `uptrakit-internal-wire/Cargo.toml:21-25`: The only crate that enforces `[lints.rust] warnings = "deny"` and `[lints.clippy] all = "deny"` locally, demonstrating the correct pattern for the rest of the workspace to adopt via `[workspace.lints]`.

### Issues

**[SEVERITY: High]** `crates/shared/crypto/Cargo.toml:14` — `uptrakit-crypto` unconditionally depends on `sea-orm`

`sea-orm = { workspace = true }` is a non-optional production dependency of what should be a foundational cryptographic primitive. The sole reason is to implement `sea_orm::sea_query::ValueType` and `TryGetable` for `EncryptedString`. Every future consumer of `uptrakit-crypto` — including any agent-side or tooling crate — will transitively compile all of SeaORM's async runtime machinery. `uptrakit-shared-types` already demonstrates the correct pattern: `sea-orm = { workspace = true, optional = true }` behind a `sea-orm` feature flag. `uptrakit-crypto` should adopt the same approach, with the `ValueType`/`TryGetable` impls gated behind an opt-in `sea-orm` feature. `uptrakit-shared-db` — the only crate that actually needs both crypto and ORM — would then enable `uptrakit-crypto/sea-orm`.

**[SEVERITY: High]** `AGENTS.md:72` — Canonical codebase layout map references a non-existent crate

`crates/shared/core/` is listed as `uptrakit-core (lib) — shared domain models` in the layout tree. No such directory or crate exists in the workspace (confirmed: `members = ["crates/*/*"]` resolves 24 crates, none named `uptrakit-core`). AGENTS.md is the primary reference document for AI coding agents and new contributors. A ghost entry in the layout map causes agents to look for `uptrakit-core` as a dependency target, confuses cross-crate import planning, and silently invalidates every document section that refers to the layout map as authoritative.

**[SEVERITY: Medium]** `crates/core/controller/Cargo.toml:45` and `crates/core/agent-ssh/Cargo.toml:37` — `sea-orm-migration` declared inline, not in `[workspace.dependencies]`

Both crates pin `sea-orm-migration = { version = "2.0.0-rc.32", ... }` inline and independently. The workspace pins `sea-orm = { version = "2.0.0-rc.32", ... }`. During an RC series, patch versions of `sea-orm` and `sea-orm-migration` must match exactly. With two separate inline declarations there is no single place to update both simultaneously: a `dependabot` or manual bump of one will not automatically bump the other, and the mismatch will produce a compile error or silent behavioral difference. Both should be moved to `[workspace.dependencies]` with the same version and feature baseline.

**[SEVERITY: Medium]** `Cargo.toml` (workspace root) — No `[workspace.lints]` table; lint enforcement covers only 1 of 24 crates

Only `uptrakit-internal-wire` declares `[lints.rust] warnings = "deny"` and `[lints.clippy] all = "deny"`. The other 23 crates accumulate warnings silently outside CI. AGENTS.md invariant 13 prohibits `#[allow()]` additions without approval, but without enforced deny-by-default, violations can accumulate undetected between CI runs. Adding a `[workspace.lints]` table to the root `Cargo.toml` would propagate the same policy to all crates via `lints.workspace = true`, consistent with how `edition`, `license`, and `version` are already inherited.

**[SEVERITY: Medium]** `crates/ui/web-api/Cargo.toml:22-23` and `crates/core/controller/Cargo.toml:50-51` — Dual datetime crates (`time` + `chrono`) introduced by `cron`, not in workspace

`time = "0.3"` is workspace-pinned and used as the canonical datetime type throughout all entities, wire types, and database code. `chrono = { version = "0.4" }` and `cron = "0.15"` are added inline in both `uptrakit-web-api` and `uptrakit-controller` because the `cron` crate requires `chrono`. Neither `chrono` nor `cron` appear in `[workspace.dependencies]`. Two independent inline declarations create the same patch-drift risk as `sea-orm-migration`. Additionally, the dual datetime crates make clock-source inconsistencies possible: code using `chrono::Utc::now()` versus `time::OffsetDateTime::now_utc()` may behave differently at DST boundaries or on hosts with non-UTC local time.

**[SEVERITY: Medium]** `crates/shared/service-sdk/Cargo.toml:34` — `tracing-subscriber` is a production dependency of a shared library

`tracing-subscriber = { workspace = true }` appears in `[dependencies]`, not `[dev-dependencies]`. The subscriber initialization call in `service-sdk/src/main_helper.rs` (`tracing_subscriber::fmt().init()`) configures the global tracing dispatcher. A library must never configure the global dispatcher — that is the binary's responsibility. Any crate that calls `init_tracing()` twice (e.g., an integration test that imports two binaries) will panic. The call should be moved to each binary's `main.rs`, and `tracing-subscriber` moved to `[dev-dependencies]` in `uptrakit-service-sdk`. The binary crates (`uptrakit-controller`, `uptrakit-agent`, `uptrakit-agent-ssh`, `uptrakit-mqtt`) all already have `tracing-subscriber = { workspace = true }` in their own `[dependencies]`, so the move is purely a removal from the library.

**[SEVERITY: Low]** `Cargo.toml` (workspace root) — No `rust-version` MSRV declared

`AGENTS.md:109` states "Some specify `rust-version = \"1.91\"`", but zero crates in the workspace actually set `rust-version`. The workspace `[workspace.package]` section has no `rust-version` field. Without an MSRV declaration, `cargo check` on any Rust version will succeed, and edition-2024 features may silently break older toolchains used by downstream packagers or CI matrix jobs. The correct fix is a single `rust-version = "1.85"` (or whichever minimum is validated) in `[workspace.package]` and updating AGENTS.md to reflect the actual state.

**[SEVERITY: Low]** `crates/ui/cli/Cargo.toml:30` and `crates/shared/wire/Cargo.toml` (dev) — `serde_yaml_ng` and `rumqttc` not in `[workspace.dependencies]`

`serde_yaml_ng = "0.10"` appears inline in `uptrakit-cli` (production) and in `uptrakit-internal-wire` dev-dependencies. `rumqttc = { version = "0.25.1" }` appears inline in `uptrakit-mqtt`. As sole consumers, neither creates immediate drift risk, but the inconsistency with the otherwise comprehensive workspace dependency table makes auditing harder. Both should be promoted to `[workspace.dependencies]` for uniformity.

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

**[SEVERITY: High]** `crates/ui/web-api/src/auth/authentication.rs:115` — OIDC `email_verified` claim silently discarded

The `resolve_oidc_user` function pattern-matches `email_verified: _`, ignoring the field entirely. A misconfigured or malicious OIDC provider can assert an unverified email address, which the controller will accept for account creation or login matching. This is a cross-cutting invariant failure: the OIDC authentication path in `crates/ui/web-api/src/routes/oidc_auth.rs` retrieves the claim value but `resolve_oidc_user` never consults it. The fix must be applied at the workspace-visible boundary: reject the authentication if `email_verified` is `Some(false)` and document the `None` treatment (either accept or reject, but do so explicitly) in `docs/security/auth-and-authorization.md`.

**[SEVERITY: High]** `crates/core/controller/src/pki.rs:1169` and `crates/ui/web-api/src/routes/server_cert.rs:199` — mTLS configured with `.allow_unauthenticated()`

`WebPkiClientVerifier::builder(...).allow_unauthenticated()` permits TLS sessions from clients that present no certificate. The agent-controller security model documented in AGENTS.md depends on mTLS as the transport-layer trust anchor; the architecture invariant "agents authenticate via client certificates issued by the controller CA" is not enforced at the TLS layer. Application-layer checks (machine-ID validation, enrollment state) remain intact, but transport-layer PKI enforcement is absent. The correct builder terminal for requiring client certificates is `.build()` without `.allow_unauthenticated()`. This is a cross-workspace concern because it affects the security guarantee described in `docs/security/pki-certificates.md` and `docs/security/tofu-tls.md`.

**[SEVERITY: Medium]** `crates/shared/crypto/src/lib.rs:244-251` — `EncryptedString::new` silently degrades to plaintext when master key is absent

When `MASTER_KEY` is not initialised, `EncryptedString::new` logs `tracing::warn!` and stores the plaintext value. There is no startup guard or environment assertion that prevents a production deployment from running in this mode. OIDC client secrets, provider API keys, and CA private key material would all be stored cleartext in the database with no machine-readable indicator beyond a transient log line. This is a workspace-level policy concern: `uptrakit-service-sdk` initializes the application lifecycle, and the master-key check belongs there as a hard startup failure, not a soft warning in the crypto library.

**[SEVERITY: Medium]** `crates/ui/web-api/src/auth/jwt.rs:101` — JWT validation uses `Validation::default()` with no audience claim

No `aud` claim is required during token validation. A JWT issued for one deployment environment will be accepted by another if the HMAC signing key is shared or accidentally reused. There is also no `iss` or `sub` format validation. This is a workspace-level concern because the JWT signing key file path and generation are handled in `uptrakit-service-sdk` and `uptrakit-directories` — the key lifecycle spans crates, so the validation policy must be defined and enforced consistently.

**[SEVERITY: Medium]** `crates/core/controller/src/pki.rs:497` — CA certificate generated with `BasicConstraints::Unconstrained`

The controller CA is issued without a path-length constraint, which permits a compromised agent certificate to be used as an intermediate CA to sign further certificates. The correct value is `BasicConstraints::Constrained(0)` (path length zero), preventing issuance of any subordinate CA. This constraint should also be verified in the test suite at `crates/core/controller/tests/`.

**[SEVERITY: Medium]** `crates/ui/web-api/src/auth/token_denylist.rs:15` — In-memory JWT denylist provides no cross-instance revocation

The denylist is per-process. A revoked token remains valid on all other running instances for up to the JWT TTL (15 minutes). The code comment at line 15 acknowledges this. For a system managing infrastructure access with potential privilege escalation via compromised tokens, a 15-minute window is operationally significant. A shared DB-backed denylist — consistent with the existing refresh token and rate-limit tables — would close this gap. This is a workspace-level HA and security intersection: the multi-instance architecture described in `docs/development/cross-controller-comm.md` is undermined by per-process revocation state.

**[SEVERITY: Low]** `crates/shared/directories/src/lib.rs:829,837` — Test-only `unsafe` uses unguarded `std::env::set_var` in potentially parallel tests

Two `unsafe` blocks manipulate environment variables without a `Mutex` guard. `std::env::set_var` is not thread-safe when other threads concurrently call `std::env::var`. Under `cargo nextest` (which runs tests in parallel by default), this is a data race. Tests should either use a `Mutex<()>` guard or be marked `#[serial]`.

---

## Code Quality

### Strengths

- Zero `#[allow(dead_code)]` annotations across all 24 crates — dead code is removed, not suppressed.
- Only one `#[allow(clippy::...)]` in the entire workspace (`crates/ui/web-api/src/queries/autodiscovery.rs:554`), consistent with the AGENTS.md invariant that no approved exceptions exist.
- `crates/core/controller/src/durations.rs`: All domain-significant durations (shutdown timeout, stale claim window, cert renewal threshold) are named constants with doc-comments. No magic numeric literals for time values in the controller.
- Uniform error propagation via `rootcause` (`bail!`, `report!`, `context_to`, `impl_report_conversion!`) with no `Report::new()` anti-pattern and no `Result<T, String>` in library boundaries.
- `crates/ui/web-api/src/lib.rs:98`: `CaKeyStore` `Debug` implementation manually redacts all key fields to `[REDACTED]` — secrets cannot leak via `{:?}` formatting, log macros, or `dbg!()`.
- `crates/ui/web-api/src/startup.rs`: Discrete startup phases use distinct typed structs (`ReconciledSettings`, `ValidatedConfig`, `PkiRuntime`) — partially initialized state cannot be passed to functions expecting fully initialized state.

### Issues

**[SEVERITY: High]** `crates/ui/web-api/src/routes/agent_ws.rs:453` — Wildcard arm on `UpdateFinalStatus` match silently maps all future variants to `Failed`

`UpdateFinalStatus::Failed | _ => UpdateStatus::Failed` — any new variant added to `UpdateFinalStatus` will silently map to `Failed` with no compile-time warning. This is a workspace-level concern because `UpdateFinalStatus` is defined in `uptrakit-internal-wire`, and the wire crate is the intended extension point for new status codes. Adding a new status variant passes compilation without triggering any exhaustiveness error in the consumer. The wildcard should be removed; the match should be exhaustive.

**[SEVERITY: High]** `crates/ui/web-api/src/routes/service_ws.rs:610-614` — Magic constant `120` repeated inline for shutdown timeout; wildcard arm masks future service types

The value `120` (seconds) appears three times inline in proximity to a `_ => Some(120)` wildcard arm. Domain-significant timeout constants should be in `crates/core/controller/src/durations.rs`, and the wildcard should be replaced with exhaustive matching so that a new `ServiceType` variant does not silently inherit an arbitrary timeout. This violates the same principle as the `agent_ws.rs` finding above: `ServiceType` is a wire-protocol extension point in `uptrakit-internal-wire`.

**[SEVERITY: High]** `crates/ui/web-api/src/routes/oidc_auth.rs:224-637` — `oidc_callback` is 413 lines with at least 7 levels of nesting

This single function mixes HTTP client construction, PKCE code exchange, ID token validation, claims extraction, registration checks, user resolution, role synchronization, and session creation. Cyclomatic complexity exceeds 10 by a substantial margin. The workspace-level concern is that OIDC is gated behind a feature flag, and all of this logic is untested at the unit level — the complexity makes it resistant to targeted unit testing. Decomposing into `exchange_code_for_token`, `validate_id_token`, `resolve_or_create_user`, and `create_session_and_redirect` would bring each function within the complexity budget and make the OIDC feature flag boundary auditable.

**[SEVERITY: Medium]** `crates/ui/web-api/src/routes/oidc_auth.rs:873-906` and `:1088-1124` — Identical 33-line role reverse-mapping block duplicated verbatim

Both `oidc_complete_registration` and `oidc_link` reconstruct "fake claims" using identical logic including the same `// first path segment only` limitation comment. A shared `fn build_claims_for_role_sync(provider, mapped_roles) -> serde_json::Value` would eliminate the duplication and ensure the limitation is fixed in one place.

**[SEVERITY: Medium]** Multiple route files — Pervasive `Path<String>` + manual `uuid::Uuid::parse_str` pattern (43 occurrences)

Axum's extractor system supports `Path(id): Path<Uuid>` directly, returning a typed 422 on malformed input. The manual `parse_str` pattern produces inconsistent error responses (varies by handler), increases per-function complexity, and duplicates UUID validation logic. Key files: `crates/ui/web-api/src/routes/hosts.rs:77`, `services.rs:98`, `software_items.rs:146`, `api_tokens.rs:111`, `provider_configs.rs:106`. This is a workspace-wide pattern problem touching at least 10 route files.

**[SEVERITY: Medium]** `crates/shared/service-sdk/src/main_helper.rs:37-38,42-43` — `expect()` calls in a shared library outside approved exception list

`expect()` is used to unwrap results in `init_tracing()`. AGENTS.md lists approved `unwrap()` exceptions as `Mutex::lock()`, `RwLock::read()`, and `RwLock::write()` only. These `expect()` calls do not qualify. In addition, since `init_tracing()` must be removed from the library entirely (see Architecture issue above), this is a compound fix.

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

`auth`, `scheduler`, `provider_configs`, `settings`, `autodiscovery`, `history`, `update`, and `check` command groups have no integration-level test coverage against `MockApiServer`. The AGENTS.md invariant requires covering success and failure paths for new logic; these commands have existing logic that has never been exercised at the integration level.

**[SEVERITY: Medium]** `crates/shared/openapi-client/src/lib.rs:687-885` — Retry-backoff tests do not verify delay durations

Eight backoff tests run with `start_paused = false` and assert only that operations eventually succeed. The actual delay values — the functional correctness of the backoff algorithm — are never asserted. Switching to `start_paused = true` and asserting the elapsed virtual time after each attempt would validate the exponential growth, jitter range, and cap behaviour.

**[SEVERITY: Medium]** Route handlers in `crates/ui/web-api/src/routes/` — Multiple handlers have no inline unit tests

`hosts.rs`, `agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`, `oidc_providers.rs`, `server_cert.rs`, `settings_auth.rs`, and `ocsp.rs` carry no `#[cfg(test)]` module. Given that Axum handlers can be tested cheaply via Tower `oneshot` calls with an in-memory SQLite state (as `auth.rs` already demonstrates), the gap is a coverage deficit rather than a testing infrastructure gap.

**[SEVERITY: Low]** `crates/core/controller/tests/reverse_proxy/nginx_ocsp.rs:424` — `reserve_port()` has a TOCTOU race

The helper binds a port, reads the port number, drops the listener, then starts the OCSP responder on that port. Between the drop and the bind there is a window where another process can claim the port. The correct pattern is to keep the listener alive and pass the bound `TcpListener` directly to the responder if the API supports it, or to use socket activation.

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

**[SEVERITY: High]** `crates/ui/web-api/src/mqtt_client_store.rs:235-263` — `update_mqtt_clients_status` issues one `SELECT + UPDATE` per MQTT client in a serial loop without a wrapping transaction

Partial failure (DB error mid-loop) leaves the `mqtt_clients` table in an inconsistent state — some clients marked Offline, others not. The `?` early return propagates the first error without processing remaining clients. This should be a single bulk `UPDATE ... WHERE id IN (...)` or wrapped in a transaction that either commits all status changes or none.

**[SEVERITY: High]** `crates/core/controller/src/tasks.rs:254-256` — CRL manager task registered with `track_abort` instead of `track`

`track_abort` unconditionally aborts the task handle without first cancelling the token and waiting for clean exit. If the CRL manager is mid-write to the TLS configuration file when aborted, the file is left partially written. A partially written TLS config causes a startup failure on the next process start. The CRL manager should be registered with `track` so it receives the cancellation signal and can complete or roll back its current write before exiting.

**[SEVERITY: High]** `crates/ui/web-api/src/mqtt_lease_coordinator.rs:533-576` — `reconcile_mqtt_clients` silently overwrites an active peer's MQTT lease

The reconciliation function updates `instance_id` and `heartbeat_at` without any notification to the previous holder. Two controller instances can both believe they hold the same MQTT lease simultaneously, producing duplicate MQTT connections, conflicting status update messages, and broken QoS guarantees. Reconciliation should only claim a lease whose heartbeat has exceeded the stale threshold and must update the holder atomically (e.g., `UPDATE WHERE instance_id = :previous_id AND heartbeat_at < :stale_threshold`).

**[SEVERITY: Medium]** `crates/core/mqtt/src/mqtt_client.rs:253-254` — MQTT reconnect uses a fixed 5-second delay with no backoff

Unlike WebSocket reconnection (which uses the `backoff.rs` exponential backoff with jitter), MQTT reconnection retries at a flat 5-second interval indefinitely. During an extended broker outage, all MQTT clients hammer the broker at 5-second intervals with no circuit-breaker. The same `backoff.rs` helper already used for WebSocket connections should be applied here for consistency.

**[SEVERITY: Medium]** `crates/core/controller/src/tasks.rs:98-104` — 5-second shutdown timeout may be insufficient for `release_all_claims` under a slow database

`BACKGROUND_TASK_SHUTDOWN_TIMEOUT = 5s` covers the `release_all_claims` DB write. Under a slow or temporarily unavailable database, the write is abandoned after 5 seconds, claims are not released, and the next controller instance must wait the full 10-minute stale recovery window before resuming scheduled work. The timeout should be configurable, or the `release_all_claims` call should carry its own shorter internal deadline with a logged warning on timeout.

**[SEVERITY: Medium]** `crates/shared/service-sdk/src/event_loop.rs:244-246` — `tick().await` inside `handle_service_settings` suspends the event loop

A new `Interval` is created and its first tick immediately consumed with `.await`. This suspends the event loop while waiting, blocking incoming WebSocket frame reads and ping responses. The initialization should be restructured so the interval is returned to the outer event-loop driver without any intermediate `.await` in the settings handler.

---

## Database

### Strengths

- UUID v7 primary keys throughout all entities — time-ordered, index-friendly, no hot-spot write contention.
- `crates/shared/db/src/`: `TenantScoped` trait makes tenant-scoped queries structurally impossible to omit on typed query paths — tenant data leakage is a compile-time error, not a runtime risk.
- Partial (filtered) unique indexes on `software_items` (`WHERE deactivated_at IS NULL`) and `provider_configs` — prevents name collisions among active records while allowing post-soft-delete re-creation.
- Every foreign key has an explicit `ON DELETE` action — no implicit cascade surprises.
- `m20260209_000001_initial.rs`: `down()` drops all tables in correct reverse FK order — migrations are fully reversible.
- Referential integrity CHECK constraint on `sessions`: `auth_method != 'oidc' OR oidc_provider_id IS NOT NULL` — enforced at the DB layer, not the application layer.
- Transactions used consistently for all multi-step mutations; `lock_exclusive()` acquired before `merge_service` to prevent concurrent modification.

### Issues

**[SEVERITY: High]** `crates/ui/web-api/src/queries/update_history.rs:78-83` — Full host table scan for tenant scoping in `tenant_host_ids()`

The helper loads all host rows for the tenant into application memory and passes them as an `IN (...)` clause. For tenants with many hosts, this degrades non-linearly, risks hitting driver-level parameter count limits, and forces the DB to build a large ephemeral set for each query that calls `tenant_host_ids()`. The fix is to replace the in-memory ID collection with a JOIN or a correlated subquery that keeps filtering inside the DB.

**[SEVERITY: High]** `crates/ui/web-api/src/queries/software_items.rs:126-178` — N+1 query pattern in `load_item_hosts`

For N host assignments: 1 query to load links, then N individual `find_by_id(host_id)` calls plus N individual `find_by_id(provider_config_id)` calls = 1 + 2N round trips. `load_item_hosts` is called from `get_software_item`, `assign_hosts`, and `update_host_assignment`. A single JOIN across `host_software_items`, `hosts`, and `provider_configs` would reduce this to one query regardless of N.

**[SEVERITY: High]** `crates/ui/web-api/src/queries/update_history.rs:146-151` — N+1 in `list_update_history` for a paginated list

For a page of 20 records: `resolve_host_name` and `resolve_software_item_name` each issue an individual lookup per record, producing up to 40 extra round trips per page. Both should be batched using `Column::Id.is_in(ids)` before the main loop.

**[SEVERITY: High]** `crates/ui/web-api/src/queries/autodiscovery.rs:567-580` — Ignore-list check issued inside a per-item loop

`process_one_discovery` issues a `COUNT(*)` query for every discovered software item. For a host with 200 installed packages, this is 200 sequential DB queries. The ignore list should be bulk-loaded once before the discovery loop and checked in memory.

**[SEVERITY: Medium]** `crates/shared/db/src/entity/oidc_provider.rs:89` — Soft-delete column named `deleted_at` instead of `deactivated_at`

All other 7 soft-deletable entities (`tenants`, `users`, `hosts`, `services`, `software_items`, `provider_configs`, `ca_certificates`) use `deactivated_at`. The `oidc_providers` entity uses `deleted_at`. This inconsistency prevents a generic soft-delete utility from working across all entities and confuses developers who expect a uniform column name. A migration renaming the column is the correct fix.

**[SEVERITY: Medium]** `crates/shared/db/src/entity/update_history.rs:28` — Dual output storage: `output` column and `update_output_lines` child table

`get_update_history` checks `if record.output.is_empty()` and conditionally loads from the child table. This dual-path design means a partially migrated database (records with neither `output` populated nor `update_output_lines` rows) would silently return empty output with no error. There is no DB constraint enforcing which storage path is canonical.

**[SEVERITY: Medium]** `crates/core/agent-ssh/src/db/entity/ssh_host.rs:72-73` — Timestamp columns stored as `INTEGER` epoch seconds instead of typed TIMESTAMP

`created_at` and `updated_at` are `i64` Unix seconds rather than `time::OffsetDateTime` columns used by every other entity. DB tooling (migrations, generic timestamp queries, `TenantScoped` helpers) will silently miss these columns, and sub-second precision is lost. Aligning with the rest of the codebase requires a migration on the SSH agent's local SQLite database.

**[SEVERITY: Low]** Missing indexes identified across multiple tables: `update_history` has no index on `created_at` despite `ORDER BY created_at DESC` in the list query; `host_software_items` has no index on `software_item_id` alone (the composite PK starts with `host_id`); `mqtt_leases` has no index on `tenant_id`; `sessions` has no composite `(user_id, expires_at)` index for active-session lookups. `tenants.slug` and `users.email` each have both a `string_uniq()` SeaORM constraint and an explicit `Index::create()` call — duplicate indexes waste write overhead.

**[SEVERITY: Low]** `api_tokens` table — No `expires_at` column

API tokens have no expiry mechanism. A forgotten token remains valid indefinitely. At minimum, an optional `expires_at` column should be supported; for security-sensitive infrastructure access, a mandatory maximum TTL should be enforced.

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

**[SEVERITY: High]** `crates/ui/web-api/src/queries/autodiscovery.rs:554` — `#[allow(clippy::too_many_arguments)]` violates AGENTS.md invariant 13

AGENTS.md states: "There are currently no approved exceptions in the codebase; all previously allowed lints have been resolved via parameter structs, `FromStr` implementations, or dead code removal." This single remaining suppression must be resolved. The prescribed fix is to introduce a `DiscoveryEntry<'a>` struct grouping the `package_identifier`, `name`, `installed_version`, and `provider_type_str` arguments, reducing the parameter count below Clippy's threshold without changing behaviour.

**[SEVERITY: High]** `crates/ui/web-api/src/routes/api_tokens.rs:19-31` and `crates/ui/web-api/src/routes/auth.rs:339,666` — User-identity endpoints missing `x-required-permission` OpenAPI extension

`create_api_token`, `list_api_tokens`, `revoke_api_token`, `logout`, and `me` use `Extension<AuthenticatedUser>` directly (appropriate for user-scoped endpoints not governed by the RBAC permission model), but none carry a `x-required-permission` annotation. AGENTS.md invariant 18 requires every protected endpoint to carry the matching annotation. User-identity endpoints should carry a documented sentinel value (e.g., `"authenticated"`) so the OpenAPI spec is consistent and automated permission-audit tooling does not treat them as unprotected.

**[SEVERITY: Medium]** Multiple route files — 43 `Path<String>` + manual `uuid::Uuid::parse_str` occurrences violate the `FromStr` invariant

AGENTS.md invariant 14 requires `FromStr` for all string-to-type conversions. Axum's `Path<Uuid>` extractor uses `FromStr<Uuid>` internally and produces a typed 422 on malformed input. The manual `parse_str` pattern is a partial re-implementation of what the framework already provides, produces inconsistent error response shapes, and violates the spirit of the standard conversion pattern. This affects at least 10 route files across `crates/ui/web-api/src/routes/`.

**[SEVERITY: Medium]** `crates/ui/web-api/src/routes/hosts.rs:141`, `services.rs:282`, `services.rs:483` — Three `DELETE` endpoints return `200 OK` with a body

Soft-delete endpoints that use the `DELETE` HTTP method should return `204 No Content`. Returning `200 OK` with a response body on a `DELETE` is inconsistent with the REST conventions applied elsewhere in the API (other delete endpoints correctly return `204`) and violates the principle of least surprise for API consumers. The fix is either to return `204` with no body, or to rename the endpoint to `POST /{id}/deactivate` if a body is semantically necessary.

**[SEVERITY: Medium]** `crates/ui/web-api/src/routes/autodiscovery.rs:154,159` — `create_autodiscovery_ignore` returns `201 Created` for pre-existing records

The idempotent upsert path returns `201` regardless of whether a new row was inserted or an existing row was found. Standard REST convention: return `201 Created` for newly created resources, `200 OK` for pre-existing ones. The handler should distinguish between insert and no-op outcomes and set the status code accordingly.

**[SEVERITY: Low]** All `#[utoipa::path]` annotations with UUID path parameters declare `String` type instead of `Uuid` — 43 occurrences

The OpenAPI schema emitted by `utoipa` for these parameters will declare `type: string` without `format: uuid`. API clients and OpenAPI linters use `format: uuid` to enable UUID validation, code generation, and documentation accuracy. Each annotation should use `schema(value_type = Uuid)` or the equivalent `utoipa` attribute for the path parameter.

---

## Extensibility

### Strengths

- `crates/providers/core/src/traits.rs:22-98`: `Provider` trait uses default implementations for all optional methods — adding a new provider requires implementing only the methods relevant to its capability set.
- `crates/providers/registry/src/registry.rs:43-135`: `register_providers!` macro generates all dispatch boilerplate — adding a new provider type is a one-line change in the macro invocation plus a dependency entry.
- `crates/shared/wire/src/lib.rs:32` and `crates/shared/types/src/provider_types.rs:17`: `#[non_exhaustive]` on `ProviderCapability`, `ServiceMessage`, `ControllerMessage`, and related wire enums — downstream consumers cannot write exhaustive matches that break on new variants.
- `crates/shared/wire/src/lib.rs:71`: `Capability::Other(String)` forward-compatibility catch-all — unknown capabilities from newer peers are preserved and excluded from intersection without causing a deserialization error.
- `crates/shared/wire/src/close_reason.rs:49`: `CloseReason::Unknown(String)` mirrors the same forward-compat pattern for close reason codes.
- `crates/shared/service-sdk/src/lifecycle.rs:76-160`: `ServiceHandler` trait externalizes the entire service-specific surface — new service types plug in without modifying the SDK lifecycle machinery.
- `crates/providers/core/src/secrets.rs:9-17`: `SecretMasking` trait with no-op defaults — providers that have no secrets do not need to implement masking.

### Issues

**[SEVERITY: High]** `crates/shared/types/src/provider_types.rs:34-36` — `supports_discovery()` hardcodes discovery capability; duplicates `Provider::capabilities()` in a different crate

`ProviderType::supports_discovery()` uses a `matches!(self, Self::Homebrew | Self::ProxmoxHelperScripts)` literal. The authoritative source of capability knowledge is `Provider::capabilities()` in each provider crate. Adding a new discovery-capable provider requires changes in three places: the provider crate (add `ProviderCapability::SoftwareDiscovery` to `capabilities()`), the `ProviderType` enum in `uptrakit-shared-types` (update `supports_discovery()`), and a hardcoded slice in `crates/ui/web-api/src/routes/agent_ws.rs:1217-1220`. Missing any one of these produces a silent runtime failure — discovery assignments are not sent to the new provider. The fix is to drive the discovery filter from a capability flag queried via the registry, eliminating `supports_discovery()` as a separate concern.

**[SEVERITY: High]** `crates/providers/registry/src/registry.rs:165-204` — `create_provider_for_discovery` is a 40-line manual dispatch block that bypasses the `register_providers!` macro

This method exists because `create_provider` calls `validate()`, which requires a populated config, while discovery uses empty configs. The `_ =>` catch-all produces a runtime error for any provider not explicitly handled — a new provider added to the macro but not to this method will only fail at runtime when a discovery assignment is received. The gap should be closed either by adding a `validate: bool` parameter to the macro-generated creation path or by introducing a `Provider::create_for_discovery()` trait method with a default that delegates to `create_provider` with an empty config.

**[SEVERITY: High]** `crates/ui/web-api/src/queries/software_items.rs:329-332` — Package identifier validation uses raw string comparison against `"homebrew"`

`config.provider_type == "homebrew"` bypasses the `ProviderType` enum entirely. This is provider-specific business logic placed in the query layer, not the provider layer. A new provider with custom identifier constraints must add a branch here and in `validate()`, with no compile-time guidance that the query layer needs updating. The fix is to add a `fn validate_package_identifier(&self, value: &str) -> Result<()>` method to the `Provider` trait, implement it per-provider, and call it through the registry from the query helper.

**[SEVERITY: Medium]** `crates/providers/registry/src/registry.rs:151-156` — Platform-specific providers compiled unconditionally into all agent binaries

`HomebrewProvider` (macOS-only) and `ProxmoxHelperScriptsProvider` (Proxmox-VE-only) are compiled into every agent binary regardless of target platform. A Linux agent will accept a `HomebrewProvider` configuration and fail only when the `brew` executable is absent at runtime. Platform-specific providers should be gated with `#[cfg(target_os = "macos")]` or behind optional Cargo features in `uptrakit-provider-registry`, with the macro conditionally including them.

**[SEVERITY: Medium]** `crates/shared/wire/src/lib.rs:214-234` — `ServiceMessage` and `ControllerMessage` mix agent-specific and MQTT-specific variants

MQTT-specific wire variants are deserializable on agent connections and vice versa. Implementors of `ServiceHandler` must mentally classify each variant to understand which are relevant to their service type. A per-service message type (achieved through enums that each contain only the variants relevant to that service) or a discriminated wrapper would make the extension surface explicit for new service types.

**[SEVERITY: Medium]** `crates/shared/service-sdk/src/lifecycle.rs:79,89` — `ServiceHandler` is not object-safe due to associated constants; no documentation or `where Self: Sized` guards

`DIR_NAME`, `SERVICE_LABEL`, and `SERVICE_TYPE` are `const` items on the trait. Object safety requires that no associated constants be present without a `where Self: Sized` bound. If anyone attempts `Box<dyn ServiceHandler>` or `Arc<dyn ServiceHandler>`, the compiler error is non-obvious. The trait should either document that it is not intended as a trait object, or add `where Self: Sized` to the constant items to produce a clear diagnostic.

**[SEVERITY: Low]** `crates/shared/wire/src/lib.rs:316-318` — `EnrollPayload.service_type` deprecation is undocumented in code

The comment states service type will eventually be inferred from capabilities, but there is no `#[deprecated]` attribute, no tracking issue reference, and no compiler warning. Contributors adding new service types will not be guided toward the capability-based path. Adding `#[deprecated = "service_type will be inferred from capabilities; see EnrollPayload docs"]` with a tracking issue reference makes the migration intent visible at compile time.
