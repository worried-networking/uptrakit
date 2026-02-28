# Code Review: Workspace (Root)

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

The Uptrakit backend (~112K LoC, 31 crates) is a well-structured Rust workspace implementing an
agent-based update tracking toolkit. The codebase demonstrates mature Rust engineering practices:
workspace-level `clippy::all = "deny"` and `warnings = "deny"` lints, consistent error handling
via `rootcause` + `thiserror` + `impl_report_conversion!`, and strong type-system enforcement of
security invariants (`EncryptedString`, `SecretString`, `TenantScoped`).

The dependency graph is a clean DAG with no circular dependencies. Feature flags are used
judiciously for database backends, OIDC, NATS, and embedded components. The plugin system is
well-designed with a macro-generated registry. The service SDK provides an excellent abstraction
for the enrollment-lifecycle-reconnect flow shared across all service binaries.

Key areas for improvement: the `web-api` crate at ~32K LoC is approaching "god crate" territory
and would benefit from decomposition; the wire protocol file (`wire/src/lib.rs`) at 3.5K lines
should be split into domain modules; the systemic `#[tokio::test]` violation (273+ tests without
`start_paused = true`) should be addressed with a bulk annotation pass; and several remaining HA
concerns (in-memory-only token denylist, blocking Mutex in rate limiter) should be addressed
before multi-instance deployment. The previously-reported SSRF in Docker auth realm, the OIDC
privilege escalation via `unwrap_or(0)`, all `#[cfg(not(feature))]` violations, the unbounded
MQTT event channel, and the scheduler claim leak on cancellation have been fixed. The OIDC
registration DB error masking (`unwrap_or(1)`), `count_linked_hosts` DB error swallowing,
`Report::new()` macro violations, invalid UUID query parameter handling, HTTP status code
violations on soft-delete and idempotent-create endpoints, and the `require_auth.rs`
permission-fetch fallback have since been fixed.

## Per-Crate Review Files

| Crate | Review |
| --- | --- |
| `crates/core/controller` | [CODEREVIEW.md](crates/core/controller/CODEREVIEW.md) |
| `crates/core/agent-ssh` | [CODEREVIEW.md](crates/core/agent-ssh/CODEREVIEW.md) |
| `crates/core/agent` | [CODEREVIEW.md](crates/core/agent/CODEREVIEW.md) |
| `crates/core/mqtt` | [CODEREVIEW.md](crates/core/mqtt/CODEREVIEW.md) |
| `crates/core/scheduler` | [CODEREVIEW.md](crates/core/scheduler/CODEREVIEW.md) |
| `crates/shared` (umbrella: macros, directories, build-info, update-hooks) | [CODEREVIEW.md](crates/shared/CODEREVIEW.md) |
| `crates/shared/agent-core` | [CODEREVIEW.md](crates/shared/agent-core/CODEREVIEW.md) |
| `crates/shared/command` | [CODEREVIEW.md](crates/shared/command/CODEREVIEW.md) |
| `crates/shared/crypto` | [CODEREVIEW.md](crates/shared/crypto/CODEREVIEW.md) |
| `crates/shared/db` | [CODEREVIEW.md](crates/shared/db/CODEREVIEW.md) |
| `crates/shared/nats` | [CODEREVIEW.md](crates/shared/nats/CODEREVIEW.md) |
| `crates/shared/openapi-client` | [CODEREVIEW.md](crates/shared/openapi-client/CODEREVIEW.md) |
| `crates/shared/scheduler-engine` | [CODEREVIEW.md](crates/shared/scheduler-engine/CODEREVIEW.md) |
| `crates/shared/service-sdk` | [CODEREVIEW.md](crates/shared/service-sdk/CODEREVIEW.md) |
| `crates/shared/types` | [CODEREVIEW.md](crates/shared/types/CODEREVIEW.md) |
| `crates/shared/web-api-types` | [CODEREVIEW.md](crates/shared/web-api-types/CODEREVIEW.md) |
| `crates/shared/wire` | [CODEREVIEW.md](crates/shared/wire/CODEREVIEW.md) |
| `crates/ui/web-api` | [CODEREVIEW.md](crates/ui/web-api/CODEREVIEW.md) |
| `crates/ui/cli` | [CODEREVIEW.md](crates/ui/cli/CODEREVIEW.md) |
| `crates/plugins` (umbrella: generic/shell + cross-cutting) | [CODEREVIEW.md](crates/plugins/CODEREVIEW.md) |
| `crates/plugins/infrastructure/core` | [CODEREVIEW.md](crates/plugins/infrastructure/core/CODEREVIEW.md) |
| `crates/plugins/infrastructure/registry` | [CODEREVIEW.md](crates/plugins/infrastructure/registry/CODEREVIEW.md) |
| `crates/plugins/releases/docker` | [CODEREVIEW.md](crates/plugins/releases/docker/CODEREVIEW.md) |
| `crates/plugins/releases/github` | [CODEREVIEW.md](crates/plugins/releases/github/CODEREVIEW.md) |
| `crates/plugins/package-managers/apt` | [CODEREVIEW.md](crates/plugins/package-managers/apt/CODEREVIEW.md) |
| `crates/plugins/package-managers/npm` | [CODEREVIEW.md](crates/plugins/package-managers/npm/CODEREVIEW.md) |
| `crates/plugins/package-managers/homebrew` | [CODEREVIEW.md](crates/plugins/package-managers/homebrew/CODEREVIEW.md) |
| `crates/plugins/discovery/proxmox-helper-scripts` | [CODEREVIEW.md](crates/plugins/discovery/proxmox-helper-scripts/CODEREVIEW.md) |

## Architecture

### Strengths

- `Cargo.toml` (workspace root) -- `resolver = "3"` and `edition = "2024"` set once; all 31
  crates inherit via `edition.workspace` — no per-crate drift possible.
- `Cargo.toml:13-60` (`[workspace.dependencies]`) -- All major external dependencies pinned here
  with exact version ranges and default-feature overrides. Per-crate declarations only specify
  `features = [...]`, eliminating duplicate version declarations.
- `Cargo.toml:62-67` (`[profile.release]`) -- `lto = "fat"`, `codegen-units = 1`,
  `panic = "abort"`, `strip = true` — production-hardened single-binary output.
- `[workspace.package]` carries `license`, `authors`, `repository`, and `version`.
- Clean DAG dependency graph flowing from leaf types through shared libraries to core binaries.
  The four-domain layout (`core/`, `plugins/`, `shared/`, `ui/`) enforces a natural dependency
  gradient.
- Feature flags compose correctly: `db-sqlite`/`db-postgres`/`db-mysql` for backends, `oidc` for
  OIDC support, `nats` for NATS transport, `embedded-scheduler` for in-process scheduling.
- `ServiceHandler` trait in `service-sdk` is the architectural linchpin enabling agent, MQTT,
  SSH agent, and scheduler to share enrollment/lifecycle logic with minimal per-service code.
- `TenantScoped` marker trait (`db/src/entity/tenant_scoped.rs:9-16`) enables `TenantDb`
  extractor to automatically apply tenant filtering, eliminating an entire class of isolation
  bugs. Compile-time tenant filtering; tenant data leakage is structurally impossible through
  typed paths.
- Plugin system uses trait-based dispatch with `register_plugins!` macro that generates all six
  dispatch methods from a single declaration.
- `[workspace.lints]` enforces `warnings = "deny"` and `clippy::all = "deny"` across all 31
  crates via `[lints] workspace = true`.
- UUID v7 primary keys throughout all entities -- time-ordered, index-friendly, no hot-spot
  write contention.
- Partial (filtered) unique indexes on `software_items` and `plugin_configs`
  (`WHERE deactivated_at IS NULL`) -- prevents name collisions among active records while
  allowing post-soft-delete re-creation.
- Every foreign key has an explicit `ON DELETE` action -- no implicit cascade surprises.
- `m20260209_000001_initial.rs` -- `down()` drops all tables in correct reverse FK order.
- Referential integrity CHECK constraint on `sessions`:
  `auth_method != 'oidc' OR oidc_provider_id IS NOT NULL` -- enforced at DB level.
- Transactions used consistently for all multi-step mutations; `lock_exclusive()` acquired
  before `merge_service`.
- Batch plugin config loading in `list_ignore_rules` and JOIN-based `load_plugins` eliminate
  N+1 patterns.

### Issues

**[CRITICAL]** `crates/ui/web-api/src/settings_store.rs:13` and
`crates/core/controller/src/startup.rs:164-197` -- Settings persistence logic lives in the UI
layer (`web-api/settings_store.rs`) but is called from the core binary (`controller/startup.rs`).
This creates a logical layering inversion where the controller depends on web-api internals.

**[HIGH]** `crates/ui/web-api/src/lib.rs:1-24` -- At ~32K LoC, `web-api` is the largest crate
(~29% of total). Contains authentication, authorization, middleware, routes, queries, settings,
MQTT coordination, NATS transport, OCSP, PKI, notifications, and update output broadcasting.

**[HIGH]** `crates/core/controller/Cargo.toml:22-60` -- Controller depends directly on
`uptrakit-web-api` (a UI crate), coupling the core binary to the entire web-api surface.

**[HIGH]** `crates/shared/wire/src/lib.rs` -- At 3,524 lines in a single file, the entire wire
protocol definition lives in one module. Should be decomposed into domain modules.

**[HIGH]** `crates/ui/web-api/src/app_state.rs:37-96` -- `AppState` has 22+ public fields, most
with `pub` visibility.

**[MEDIUM]** `crates/shared/db/src/entity/oidc_provider.rs:89` -- Soft-delete column named
`deleted_at` instead of `deactivated_at`. All other 7 soft-deletable entities use
`deactivated_at`. Inconsistency prevents generic soft-delete utility.

**[MEDIUM]** `crates/shared/db/src/entity/update_history.rs:28` -- Dual output storage:
`output` column and `update_output_lines` child table. No DB constraint enforcing which storage
path is canonical.

**[MEDIUM]** `crates/ui/web-api/src/queries/autodiscovery.rs:36-75` -- TOCTOU race in
`create_or_ignore_ignore_rule`. Check-then-insert without transaction.

**[MEDIUM]** `crates/ui/web-api/src/queries/update_history.rs:148-150` -- Output not loaded for
`list_update_history`. Returns empty output for records using newer `update_output_lines`
storage.

**[MEDIUM]** `crates/core/controller/src/migration/m20260209_000001_initial.rs:615-621` -- Raw
SQL in migration seed uses `CURRENT_TIMESTAMP` which behaves differently across backends.

**[LOW]** `Cargo.toml` (workspace root) -- No `rust-version` MSRV declared. Without an MSRV
declaration, edition-2024 features may silently break older toolchains.

**[LOW]** `api_tokens` table -- No `expires_at` column. API tokens valid indefinitely once
issued.

## Security and Safety

### Strengths

- AES-256-GCM with random 96-bit nonces, master key in `OnceLock<Zeroizing<[u8; 32]>>`,
  `EncryptedString` with redacted Debug/Display, and `ENC:v1:` prefix for format versioning.
- Argon2id with OWASP parameters (19 MiB, 2 iterations) for password and enrollment token
  hashing.
- JWT with HS256, 15-minute expiry, required `iss`/`aud` claims, and explicit legacy rejection.
- Refresh token rotation in DB transactions with replay detection,
  HttpOnly/Secure/SameSite=Strict cookies.
- mTLS with ECDSA P-256 and `aws-lc-rs` provider. File permissions 0o700/0o600 for secrets.
- OIDC: PKCE (S256), single-use CSRF state with 10-minute TTL, email verification enforcement,
  encrypted PKCE verifiers at rest, Referrer-Policy on redirects.
- Shell escape uses POSIX single-quote wrapping. Direct exec mode bypasses shell entirely.
  `kill_on_drop(true)` and 10 MB output limit on commands.
- Proxy headers stripped from non-proxy requests. Certificate issuer CN verified against known
  CAs.
- No `unsafe` code outside of test utilities (2 instances in `directories` for env var tests).
- No secret values found in tracing calls across the entire codebase.
- `permission_extractor!` macro generates typed Axum extractors for all 9 permission levels.
  Authorization auditable by function signature alone.
- `SecretString` at all API input and output boundaries across `web-api-types`.
- CA private keys stored AES-256-GCM encrypted in DB and held as `Zeroizing<String>`.

### Issues

**[HIGH]** `crates/ui/web-api/src/auth/token_denylist.rs:14-16` -- Token denylist is in-memory
and per-instance only. Revoked tokens valid on other instances for up to 15 minutes (JWT TTL).

**[HIGH]** `crates/ui/web-api/src/routes/service_ws/mod.rs:145-157` -- WebSocket
`lookup_by_secret` queries without tenant filter.

**[MEDIUM]** `crates/core/controller/src/startup.rs:61-75` -- Master key hex returned as plain
`String`, not wrapped in `Zeroizing<String>`.

**[MEDIUM]** `crates/shared/service-sdk/src/tls.rs` -- TOFU TLS verifier accepts any server
certificate during initial CA fetch. MITM window during initial enrollment.

**[MEDIUM]** `crates/ui/web-api/src/routes/oidc_auth.rs:349-352` -- OIDC registration code
exposed in redirect URL query parameter.

**[MEDIUM]** `crates/shared/crypto/src/lib.rs` -- `PLAINTEXT_MODE` uses `Ordering::Relaxed`.
Use `Ordering::Release` / `Ordering::Acquire` for correctness.

**[MEDIUM]** `crates/ui/web-api/src/auth/jwt.rs:38-61` -- Legacy file-based
`JwtManager::load_or_generate` writes signing key to disk without at-rest encryption. After
migration to DB-based storage, the plaintext key persists on disk. File should be deleted after
migration and method marked `#[deprecated]` or `pub(crate)`.

**[LOW]** `crates/shared/directories/src/lib.rs:829,837` -- Test-only `unsafe` uses unguarded
`std::env::set_var` in potentially parallel tests. Data race under `cargo nextest`.

## Code Quality

### Strengths

- Consistent `rootcause` + `thiserror` + `impl_report_conversion!` error handling across all 31
  crates. Every crate with errors defines a custom error type with `#[derive(Debug, Error)]`.
- 2,363 test functions across 217 files with meaningful assertions and realistic scenarios.
- Zero `#[allow(dead_code)]` annotations. Zero `#[allow(clippy::...)]` suppressions.
- 76% of files have doc comments. All public traits, errors, and wire protocol types documented.
- `Zeroizing` wrapper used consistently for key material in the crypto crate.
- `CaKeyStore` `Debug` implementation manually redacts all key fields to `[REDACTED]` -- verified
  by dedicated test.
- Discrete startup phases use distinct typed structs (`ReconciledSettings`, `ValidatedConfig`,
  `PkiRuntime`).
- All domain-significant durations centralized in `durations.rs` with doc-comments.
- `#[tokio::test(start_paused = true)]` used correctly for all time-dependent tests in
  controller executors, command executor, and cert handler.
- Docker integration tests carry `#[ignore = "Docker integration test..."]` with exact runbook
  invocation commands.
- In-process SQLite via SeaORM for database tests -- full schema semantics, no external process.
- Security-sensitive paths have targeted test coverage: JWT wrong-secret rejection, denylist
  revocation, OIDC state one-time-use, device-flow consumption, session double-approve.
- Mock server infrastructure (`openapi-client/src/mock.rs`) well-designed for integration tests.

### Issues

**[HIGH]** Workspace-wide (273+ `#[tokio::test]` across 56 `.rs` files) -- Systemic violation
of `start_paused = true` invariant. Of 295 total `#[tokio::test]` annotations, only 9 across 4
files use `start_paused = true`. Even tests that appear time-insensitive today become flaky when
future refactors introduce timeouts. Requires bulk annotation pass and CI grep gate.

**[HIGH]** `crates/ui/cli/src/main.rs` -- At 3,870 lines (including ~1,500 lines of tests),
this is the largest single file.

**[HIGH]** Route handlers in `crates/ui/web-api/src/routes/` -- Multiple handlers have no
inline unit tests. `hosts.rs`, `agents.rs`, `settings_ca.rs`, `settings_mqtt.rs`,
`oidc_providers.rs`, `server_cert.rs`, `settings_auth.rs`, and `ocsp.rs` carry no
`#[cfg(test)]` module.

**[MEDIUM]** No CI gate or lint enforces the `#[tokio::test(start_paused = true)]` invariant.

**[MEDIUM]** `crates/ui/web-api/src/routes/software_items.rs:98,102` and
`crates/shared/scheduler-engine/src/executors/version_check.rs:44,51` -- `NoopCommandExecutor`
duplicated with `unreachable!()` in two locations.

**[MEDIUM]** `.expect()` on guarded values in `agent-ssh/src/commands/update_sudoers.rs:72,114`
is logically safe but fragile to refactoring.

**[MEDIUM]** `test_state(db)` / `test_db()` / `NoopCertSigner` construction duplicated across
17+ test modules. Shared `test_helpers` module would eliminate duplication.

**[MEDIUM]** `crates/ui/cli/tests/command_execution.rs` -- Only `hosts`, `services`, and
`software_items` namespaces covered by integration tests.

**[MEDIUM]** `crates/shared/openapi-client/src/lib.rs:687-885` -- Retry-backoff tests do not
verify delay durations. Eight tests assert only eventual success, not backoff timing.

## High Availability

### Strengths

- `CancellationToken` used for cooperative shutdown across all services. `BackgroundTasks` in
  controller provides named handles with both cooperative and forceful shutdown modes.
- `ServerRestarting` notifications scattered over 5 seconds to prevent thundering herd.
- Exponential backoff with jitter in `service-sdk/src/backoff.rs` prevents thundering herd on
  reconnect. All services use this consistently.
- Optimistic locking via `UPDATE ... WHERE locked_by IS NULL` in scheduler prevents double
  execution across instances. Stale claim recovery (10-minute threshold) handles crashes.
- MQTT client has LWT for offline status, bounded per-service channels (capacity 32), and
  connection deduplication in `ServiceConnectionRegistry`.
- Settings use `watch` channel for atomic snapshot publishing with serialized writes.
- `send()` acquires read lock, clones sender, drops lock before async send -- no lock held
  across await points.
- Event poller advances cursor only past successfully delivered events. `MAX_DELIVERY_RETRIES`
  (3) prevents single bad event from blocking all delivery.
- User-level token revocation uses latest-timestamp guard (`if until > *entry`) to prevent
  concurrent revocation from narrowing the window.
- MQTT event delivery uses targeted routing via `mqtt_client_index` -- routes tenant-specific
  messages to specific MQTT service instance.

### Issues

**[CRITICAL]** `crates/ui/web-api/src/middleware/rate_limit.rs:114-115` -- Fallback rate limiter
uses `std::sync::Mutex` (blocking) in async context.

**[HIGH]** `crates/core/controller/src/tasks.rs:105-111` -- Per-task shutdown timeout is only 5
seconds. The embedded scheduler may need longer for in-progress DB operations.

**[HIGH]** `crates/ui/web-api/src/service_connections.rs:192-203` -- `broadcast` awaits send to
each service sequentially. Single slow consumer delays broadcasts to all services.

**[HIGH]** `crates/shared/nats/src/connection.rs:24-28` -- No NATS retry at startup. If NATS is
temporarily unavailable, controller fails to start entirely.

**[MEDIUM]** `crates/core/controller/src/tasks.rs:98-104` -- 5-second shutdown timeout may be
insufficient for `release_all_claims` under slow database.

**[MEDIUM]** `crates/shared/service-sdk/src/event_loop.rs:244-246` -- `tick().await` inside
`handle_service_settings` suspends the event loop, blocking incoming WebSocket reads and pings.

## Coding Standards

### Strengths

- All 31 crates use `workspace = true` for shared dependencies and `[lints] workspace = true`.
- `TenantDb` wrapper used consistently across all tenant-scoped web-api routes.
- All string-to-type conversions use `FromStr` with dedicated `Parse{Type}Error` types.
- No N+1 query patterns found. Batch loading with `is_in(ids)` and `HashMap` lookups used.
- Database-backed rate limiting with atomic SQL upsert, HA-safe across controller instances.
- Structured tracing with proper levels. No sensitive data in log output.
- `bail!` / `report!` / `context_to` / `impl_report_conversion!` pattern uniform; `Report::new()`
  anti-pattern absent across all 31 crates; `Result<T, String>` absent from all library
  boundaries.
- No `StatusCode` numeric literal comparisons anywhere in the codebase.
- 70+ endpoints carry `x-required-permission` OpenAPI extension annotations.
- Zero `anyhow` usage across all 31 crates.

### Issues

**[MEDIUM]** `crates/shared/web-api-types/src/permissions.rs:9` -- `Permission` enum lacks
`#[non_exhaustive]`.

## Extensibility

### Strengths

- Plugin system: trait with default implementations, capability-based feature declaration,
  macro-generated registry dispatch, JSON configuration with `SecretMasking`. Adding a new plugin
  requires one line in `register_plugins!`.
- Wire protocol: `Other(String)` catch-all on `PluginType`, `PluginRole`, `Capability`, and
  `CloseReason` for forward-compatible deserialization.
- `ServiceHandler` trait uses associated type `ServiceEvent` (defaulting to `Infallible`),
  associated constants, and default implementations for clean extensibility.
- Feature gates on `shared-types` (`sea-orm`, `openapi`), `shared-db` (`migration`,
  `db-sqlite/postgres/mysql`), and `plugin-registry` (`daemon`, `ssh`).
- Migrations follow date-stamped naming with `up()`/`down()` methods. JSON columns for
  schema-free configuration evolution.
- `SecretMasking` trait with no-op defaults -- plugins without secrets need no masking code.
- Sequence validation decoupled from full deserialization enables forward-compatible message
  handling.

### Issues

**[HIGH]** `crates/shared/wire/src/lib.rs:254-276` -- `ServiceMessage` and `ControllerMessage`
use `#[serde(tag = "type")]` without a catch-all variant. Unknown message types from a newer peer
cause deserialization errors and are irreversibly lost.

**[MEDIUM]** 13 public enums across `shared-types`, `web-api-types`, `wire`, `command`, and
`plugin-infrastructure-core` lack `#[non_exhaustive]`: `DeviceAuthStatus`, `ServiceStatus`,
`SoftwareDiscoveryState`, `OutputStreamType`, `MqttClientConnectionStatus`, `Permission`,
`UpdateStatus`, `PluginError`, `ScheduledTaskType`, `HookCommand`, `CommandMode`,
`MqttTransport`, `RegistrationMode`.

**[MEDIUM]** `crates/plugins/infrastructure/registry/src/registry.rs:257-279` --
`validate_package_identifier` manually maintained outside the `register_plugins!` macro.

**[MEDIUM]** `crates/shared/wire/src/lib.rs:214-234` -- `ServiceMessage` and
`ControllerMessage` mix agent-specific and MQTT-specific variants.

**[MEDIUM]** `crates/shared/service-sdk/src/lifecycle.rs:79,89` -- `ServiceHandler` is not
object-safe due to associated constants. No documentation or `where Self: Sized` guards.

**[LOW]** No explicit wire protocol version number. A `protocol_version: u8` in `EnrollPayload`
would provide a safety valve for hard protocol breaks.

**[LOW]** 7 public enums in `uptrakit-web-api-types` lack `#[non_exhaustive]`:
`AlertSeverity`, `TriggerUpdateStatus`, `UpdateStatus`, `RegistrationMode`, `SystemdAction`,
`DockerComposeAction`, `PredefinedHook`.
