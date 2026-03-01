# Code Review: uptrakit-controller

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-controller` is the most complex binary in the workspace (~6K LoC). It orchestrates
database migrations, a 10-phase startup sequence, full PKI lifecycle management (CA generation,
CRL signing, OCSP support), a DB-backed HA scheduler, and a suite of background tasks. The
overall design is solid: startup phases are typed structs, the scheduler uses TOCTOU-free
optimistic locking, and the PKI layer is well-covered by 30+ unit tests. The `durations.rs`
module centralizes all timing constants with doc-comments, eliminating magic numbers throughout.

The main concerns are the 5-second background task shutdown timeout (too short for the embedded
scheduler), a direct dependency on the UI-layer `uptrakit-web-api` crate, and the hardcoded
database connection pool configuration. The PKI module at 1,646 lines could benefit from
sub-module extraction.

## Architecture

### Strengths

- `src/main.rs:114-499` -- 10-phase startup with typed intermediate structs. `startup.rs`
  separates each startup phase into a distinct function returning `ReconciledSettings`,
  `ValidatedConfig`, or `PkiRuntime`. Each phase is named in a comment in `main.rs`
  (`// Phase N: …`), making startup failures immediately attributable to the phase that failed
  and eliminating accidental partial initialization.
- `src/tasks.rs` -- `BackgroundTasks` registry provides a clean `track` / `track_abort`
  separation: tasks that respect a `CancellationToken` are awaited with a per-task timeout,
  while tasks that cannot be gracefully stopped are aborted. Shutdown is a single
  `bg.shutdown(…).await` call from `main.rs`.
- `src/startup.rs` -- `AppState` builder pattern prevents partial initialization. The builder
  catches the first missing field by name at compile time via `AppStateBuildError`, eliminating
  runtime panics from forgotten fields.
- `src/pki.rs` -- `CaKeyStore` is not `Clone`; `Debug` redacts all key material. The private
  key store cannot be accidentally duplicated and never leaks key material to logs.
  `Zeroizing<String>` is used for all CA private keys throughout `CaKeyStore`.
- `src/tasks.rs` -- `spawn_ca_rotation` is trigger-based, not timer-based. The expensive CA
  rotation is driven by a `Notify` signal from the scheduler's `CaRotationCheckExecutor`,
  which fires only when rotation is genuinely needed. Avoids a fixed 24-hour interval holding
  a rotation lock on all controller instances simultaneously.
- `src/startup.rs:157-232` -- HA-safe master key verification at Phase 4. `verify_master_key`
  uses `insert_setting_if_absent` to handle the race between two controller instances starting
  simultaneously. If a race is detected, the lagging instance reads the winner's token and
  verifies against it, failing hard with a clear error message.
- `src/main.rs:419-445` -- Rolling zero-downtime handoff via SIGUSR1 / `--takeover-from`. The
  `run()` event loop listens for `SIGUSR1` as a shutdown signal. A new instance sends
  `SIGUSR1` to the old instance after its server is ready, enabling blue/green restarts
  without a gap in service.
- `Cargo.toml:10-19` -- Feature flags for database backends, OIDC, embedded scheduler, and
  embedded frontend compose correctly via cascading features.
- `migration/m20260209_000001_initial.rs` -- Squashed single migration with a correct `down()`
  path that drops tables in correct reverse FK order. Partial unique indexes
  (`WHERE deactivated_at IS NULL`) prevent duplicate slugs among active records without
  conflicting with soft-deleted rows, correctly expressed as raw SQL with inline comments.
- CA rotation uses CAS (`UPDATE WHERE value = expected_fp`) inside a transaction. CA version
  counter with `bump_setting_i64` gives cross-instance reload tasks a cheap change-detection
  probe.

### Issues

**[MEDIUM]** `Cargo.toml:22-60` -- Controller depends directly on `uptrakit-web-api`, creating
a dependency from a core binary upward into the UI layer. The startup code directly constructs
web-api internal types (`OidcFlowStore`, `DeviceFlowStore`, `RateLimitStore` at
`src/main.rs:206-219`) that should be constructed inside `AppState::builder().build()`.

**[LOW]** `src/main.rs:324-379` -- Embedded scheduler block directly imports executor types and
manually registers each task type. This registration should be encapsulated in the
scheduler-engine crate via a `register_all_executors` function.

## Security and Safety

### Strengths

- `src/pki.rs` -- CA private keys stored AES-256-GCM encrypted in the database. The
  `generate_ca` / `rotate_managed_ca` path calls `EncryptedString::new(bundle.key_pem)` before
  storing. In memory the key lives in `Zeroizing<String>` and is never exposed through `Debug`.
- `src/crl_manager.rs` -- CRL revocation integrated into the live TLS configuration. The
  `CrlManager` rebuilds `rustls::ServerConfig` from current CA state plus DB revocation records
  whenever a certificate is revoked or the CA rotates. The `WebPkiClientVerifier` is constructed
  with `.with_crls(crls)` so revocation checking is enforced at the TLS handshake level.
- CA rotation uses compare-and-swap in the database. `rotate_managed_ca` issues
  `UPDATE WHERE value = expected_fp` on the active fingerprint setting. If another instance
  raced first, `rows_affected == 0` and the local instance returns `rotated: false`.
- `recover_stale_claims` limits the crash-recovery window. Stale claims (locked longer than
  600 seconds) are released in every poll cycle. A crashed controller does not permanently
  block task execution.
- Server certificate auto-renewal is watch-channel-driven. `spawn_ca_reload` detects
  cross-instance CA updates by comparing a version counter in the `settings` table and
  rebuilds the TLS config without a restart.
- `src/startup.rs:60-76` -- `init_master_key` returns `Option<SecretString>` so the hex string
  is zeroed on drop. The parsed key bytes use `Zeroizing<[u8; 32]>` throughout.
- `src/startup.rs:77-83` -- Master key initialization requires explicit
  `--allow-plaintext-secrets` with clear warning.
- `src/startup.rs:897-910` -- Master key rejected if not exactly 64 hex characters (32 bytes).
- `src/startup.rs:854-870` -- JWT signing key stored in DB (encrypted), migrated from
  file-based storage.
- `src/mtls_acceptor.rs` -- Dual-auth mTLS model supporting both enrolled (no cert) and
  authenticated (with cert) agents on a single listener.
- `src/reencrypt.rs` -- Idempotent, HA-safe re-encryption of legacy plaintext values at
  startup.
- `src/startup.rs:119` -- Database URL sanitized before logging.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/durations.rs` -- Centralizes all timing constants with doc-comments. There are no magic
  numbers in the background task and scheduler code paths. Constants such as
  `BACKGROUND_TASK_SHUTDOWN_TIMEOUT`, `SERVER_CERT_RENEWAL_WINDOW_DAYS`, and
  `RESTART_NOTIFICATION_SCATTER` are used consistently.
- `src/startup.rs` -- Discrete startup phases reduce function complexity. Each phase function
  is independently readable and testable. `reconcile_all_settings` follows a repetitive,
  auditable `ReconcileParams` pattern. The `DisplayVec` helper and the
  `reconcile_setting_vec` / `reconcile_socket_addr` wrappers are clean abstractions.
- `src/main.rs` -- `AppError` error type is minimal and domain-appropriate. The six variants
  cover exactly the startup failure modes without over-engineering. No `String`-wrapped
  generic errors.
- `src/scheduler/mod.rs:417-424` -- `TrackingExecutor` pattern avoids mocking frameworks.
  An anonymous struct with `AtomicBool` flag keeps tests self-contained.
- `src/pki.rs` -- 30+ inline unit tests covering CA generation, server certificate
  round-trips, fingerprint determinism, SAN extraction, AIA/CDP extension embedding, and
  the `validate_ca_pki_addr` matrix (four cases).
- `src/reconcile.rs` -- `reconcile_setting` tests cover all five branches: no DB + no CLI =
  default, no DB + CLI = CLI value, DB exists + no CLI = DB value, DB exists + CLI differs +
  no force = DB wins, DB exists + CLI differs + force = CLI wins. Each verifies both return
  value and persisted DB state.
- `tests/reverse_proxy/` -- Reverse-proxy integration tests covering Nginx, HAProxy, Caddy,
  Envoy, and Traefik for both CRL and OCSP revocation scenarios. Every test carries
  `#[ignore = "Docker integration test…"]` with an exact `cargo test` runbook command.
- `src/scheduler/claim.rs` -- Thorough claim tests covering acquisition, double-claim
  rejection, release with success, release with error (run_count not incremented),
  `find_due_tasks` filtering, `release_all_claims` scoped to controller, and
  `trigger_immediate`. All use in-memory SQLite with no inter-test leakage.
- `src/scheduler/executors/ca_rotation_check.rs` -- `#[tokio::test(start_paused = true)]`
  used correctly in executor tests, making `tokio::time::timeout` assertions deterministic
  regardless of CI load.
- `src/pki.rs:26-107` -- Hand-written DER encoding for AIA/CDP extensions is well documented
  with correct length encoding bounds checks.
- `src/crl_manager.rs` -- Atomic counters for CRL number and revocation version prevent data
  races without lock contention.

### Issues

**[LOW]** `src/pki.rs:543` -- `bundle_from_pem` takes `String` parameters where `&str` would
suffice for key parsing. Ownership is eventually needed for the struct, but the parameter
naming could clarify this.

**[LOW]** `src/pki.rs` -- 1,646 lines with ~600 lines of production code and ~1,000 lines of
tests. The DER encoding functions could be extracted to a sub-module.

**[LOW]** `tests/reverse_proxy/nginx_ocsp.rs:46,164,297` -- Three OCSP test scenarios wait a
fixed 1 second for Nginx to initialize its OCSP cache. Fragile on slow or loaded hosts. Should
be replaced with a retry loop polling a health endpoint. Note: these tests carry
`#[ignore = "Docker integration test…"]`, so per AGENTS.md Exception 1 this does not violate
the no-real-sleeps invariant.

**[LOW]** `tests/reverse_proxy/nginx_ocsp.rs:424` -- TOCTOU port reservation race.
`reserve_port()` binds a listener, reads its port, then drops the listener before starting the
OCSP responder on that port. Another process can bind the port in the window between the drop
and the bind.

## High Availability

### Strengths

- `src/scheduler/claim.rs:22-37` -- `try_claim` uses a single atomic
  `UPDATE WHERE locked_by IS NULL`. This is a TOCTOU-free optimistic lock: two concurrent
  controllers racing to claim the same task will have exactly one winner determined by the
  DB engine, not by application-level reads.
- `src/scheduler/mod.rs:85-91` -- `release_all_claims` on cancellation avoids the stale-claim
  window. The `token.cancelled()` arm releases all holds before the scheduler task exits.
  A clean shutdown does not trigger the 600-second stale recovery wait on restart.
- `recover_stale_claims` as a safety net for crashed instances. Every poll cycle calls
  `recover_stale_claims` before finding due tasks, releasing locks held for more than
  10 minutes. Bounds the worst-case re-execution delay after an unclean shutdown.
- `src/tasks.rs:69-91` -- `broadcast_server_restarting_scattered` spreads notifications across
  a configurable window (`RESTART_NOTIFICATION_SCATTER`) so agents do not all reconnect
  simultaneously after a controller restart.
- `src/tasks.rs` -- `spawn_settings_reload` skips first tick, preventing a redundant reload of
  settings that were just loaded during startup.
- Cross-instance CA update propagation via version counter. `spawn_ca_reload` polls
  `PkiCaVersion` in the settings table. When the version advances, it reloads CA state and
  rebuilds the CRL manager and TLS config without a restart.
- `src/crl_manager.rs:234-282` -- CRL manager uses version-gated polling (60s) with instant
  local rebuilds via `Notify`. Minimizes unnecessary rebuilds.
- `src/durations.rs:1-34` -- All HA-critical timing constants centralized with documentation,
  making HA tuning auditable from a single location.
- `src/tasks.rs:17-115` -- Well-structured `BackgroundTasks` with both cooperative (`track`)
  and forceful (`track_abort`) shutdown modes.
- `src/tasks.rs:125-175` -- Service drain waits for disconnects with polling and timeout.
- `src/main.rs:419-445` -- SIGUSR1 handler for zero-downtime restarts with `--takeover-from`
  and `SO_REUSEPORT` support.
- `src/tasks.rs:335-348` -- CA rotation uses optimistic locking via `expected_fp` for
  multi-instance safety.

### Issues

**[LOW]** `src/main.rs:451-457` -- PKI HTTP server registered with `track_abort`, meaning
in-flight OCSP/CRL requests are terminated mid-response on shutdown.

## Coding Standards

### Strengths

- `edition = "2024"` with workspace inheritance for all standard fields. `license`, `authors`,
  `repository`, and `version` all use `workspace = true`.
- Consistent use of `bail!`, `report!`, and `context` / `context_to`. No `Report::new()`
  anti-pattern. `AppError` conversion via `impl_report_conversion!`.
- No `unwrap()` in startup, shutdown, or scheduler paths. The startup sequence propagates all
  errors as `Report<AppError>` and exits via `std::process::ExitCode::FAILURE` with a
  formatted error message.
- `src/main.rs:364-371` -- Signal handling uses `tokio::signal::unix::signal` typed by
  `SignalKind`. Handles `SIGTERM`, `SIGINT`, and `SIGUSR1` with error-propagating setup.
- `tracing_subscriber` initialized in the binary's `main`, not in a library. This is the
  correct location; the binary owns subscriber initialization.
- All domain-significant duration constants centralized in `src/durations.rs` with
  doc-comments.
- Well-organized build metadata handling via `build.rs`.

### Issues

**[MEDIUM]** `src/pki.rs:700` -- `unwrap_or(0)` on CA version query. While the `None` case is
intentional (default to 0 for new systems), it would benefit from a clarifying comment.

**[LOW]** `src/scheduler/mod.rs:18` -- `DEFAULT_POLL_INTERVAL_SECS` is a module-private
`const u64` that duplicates domain knowledge in `durations.rs`. `durations.rs` already contains
all other timing constants. Move to `durations.rs` as
`pub(crate) const SCHEDULER_POLL_INTERVAL: Duration` for consistency and discoverability.

## Extensibility

### Strengths

- `src/scheduler/executor.rs:9` -- `TaskExecutor` trait is minimal and object-safe. The single
  required method `async fn execute(&self, task: &Model) -> Result<()>` has no associated
  constants or types. Adding a new executor is a two-step operation: implement the trait, then
  call `sched.register(TaskType, Box::new(…))`.
- `ScheduledTaskType` enum drives both DB seed data and runtime executor registration. A clear
  mapping between the enum variant, the migration seed row, and the executor registered in
  `main.rs`. A new task type requires additions in only two places.
- `src/startup.rs` -- `PkiRuntime` struct decouples PKI initialization from `AppState`
  building. All PKI-related fields flow through a single intermediate struct.
- Feature flag pass-through for `oidc` and database backends demonstrates proper cascading.

### Issues

**[MEDIUM]** `src/scheduler/executor.rs:9` -- `TaskExecutor` trait has no registration or
discovery mechanism for new task types. Adding a new scheduled task requires creating a new
executor, adding a match arm, and registering manually. Unlike the `register_plugins!` macro,
there is no compile-time check ensuring all `ScheduledTaskType` variants have executors.

**[LOW]** `src/scheduler/claim.rs:160-164` -- `find_due_tasks` is scoped to a single
`tenant_id`. The `SchedulerConfig` carries a `tenant_id` and the query enforces it. Any future
multi-tenant extension will require a redesign of how tasks are discovered and assigned.
Adding a comment noting "single-tenant by design" would make the limitation visible.

## Tests

### Strengths

- `src/pki.rs:1189-1646` -- 38 unit tests covering CA generation, server certificate signing,
  extra SANs, hostname deduplication, expired certificate detection, malformed PEM handling,
  fingerprint determinism, SAN extraction, server cert renewal and CA rotation threshold logic,
  AIA/CDP extension generation and round-trip parsing, `validate_ca_pki_addr` four-case matrix,
  basic constraints path-length, DER encoding correctness, and the async `managed_ca_init_and_rotation`
  round-trip. The module is comprehensively tested at the unit level.
- `src/scheduler/claim.rs` -- Claim acquisition, double-claim rejection, release with success,
  release with error (run_count not incremented), `find_due_tasks` filtering,
  `release_all_claims` scoped to controller, and `trigger_immediate` all covered with in-memory
  SQLite. No inter-test leakage.
- `src/scheduler/mod.rs:197-243` -- `#[tokio::test(start_paused = true)]` used correctly for
  the three tracking-executor tests. Tests use `tokio::time::timeout` internally, so virtual
  time is justified and makes them deterministic on loaded CI.
- `src/reconcile.rs:142-274` -- Five branches of `reconcile_setting` tested exhaustively with
  in-memory SQLite: no-DB/no-CLI defaults, no-DB/CLI uses CLI, DB/no-CLI uses DB,
  DB/CLI-differs/no-force uses DB, DB/CLI-differs/force uses CLI. Each asserts both return
  value and the persisted DB state.
- `src/startup.rs:1102-1150` -- Six tests for master key parsing covering missing key, env/file
  whitespace trimming, invalid hex, invalid length, and valid 32-byte hex. These gates guard the
  security-critical startup path.
- `src/db_migrate/mod.rs:163-197` -- Schema migration smoke test present; the heavier
  integration test is correctly tagged `#[ignore = "integration — runs schema migrations on two
  in-memory SQLite databases"]` with a precise runbook comment, following project convention.
- `tests/reverse_proxy/` -- Docker integration tests for Nginx (CRL, OCSP), HAProxy (CRL,
  OCSP), Caddy, Envoy (CRL), and Traefik; every test carries `#[ignore = "Docker integration
  test…"]` with an exact `cargo test` runbook command, consistent with AGENTS.md requirements.
- `src/reconcile.rs` -- Tests use plain `#[tokio::test]` (no `start_paused`): correct, because
  they acquire SQLite connections and `start_paused` would trigger `ConnectionAcquire(Timeout)`.

### Issues

**[HIGH]** `src/reencrypt.rs` -- `reencrypt_legacy_plaintext` and all five per-table helpers
have zero test coverage. The function is idempotent and HA-safe by design, but the logic that
distinguishes already-encrypted values (`is_db_value_encrypted()` prefix check) from plaintext
values, skips `None` passwords, and counts re-encrypted rows is entirely untested. A test using
an in-memory SQLite database with a mix of plaintext and pre-encrypted rows for each table
(CA keys, OIDC secrets, MQTT passwords, MQTT CA certs, OIDC PKCE verifiers) should be added.
The skip-when-no-master-key path also warrants a test.

**[MEDIUM]** `src/startup.rs` -- `reconcile_all_settings` (the 50+ setting reconciliation
orchestrator at line 247) has no direct test coverage. The individual `reconcile_setting`
primitive is well tested, but the composition — setting-key enumeration, `force` flag
propagation, `ReconcileParams` wiring — is exercised only by running the binary. A test
constructing a minimal in-memory DB and calling `reconcile_all_settings` with a partial
`ReconciledSettings` fixture would catch mis-wiring without requiring a full controller startup.

**[MEDIUM]** `src/crl_manager.rs` -- CRL generation, revocation record insertion, and
`rustls::ServerConfig` rebuild are untested. The `CrlManager` is central to mTLS security:
a regression in CRL number sequencing or revocation record encoding would not be caught before
production. Unit tests using in-memory SQLite and the existing `generate_ca` helper could
exercise `build_crl`, `revoke_certificate`, and the `update_server_config` notification path.

**[LOW]** `src/db_migrate/tables.rs:347-360` -- Only one unit test (`parse_table_names_basic`)
covers the table-name parsing helper. Edge cases (empty input, mixed whitespace, duplicate
names) are not exercised.

**[LOW]** `src/pki.rs:1257` -- `#[tokio::test]` (without `start_paused`) used for
`server_cert_round_trip` at line 1257. This test calls no Tokio time APIs, so omitting
`start_paused` is correct per AGENTS.md rule 1; noted here as confirmation rather than issue.

## Consistency

### Strengths

- `src/main.rs:325-444` -- All background tasks are registered through `BackgroundTasks` via
  `bg.track()`, `bg.track_with_timeout()`, or `bg.track_abort()`. The sole direct
  `tokio::spawn` calls (`crl_handle`, `server_task`, `pki_http_handle`) each flow immediately
  into a `bg.track*` call or are managed explicitly in the event loop. No task is silently
  orphaned.
- `src/startup.rs:247-506` -- Every setting in `reconcile_all_settings` uses the same
  `ReconcileParams` struct and the same five-case logic from `reconcile_setting`. The NATS URL
  reconciliation at `startup.rs:459-497` follows the identical pattern — including the
  post-reconcile `settings.set_nats_url()` call — with no divergence from the earlier settings.
- `src/tasks.rs` -- All `spawn_*` functions share the same signature shape: first argument is
  `CancellationToken`, remaining arguments are the resources needed, return type is
  `JoinHandle<()>`. Error paths inside each loop use the same `tracing::warn!` / `continue`
  pattern. No task silently absorbs errors without logging.

### Issues

**[MEDIUM]** `src/startup.rs:33-34` vs all other `ReconciledSettings` fields -- The NATS URL
field is gated with `#[cfg(feature = "nats")]`, making it the only field in `ReconciledSettings`
that callers must feature-gate to access. Every other field (`extra_sans`, `pki_addr`,
`https_addr`) is an unconditional plain field. This asymmetry forces `#[cfg(feature = "nats")]`
annotations at every downstream access site (`main.rs:233`, `main.rs:269`). Wrapping NATS
settings in a dedicated always-present sub-struct (e.g., `nats: NatsSettings` with
`url: Option<String>`) would let callers read `reconciled.nats.url` without conditional
compilation.

**[MEDIUM]** `src/main.rs:499-504` vs `src/tasks.rs:spawn_*` -- The PKI HTTP server is
spawned as an inline `async move` closure directly in `main.rs` rather than through a dedicated
`spawn_pki_http` helper in `tasks.rs`. Every other background task (denylist cleanup, settings
reload, CA reload, CA rotation, server cert renewal, NATS consumer) has its own named function
in `tasks.rs`. The inline spawn diverges from this pattern and buries the PKI HTTP error
logging inside `main.rs:run()`.

**[LOW]** `src/main.rs:325` and `src/main.rs:408` -- The CRL manager and the embedded scheduler
are spawned with bare `tokio::spawn` inside `main.rs:run()` before being handed to `bg.track`.
All other tasks are spawned inside their `spawn_*` helpers, meaning the `tokio::spawn` call
and the `bg.track` registration are co-located in one function. For the CRL manager and
scheduler, the spawn and registration are in `main.rs` but separated by unrelated code,
making it harder to audit that every task is properly tracked.

## Maintainability

### Strengths

- `src/durations.rs` -- All timing constants in one file with doc-comments. Every consumer uses
  the constant name rather than a numeric literal. Tuning and audit is a single-file operation.
- Startup phase functions are named, independent, and bounded in length. Each phase in
  `startup.rs` is a self-contained function; any one phase can be read without understanding
  the others.
- `src/reencrypt.rs` -- Re-encryption logic is properly isolated with a clear module doc
  explaining idempotency and HA-safety guarantees. Adding a new encrypted column follows an
  obvious 30-line pattern.
- `src/scheduler/mod.rs` -- `CaRotationCheckExecutor` is kept inside the controller with a
  module doc comment explaining why: it requires the in-process CA watch channel. The design
  decision is visible at the call site rather than buried in a dependency.

### Issues

**[MEDIUM]** `src/reencrypt.rs:46-315` -- Five near-identical re-encryption helpers
(`reencrypt_ca_certificate_keys`, `reencrypt_oidc_client_secrets`, `reencrypt_mqtt_passwords`,
`reencrypt_mqtt_ca_certs`, `reencrypt_oidc_flow_pkce_verifiers`) repeat the same structure:
load all rows, iterate, check `is_db_value_encrypted`, encrypt, update, log. Adding a new
encrypted column requires copying ~50 lines of boilerplate. A generic `reencrypt_column` helper
parameterised by entity type and accessor closures would reduce each new column to a single
call, and make the count-and-log logic testable once rather than five times.

**[MEDIUM]** `src/pki.rs:1-1617` -- At 1,617 total lines, the file mixes CA lifecycle
operations, DER encoding helpers (`build_aia_extension_der`, `encode_access_description`,
`encode_der_sequence`, `encode_der_length`), and ~1,000 lines of tests. The DER encoding
functions are logically distinct from CA management and could be extracted to `src/pki/der.rs`,
reducing the primary module to pure CA lifecycle code. Already noted in the existing review;
the maintenance impact is that modifications to AIA/CDP extension logic require locating the
correct helper among unrelated CA rotation code.

**[LOW]** `src/startup.rs:247-500` -- `reconcile_all_settings` repeats `ReconcileParams {
db, key, raw, cli_value, default_value, force, convert }` construction ~12 times. The struct
is the right abstraction but the closure-heavy `convert` field makes each call visually dense.
For primitive types (`bool`, `SocketAddr`, `u16`) that appear multiple times, a typed helper
wrapper (e.g., `reconcile_bool`, `reconcile_socket_addr`) would remove the redundant closure
boilerplate and reduce the chance of mis-wiring a field.

**[LOW]** `src/scheduler/mod.rs:17` -- `DEFAULT_POLL_INTERVAL_SECS: u64` duplicates the domain
knowledge already present in `durations.rs` for all other timing constants. A reader looking
for tunable timeouts will find it only if they inspect `scheduler/mod.rs` separately. Move to
`durations.rs` as a typed `Duration` constant, consistent with the rest of the module.

## Database

### Strengths

- `src/db/mod.rs:20-25` -- Connection pool configured with four explicit timeouts:
  `connect_timeout(8s)`, `acquire_timeout(8s)`, `idle_timeout(8s)`, and `min_connections(1)`.
  All four bounds are intentional: `acquire_timeout` prevents indefinite hangs under load,
  `idle_timeout` ensures connections released by a peaked workload are reaped promptly, and
  `min_connections(1)` avoids the cold-start latency of establishing the first connection on the
  first request after an idle period.
- `src/db/mod.rs:25` -- `sqlx_logging(false)` is the correct production setting. The default
  SQLx query logger emits raw SQL at DEBUG level, which risks leaking sensitive query parameters
  (e.g., hash values, token strings) to log aggregators. Suppressing it in favour of explicit
  `tracing` calls in application code is the safer default.
- `src/db/config.rs:46-73` -- `validate_backend_support` provides a compile-time-aware runtime
  check: if the URL scheme does not match an enabled Cargo feature, the error is caught before
  the connection attempt rather than as an opaque `ConnectOptions` failure. The `compile_error!`
  in `src/db/mod.rs:1-5` ensures at least one backend feature is always present.
- `src/startup.rs:119-160` -- `init_database` runs migrations before loading the default tenant.
  The migration-first order guarantees the tenant row seeded by the initial migration is present
  before any subsequent startup phase issues a query against it. The `ok_or_else` on the tenant
  lookup produces a typed `AppError::Database` rather than a silent `None` panic.
- `src/startup.rs:166-239` -- `verify_master_key` uses `insert_global_setting_if_absent`
  (a compare-and-insert path that returns a boolean on conflict) to handle the race between two
  controller instances starting simultaneously. The lagging instance re-reads the winner's stored
  token and verifies against it rather than failing silently or overwriting it. This is a correct
  application-level CAS pattern for a setting that must be written exactly once.
- `src/reconcile.rs:37-45` -- `reconcile_setting` reads from a pre-fetched `RawSettings` map
  rather than issuing per-setting DB queries. A single `load_all_global_settings()` call at
  Phase 6 entry produces the map; the subsequent 12+ reconcile calls perform only upserts
  (write path), not reads. This is the correct bulk-read pattern for startup settings
  reconciliation.
- `src/db_migrate/tables.rs:10-58` -- The `COPY_ORDER` constant and the `copy_all` / `clean_all`
  / `verify_all` macros enumerate all 38 application tables in FK-safe order. The clean order
  mirrors the initial migration `down()` drop order. A three-phase migrate (clean → copy →
  verify) with row-count reconciliation at the end is a correct cross-DB migration strategy
  that catches truncated copies before the source connection is closed.
- `src/reencrypt.rs:7-15` -- Re-encryption is documented as idempotent and HA-safe. The
  `is_db_value_encrypted()` prefix check (`ENC:v1:`) makes the guard a single string operation
  with no DB round-trip. Concurrent controllers racing on the same row produce the same result
  (deterministic encryption under the same key), making last-writer-wins semantically correct.

### Issues

**[MEDIUM]** `src/db_migrate/tables.rs:235-279` -- `migrate_table` uses offset-based pagination
(`E::find().offset(offset).limit(batch_size)`) to copy rows in batches. Offset pagination is
correct for a static snapshot but silently produces incorrect results if the source database
receives new writes during the migration: rows inserted below the current offset are skipped,
and rows inserted at positions already read are duplicated. The `db-migrate` command has no
mechanism to prevent concurrent writes to the source (no advisory lock, no read-only connection
mode). For SQLite the risk is low (exclusive write lock on the source is acquired implicitly by
the copy queries), but for PostgreSQL and MySQL concurrent inserts into the source during
migration will silently produce a partial copy. A `READ ONLY` transaction on the source
connection (or a banner warning in the confirmation prompt) would make the risk explicit.

**[MEDIUM]** `src/reencrypt.rs:46-315` -- Each per-table re-encryption helper loads all rows
with `find().all(db)` before iterating. For a large deployment with thousands of CA
certificates, OIDC providers, or MQTT clients this is an unbounded in-memory load at startup.
The pattern also holds the encrypted value in a `String` clone between the read and the update.
A chunked iteration using `paginate(db, PAGE_SIZE).fetch_and_next()` would bound memory usage
and reduce the window in which decrypted key material is alive in the heap.

**[LOW]** `src/db/config.rs:37-38` -- The default SQLite path is constructed as
`data_dir.join("uptrakit.db")` with `?mode=rwc` (read-write-create). If `data_dir` does not
exist at startup, SQLite will fail to create the file, but the error message from the driver
is `unable to open database file`, with no indication that the parent directory is missing. A
pre-flight `std::fs::create_dir_all(data_dir)` in `from_args` (before constructing the URL)
would produce a clearer error and match the pattern used in `startup.rs` for the state directory.

**[LOW]** `src/db/mod.rs:23-24` -- `connect_timeout` and `acquire_timeout` are both hardcoded
to 8 seconds and are not exposed through `DbConfig` or the CLI. The `max_connections` parameter
is user-configurable via `--db-max-connections`, but the timeout values are invisible and cannot
be tuned for environments where the DB server is remote or under load. A comment citing the
rationale for the 8-second value (or exposing them as optional CLI parameters with these
defaults) would aid operators debugging slow connection acquisition.
