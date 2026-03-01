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

**[MEDIUM]** `src/db/mod.rs:20-25` -- Database connection pool hardcoded at
`max_connections=10` with no runtime configurability. All timeouts (8 seconds) and pool sizes
are hardcoded literals. Under connection exhaustion, cascading failures affect all subsystems
simultaneously.

**[MEDIUM]** `src/crl_manager.rs:202-222` -- CRL rebuild holds two `RwLock` read guards
simultaneously (`issuers.read()` and `server_cert.read()`). Lock ordering is not documented.
Creates a consistency window during concurrent CA rotation + server cert renewal.

**[MEDIUM]** `src/main.rs:437` -- Fixed 100ms sleep before SIGUSR1 takeover signal. If the
server takes longer than 100ms to bind, the old process is signaled before the new one is
ready.

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

**[MEDIUM]** `src/scheduler/mod.rs:48` -- `executors` field is
`HashMap<ScheduledTaskType, Box<dyn TaskExecutor>>`; `Scheduler::register` uses
`HashMap::insert`, which silently replaces any previously registered executor for a given task
type with no warning. The method should either `debug_assert!` that the key is not already
present, or return `Option<Box<dyn TaskExecutor>>` to expose the displacement.

**[MEDIUM]** `src/scheduler/executor.rs:9` -- `TaskExecutor` trait has no registration or
discovery mechanism for new task types. Adding a new scheduled task requires creating a new
executor, adding a match arm, and registering manually. Unlike the `register_plugins!` macro,
there is no compile-time check ensuring all `ScheduledTaskType` variants have executors.

**[LOW]** `src/scheduler/claim.rs:160-164` -- `find_due_tasks` is scoped to a single
`tenant_id`. The `SchedulerConfig` carries a `tenant_id` and the query enforces it. Any future
multi-tenant extension will require a redesign of how tasks are discovered and assigned.
Adding a comment noting "single-tenant by design" would make the limitation visible.
