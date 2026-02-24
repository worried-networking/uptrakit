# CODEREVIEW — uptrakit-controller

> Reviewed: 2026-02-23
> Reviewer: Senior Rust Engineer (automated phase 2)
> Scope: `crates/core/controller/` — the primary controller binary

---

## Summary

`uptrakit-controller` is the most complex binary in the workspace. It orchestrates
database migrations, a 10-phase startup sequence, full PKI lifecycle management
(CA generation, CRL signing, OCSP support), a DB-backed HA scheduler, and a suite
of background tasks. The overall design is solid: startup phases are typed structs,
the scheduler uses TOCTOU-free optimistic locking, and the PKI layer is well-covered
by unit tests. Two issues require immediate attention: (1) the mTLS verifier allows
unauthenticated clients at the TLS layer, delegating all trust enforcement to
application-layer checks; and (2) the CRL manager is registered with `track_abort`
rather than `track`, meaning a shutdown abort may corrupt the TLS configuration file
on disk.

---

## Architecture

### Strengths

- **10-phase startup with typed intermediate structs.** `startup.rs` separates
  each startup phase into a distinct function returning `ReconciledSettings`,
  `ValidatedConfig`, or `PkiRuntime`. This makes startup failures immediately
  attributable to the phase that failed and eliminates accidental partial
  initialisation. Each phase is named in a comment in `main.rs` (`// Phase N: …`),
  which is useful for operator log parsing.

- **`BackgroundTasks` registry.** `tasks.rs` provides a clean `track` /
  `track_abort` separation: tasks that respect a `CancellationToken` are
  awaited with a per-task timeout, while tasks that cannot be gracefully stopped
  are aborted. Shutdown is a single `bg.shutdown(…).await` call from `main.rs`.

- **`AppState` builder pattern prevents partial initialisation.** The builder
  catches the first missing field by name at compile time via `AppStateBuildError`,
  eliminating runtime panics from forgotten fields.

- **`CaKeyStore` is not `Clone`; `Debug` redacts all key material.** The private
  key store cannot be accidentally duplicated and never leaks key material to logs.
  `Zeroizing<String>` is used for all CA private keys throughout `CaKeyStore`.

- **`spawn_ca_rotation` is trigger-based, not timer-based.** The expensive CA
  rotation is driven by a `Notify` signal from the scheduler's
  `CaRotationCheckExecutor`, which fires the trigger only when rotation is
  genuinely needed. This avoids a fixed 24-hour interval holding a rotation
  lock on all controller instances simultaneously.

- **HA-safe master key verification at Phase 4.** `verify_master_key` uses
  `insert_setting_if_absent` to handle the race between two controller instances
  starting simultaneously. If a race is detected, the lagging instance reads
  the winner's token and verifies against it, failing hard with a clear error
  message rather than silently proceeding.

- **Rolling zero-downtime handoff via SIGUSR1 / `--takeover-from`.** The
  `run()` event loop listens for `SIGUSR1` as a shutdown signal. A new instance
  sends `SIGUSR1` to the old instance after its server is ready, enabling
  blue/green restarts without a gap in service.

### Issues

**[SEVERITY: Medium]** `crates/core/controller/Cargo.toml:50-51` — `chrono` and `cron` not in workspace dependencies

`cron = "0.15"` and `chrono = { version = "0.4" }` are declared inline. The
workspace uses `time = "0.3"` as its primary date-time crate (workspace-pinned).
`chrono` is pulled in solely because `cron` requires it. During the `2.0.0-rc`
series of sea-orm, patch versions arrive frequently; independent inline pins risk
divergence. Both should be added to `[workspace.dependencies]`.

**[SEVERITY: Medium]** `crates/core/controller/Cargo.toml:45` — `sea-orm-migration` not in workspace dependencies

```
sea-orm-migration = { version = "2.0.0-rc.32", default-features = false, … }
```

`sea-orm` is workspace-pinned but `sea-orm-migration` is not. Both crates must
stay at matching RC versions; an independent bump of either during the RC series
will cause runtime migration failures or type-level incompatibilities. Add to
`[workspace.dependencies]` alongside `sea-orm`. `crates/core/agent-ssh` has the
same issue.

**[SEVERITY: Low]** `crates/core/controller/Cargo.toml:64` — `base64 = "0.22"` in dev-dependencies is not workspace-pinned

`base64` is used in several other crates and is already a transitive dependency.
An independent version here risks subtle encoding differences if the workspace-
transitive version and the explicitly declared version diverge. Add to
`[workspace.dependencies]`.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `crates/core/controller/src/db/config.rs:13,23,42-45,48,53,58` — Non-additive feature flag pattern: 6 `#[cfg(not(feature))]` usages in database configuration

Uses `#[cfg(not(feature = "..."))]` to provide error messages when a database URL scheme does not match an enabled feature. Violates the additivity principle. The 2 `#[cfg_attr(not(feature), allow(...))]` usages compound the violation with prohibited `#[allow()]` attributes.

**[SEVERITY: Medium]** `crates/core/controller/src/cli.rs:120-122` — Non-additive feature flag: `#[cfg(not(feature = "embed-frontend"))]` conditionally removes a CLI argument

When `embed-frontend` is enabled, the `--static-dir` CLI argument disappears. The additive alternative is to always declare the field but validate at runtime.

**[SEVERITY: Low]** `crates/core/controller/src/startup.rs:584-585,907` — Non-additive feature flag: `#[cfg(not(feature = "embed-frontend"))]` on startup logic

Two additional usages compile out the `resolve_static_dir` function when the frontend is embedded.

---

## Security & Safety

### Strengths

- **CA private keys stored AES-256-GCM encrypted in the database.** The
  `generate_ca` / `rotate_managed_ca` path calls
  `EncryptedString::new(bundle.key_pem)` before storing. In memory the key lives
  in `Zeroizing<String>` and is never exposed through `Debug`.

- **CRL revocation integrated into the live TLS configuration.** The
  `CrlManager` rebuilds `rustls::ServerConfig` from current CA state plus DB
  revocation records whenever a certificate is revoked or the CA rotates. The
  rustls `WebPkiClientVerifier` is constructed with `.with_crls(crls)` so
  revocation checking is enforced at the TLS handshake level.

- **CA rotation uses compare-and-swap in the database.** `rotate_managed_ca`
  issues an `UPDATE WHERE value = expected_fp` on the active fingerprint setting.
  If another controller instance raced and rotated first, `rows_affected == 0`
  and the local instance returns `rotated: false` without applying a double
  rotation.

- **`recover_stale_claims` limits the crash-recovery window.** Stale claims
  (locked longer than 600 seconds) are released in every poll cycle. A crashed
  controller does not permanently block task execution.

- **Server certificate auto-renewal is watch-channel-driven.** `spawn_ca_reload`
  detects cross-instance CA updates by comparing a version counter in the
  `settings` table and rebuilds the TLS config without a restart.

### Issues

**[SEVERITY: High]** `crates/core/controller/src/pki.rs:1167-1172` — mTLS verifier uses `.allow_unauthenticated()`

```rust
let verifier = WebPkiClientVerifier::builder(Arc::new(root_store))
    .with_crls(crls)
    .allow_unauthenticated()      // <-- clients without a cert are accepted
    .only_check_end_entity_revocation()
    .build()
    …
```

Clients that present no certificate establish a full TLS session. All agent
WebSocket trust then depends entirely on application-layer identity checks
(`machine_id` validation, JWT, etc.). A bug in any one of those checks allows an
unauthenticated client to reach the WebSocket handler. Removing
`.allow_unauthenticated()` would enforce mutual authentication at the transport
layer, providing defense in depth. Reverse-proxy deployments that forward
pre-validated certificates should remain an opt-out path documented at the
configuration level, not the default.

**[SEVERITY: Medium]** `crates/core/controller/src/pki.rs:69-77` — `encode_der_length` silently truncates lengths >= 65,536 bytes

```rust
fn encode_der_length(len: usize) -> Vec<u8> {
    if len < 0x80 {
        vec![len as u8]
    } else if len < 0x100 {
        vec![0x81, len as u8]
    } else {
        vec![0x82, (len >> 8) as u8, len as u8]   // only 2-byte long-form
    }
}
```

The function handles lengths up to 65,535 bytes (two-byte DER long-form). Any
length >= 65,536 produces a silently truncated DER encoding that will fail to
parse correctly. While OCSP URLs and CA Issuers URLs in practice fit in two bytes,
there is no assertion or documented invariant enforcing this. The function should
`panic!` (or return an error) for lengths >= 0x10000, and an inline comment
should document the `max = 65535` constraint.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `crates/core/controller/src/main.rs:137-139` — Registration token logged in plaintext to structured logging

The one-time registration token is emitted via `tracing::info!`. In production with centralized log aggregation, the token will be captured and accessible to log viewers. Should use `eprintln!` or write to a temporary file with 0o600 permissions.

---

## Code Quality

### Strengths

- **`durations.rs` centralises all timing constants with doc-comments.** There are
  no magic numbers in the background task and scheduler code paths. Constants such
  as `BACKGROUND_TASK_SHUTDOWN_TIMEOUT`, `SERVER_CERT_RENEWAL_WINDOW_DAYS`, and
  `RESTART_NOTIFICATION_SCATTER` are used consistently.

- **Discrete startup phases reduce function complexity.** Each phase function in
  `startup.rs` is independently readable and testable. `reconcile_all_settings`
  is long but follows a repetitive, auditable `ReconcileParams` pattern. The
  `DisplayVec` helper and the `reconcile_setting_vec` / `reconcile_socket_addr`
  wrappers are clean abstractions.

- **`AppError` error type is minimal and domain-appropriate.** The six variants
  (`Config`, `Database`, `Settings`, `Pki`, `Server`, one conversion from
  `CryptoError`) cover exactly the startup failure modes without over-engineering.
  No `String`-wrapped generic errors.

- **`TrackingExecutor` pattern in scheduler tests avoids mocking frameworks.**
  `scheduler/mod.rs:417-424` defines an anonymous `TrackingExecutor` struct with
  an `AtomicBool` flag directly inside the test, keeping the test fully
  self-contained.

- **PKI functions have comprehensive unit test coverage.** `pki.rs` contains 30+
  inline unit tests covering CA generation, server certificate round-trips,
  fingerprint determinism, SAN extraction, AIA/CDP extension embedding, and the
  `validate_ca_pki_addr` matrix (four cases: addr set / extensions present,
  addr set / no extensions, no addr / extensions present, neither set).

#### 2026-02-24 Review

- **All domain-significant durations centralized in `durations.rs` with doc-comments.** Every timing constant is in a single file with documentation, eliminating magic numbers.

### Issues

**[SEVERITY: High]** `crates/core/controller/src/tasks.rs:254-256` — CRL manager registered with `track_abort` instead of `track`

```rust
// main.rs:254-256
let crl_handle = tokio::spawn(Arc::clone(&crl_manager).run(Some(bg.child_token())));
bg.track_abort("crl-manager", crl_handle);
```

The CRL manager's `run` method accepts an `Option<CancellationToken>` and is
passed `Some(bg.child_token())`, so it can respond to cancellation. However, by
registering it with `track_abort` instead of `track`, `BackgroundTasks::shutdown`
calls `handle.abort()` on it directly — before the token is cancelled — without
waiting for a clean exit. If the CRL manager is mid-write to the on-disk TLS
configuration (`server.crt`, `server.key`) when aborted, the write is torn,
producing a corrupted or zero-length file. On the next startup the controller
will fail to load the server certificate. The fix is to change
`bg.track_abort("crl-manager", crl_handle)` to `bg.track("crl-manager", crl_handle)`,
relying on the already-wired `CancellationToken` path.

**[SEVERITY: Medium]** `crates/core/controller/src/tasks.rs:98-104` — 5-second shutdown timeout may be too short for `release_all_claims`

```rust
// tasks.rs:98-104 — applied per task
if tokio::time::timeout(durations::BACKGROUND_TASK_SHUTDOWN_TIMEOUT, handle)
    .await
    .is_err()
{
    tracing::warn!("{name} did not complete within shutdown timeout");
}
```

`BACKGROUND_TASK_SHUTDOWN_TIMEOUT` is 5 seconds. This timeout applies equally to
the scheduler task, whose cleanup path runs `release_all_claims` — a `UPDATE`
query against the database. Under a saturated DB (e.g., at shutdown during peak
load), 5 seconds may not be enough. If the scheduler task times out, all claims
held by this controller remain locked. The next poll cycle on any controller
instance will not reclaim them for 600 seconds (the `STALE_CLAIM_SECONDS`
window), causing a 10-minute gap in scheduled task execution after every
non-clean shutdown. A domain-appropriate timeout (30–60 seconds) for the scheduler
specifically, distinct from the generic 5-second default, would be safer.

**[SEVERITY: Medium]** `crates/core/controller/src/scheduler/mod.rs:153` — Sequential task execution with no timeout or cancellation awareness

```rust
// scheduler/mod.rs:153
let result = executor.execute(&task).await;
```

All due tasks in a poll cycle are executed serially. There is no per-task timeout
and no `tokio::select!` against the cancellation token inside the execution loop.
A `VersionCheckExecutor` blocked on a slow notification delivery, or a
`ServiceCertCheckExecutor` blocked on a DB query, stalls the entire poll cycle.
More critically, a graceful shutdown that arrives while a task is executing cannot
interrupt it — the `token.cancelled()` arm is only checked between interval ticks
in the outer `tokio::select!`. Execution could hold the task claim for the full
600-second stale window.

The recommended fix is to wrap each execution with a per-task timeout:

```rust
let result = tokio::time::timeout(TASK_EXECUTION_TIMEOUT, executor.execute(&task)).await
    .unwrap_or_else(|_| Err(report!(SchedulerError::Timeout)));
```

Adding a second `tokio::select!` branch on `token.cancelled()` inside the loop
would also allow cooperative shutdown mid-cycle.

#### 2026-02-24 Review

**[SEVERITY: Low]** `crates/core/controller/src/pki.rs:543` — `bundle_from_pem` takes `String` parameters where `&str` would suffice for key parsing

Ownership is eventually needed for the struct, but the parameter naming could clarify this.

---

## Tests

### Strengths

- **`#[tokio::test(start_paused = true)]` used correctly in executor tests.**
  `CaRotationCheckExecutor` tests in `scheduler/executors/ca_rotation_check.rs`
  all use `start_paused = true`, making the `tokio::time::timeout` assertions
  deterministic regardless of CI load.

- **`TrackingExecutor` + `AtomicBool` pattern for scheduler integration.**
  `scheduler/mod.rs` tests verify end-to-end task claim, execution, and release
  semantics without requiring real executor implementations. The
  `scheduler_poll_cycle_executes_registered_task` test asserts `run_count == 1`
  and `locked_by.is_none()` after a cycle, confirming the full claim lifecycle.

- **Reverse-proxy integration tests with six proxies.** The `tests/reverse_proxy/`
  suite covers Nginx, HAProxy, Caddy, Envoy, and Traefik for both CRL and OCSP
  revocation scenarios. Every test carries `#[ignore = "Docker integration test…"]`
  with an exact `cargo test` runbook command, ensuring they are not run in CI
  accidentally.

- **`claim.rs` unit tests are thorough.** The tests cover claim acquisition,
  double-claim rejection, release with success, release with error (run_count not
  incremented), `find_due_tasks` filtering, `release_all_claims` scoped to
  controller, and `trigger_immediate`. All use an in-memory SQLite DB with no
  inter-test leakage.

#### 2026-02-24 Review

- **`reconcile_setting` tests cover all five branches.** Five tests systematically cover: no DB + no CLI = default, no DB + CLI = CLI value, DB exists + no CLI = DB value, DB exists + CLI differs + no force = DB wins, DB exists + CLI differs + force = CLI wins. Each verifies both return value and persisted DB state.

### Issues

**[SEVERITY: High]** `crates/core/controller/src/scheduler/mod.rs:265` — Real `tokio::time::sleep(150ms)` in `scheduler_run_exits_on_cancellation`

```rust
// scheduler/mod.rs:264-266
tokio::spawn(async move {
    tokio::time::sleep(Duration::from_millis(150)).await;
    token_clone.cancel();
});
```

This test burns 150ms of real wall-clock time on every run. On a loaded CI agent,
the 5-second outer `timeout` is generous but the inner sleep itself is sensitive to
scheduler jitter. The test should use `#[tokio::test(start_paused = true)]` with
`tokio::time::advance(Duration::from_millis(150))` to make the timing
deterministic and eliminate wall-clock overhead.

**[SEVERITY: High]** `crates/core/controller/tests/reverse_proxy/nginx_ocsp.rs:46` — Real `sleep(1 second)` inside Docker tests

```rust
// nginx_ocsp.rs:46 (and lines 164, 297)
tokio::time::sleep(std::time::Duration::from_secs(1)).await;
```

Three OCSP test scenarios wait a fixed 1 second for Nginx to initialise its OCSP
cache. This is fragile on slow or loaded hosts where Nginx may not be ready within
1 second, causing intermittent failures. The pattern should be replaced with a
retry loop polling a `/healthz` endpoint (or the test port directly) with a
short sleep between attempts and a total timeout, which is more robust and equally
readable.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `crates/core/controller/src/reconcile.rs:157,182,206,237,268` — Five `reconcile_setting` tests use `#[tokio::test]` without `start_paused = true`

Per `testing.md`, all async tests require `start_paused = true`.

**[SEVERITY: Medium]** `crates/core/controller/src/scheduler/claim.rs:253,272,285,309,332,342,354,374` — Eight scheduler claim tests use `#[tokio::test]` without `start_paused = true`

Notably, `recover_stale_claims_only_old_enough` tests time-dependent logic but uses `OffsetDateTime::now_utc()` with manual arithmetic rather than virtual time.

**[SEVERITY: Low]** `crates/core/controller/tests/reverse_proxy/nginx_ocsp.rs:424` — TOCTOU port reservation race

`reserve_port()` binds a listener, reads its port, then drops the listener before
starting the OCSP responder on that port. Another process on the same host can bind
the port in the window between the drop and the `bind`. This is a low-risk issue in
CI but should be replaced with a pattern that passes the already-bound listener
directly to the OCSP responder.

**[SEVERITY: Low]** Scheduler `poll_cycle` tests use `#[tokio::test]` for tests that are purely DB I/O

`scheduler_poll_cycle_empty_db_leaves_no_locked_tasks` and related tests are
annotated `#[tokio::test]` but do not await any async I/O that requires the Tokio
runtime beyond what `block_on` would provide. This is a minor style inconsistency
and has no correctness impact.

---

## High Availability

### Strengths

- **`try_claim` uses a single atomic `UPDATE WHERE locked_by IS NULL`.**
  `claim.rs:22-37` updates `locked_by` and `locked_at` in a single SQL statement
  filtered on `LockedBy.is_null()`. This is a TOCTOU-free optimistic lock: two
  concurrent controllers racing to claim the same task will have exactly one
  winner determined by the DB engine, not by application-level reads.

- **`release_all_claims` on cancellation avoids the stale-claim window.**
  `scheduler/mod.rs:85-91` runs `release_all_claims` in the `token.cancelled()`
  arm, releasing all holds before the scheduler task exits. This means a clean
  shutdown does not trigger the 600-second stale recovery wait on restart.

- **`recover_stale_claims` as a safety net for crashed instances.** Every poll
  cycle calls `recover_stale_claims` before finding due tasks, releasing locks
  held for more than 10 minutes. This bounds the worst-case re-execution delay
  after an unclean shutdown.

- **`broadcast_server_restarting_scattered` prevents thundering herd.**
  `tasks.rs:76-83` spreads `ServerRestarting` messages across a configurable
  window (`RESTART_NOTIFICATION_SCATTER`) so agents do not all reconnect
  simultaneously after a controller restart.

- **`spawn_settings_reload` skips first tick.** The settings and CA reload tasks
  call `interval.tick().await` before entering the loop, preventing a redundant
  reload of settings that were just loaded during startup.

- **Cross-instance CA update propagation via version counter.** `spawn_ca_reload`
  polls `PkiCaVersion` in the settings table. When the version advances (because
  another instance rotated the CA), it reloads CA state from the database and
  rebuilds the CRL manager and TLS config without a restart. This keeps all
  controller instances in the fleet in sync without inter-process communication.

#### 2026-02-24 Review

- **CRL manager uses version-gated polling with local Notify.** `src/crl_manager.rs:234-282` — Combines 60-second periodic poll with instant local rebuilds via `Notify`.
- **All HA-critical timing constants are centralised with documentation.** `src/durations.rs:1-34` — Makes HA tuning auditable from a single location.

### Issues

**[SEVERITY: High]** `crates/core/controller/src/tasks.rs:254-256` — CRL manager abort may corrupt TLS config on disk

See Code Quality section. This is also an HA concern: a corrupted `server.crt` or
`server.key` on disk causes every subsequent startup attempt to fail until an
operator manually removes the file, creating extended downtime even though the
controller binary itself is healthy.

**[SEVERITY: Medium]** `crates/core/controller/src/scheduler/mod.rs:153` — Scheduler task execution blocks shutdown

See Code Quality section. A hung executor holds the scheduler task alive until the
5-second `BACKGROUND_TASK_SHUTDOWN_TIMEOUT` expires. The claim is not released
before the timeout fires, meaning the task stays locked for up to 600 seconds on
the next restart.

**[SEVERITY: Medium]** `crates/core/controller/src/tasks.rs:98-104` — Shutdown timeout too short for DB-dependent cleanup

See Code Quality section. The 5-second uniform timeout for all background tasks is
too short for the scheduler's `release_all_claims` DB write under transient DB
pressure.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `crates/core/controller/src/db/mod.rs:20-25` — Database connection pool hardcoded at max_connections=10 with no runtime configurability

All timeouts (8 seconds) and pool sizes are hardcoded literals. Under connection exhaustion, cascading failures affect all subsystems simultaneously. These should be configurable.

**[SEVERITY: Low]** `crates/core/controller/src/crl_manager.rs:202-223` — CRL manager `reload_tls_config` acquires two RwLock read guards sequentially without documented ordering

Creates a consistency window during concurrent CA rotation + server cert renewal. The single-threaded invariant is not documented.

---

## Database

### Strengths

- **Squashed single migration with a correct `down()` path.**
  `migration/m20260209_000001_initial.rs` creates all tables in a single
  migration (pragmatic for a pre-1.0 product) and `down()` drops them in correct
  reverse FK order. The second migration (`m20260222_000002_add_machine_id.rs`)
  demonstrates the correct additive pattern for future schema changes.

- **Partial unique index for soft-deleted records.** Filtered unique indexes
  (`WHERE deactivated_at IS NULL`) prevent duplicate slugs among active records
  without conflicting with soft-deleted rows. SeaORM's API limitation requires
  these to be expressed as raw SQL, which is correctly documented with inline
  comments.

- **CA rotation uses a CAS (`UPDATE WHERE value = expected_fp`) inside a
  transaction.** `rotate_managed_ca` wraps all mutations in an explicit
  transaction, uses `update_setting_string_cas` for the fingerprint pointer swap,
  and returns `rotated: false` on any conflict. Double-rotation is structurally
  impossible.

- **CA version counter with `bump_setting_i64`.** The CA version is incremented
  atomically inside the rotation transaction, giving cross-instance reload tasks a
  simple, cheap change-detection probe.

### Issues

**[SEVERITY: Medium]** `crates/core/controller/src/migration/m20260209_000001_initial.rs:25-41` — `tenants.slug` has both `string_uniq()` and an explicit `Index::create()`

```
string_uniq(Tenants::Slug)          // implicit unique index
…
Index::create().name("idx_tenants_slug").col(Tenants::Slug)   // duplicate index
```

`string_uniq()` in SeaORM already creates a unique index on the column. Adding a
second explicit `idx_tenants_slug` index on the same column creates a duplicate
index that wastes write overhead (two B-tree updates per INSERT/UPDATE on `slug`).
The explicit `Index::create()` call should be removed; the uniqueness constraint
from `string_uniq` is sufficient. The same pattern appears for `users.email`.

---

## Coding Standards

### Strengths

- **`edition = "2024"` with workspace inheritance for all standard fields.**
  `license`, `authors`, `repository`, and `version` all use `workspace = true`.

- **Consistent use of `bail!`, `report!`, and `context` / `context_to`.** No
  `Report::new()` anti-pattern. `AppError` conversion is implemented through
  `impl_report_conversion!`, maintaining a single conversion site.

- **No `unwrap()` in startup, shutdown, or scheduler paths.** The startup
  sequence propagates all errors as `Report<AppError>` and exits via
  `std::process::ExitCode::FAILURE` with a formatted error message. No silent
  panics.

- **Signal handling uses `tokio::signal::unix::signal` typed by `SignalKind`.**
  `main.rs:364-371` handles `SIGTERM`, `SIGINT`, and `SIGUSR1` with fully typed
  signal kinds and error-propagating setup, rather than `ctrlc` or raw libc
  signal registration.

- **`tracing_subscriber` initialised in the binary's `main`, not in a library.**
  `main.rs:77-82` calls `tracing_subscriber::fmt().with_env_filter(…).init()`.
  This is the correct location; the binary owns subscriber initialisation, unlike
  the shared `service-sdk` crate which incorrectly performs this in a library.

#### 2026-02-24 Review

- **All domain-significant duration constants are centralized with documentation.** `src/durations.rs` — No magic numeric literals for time values.

### Issues

**[SEVERITY: Medium]** `crates/core/controller/Cargo.toml:45,50-51` — `sea-orm-migration`, `cron`, and `chrono` not in `[workspace.dependencies]`

Restated here from Architecture: three direct dependencies are declared with
inline version strings, bypassing workspace-level version governance. During the
RC series of `sea-orm`, even a patch-level drift between `sea-orm` and
`sea-orm-migration` can cause runtime migration errors. `cron`'s indirect
`chrono` requirement introduces a second date-time library into the binary without
a workspace-pinned constraint.

**[SEVERITY: Low]** `crates/core/controller/src/scheduler/mod.rs:18` — `DEFAULT_POLL_INTERVAL_SECS` is a module-private constant that duplicates domain knowledge in `durations.rs`

`durations.rs` already contains all other timing constants with doc-comments.
`DEFAULT_POLL_INTERVAL_SECS = 15` is a plain `const u64` embedded in
`scheduler/mod.rs` without documentation. It should be moved to `durations.rs` as
`pub(crate) const SCHEDULER_POLL_INTERVAL: Duration` for consistency and
discoverability.

#### 2026-02-24 Review

**[SEVERITY: Low]** `crates/core/controller/src/db/config.rs:23,48,53,58` — Four `#[cfg(not(feature = "db-*"))]` blocks are defensive guards with no documentation

These are necessary for clear error messages but the pattern contradicts the "features are additive only" principle. Each should carry a justification comment.

---

## Extensibility

### Strengths

- **`TaskExecutor` trait is minimal and object-safe.** The single required method
  `async fn execute(&self, task: &Model) -> Result<()>` has no associated
  constants or types, making it fully object-safe. Adding a new executor is a
  two-step operation: implement the trait, then call `sched.register(TaskType, Box::new(…))`.

- **`ScheduledTaskType` enum drives both DB seed data and runtime executor
  registration.** There is a clear mapping between the enum variant, the migration
  seed row, and the executor registered in `main.rs`. A new task type requires
  additions in only two places: a new enum variant (in `shared-db`) and a
  `sched.register(…)` call in `main.rs`.

- **`PkiRuntime` struct decouples PKI initialisation from `AppState` building.**
  All PKI-related fields — `ca_tx`, `ca_rx`, `ca_key_store`, `rustls_config`,
  `crl_manager`, etc. — flow through a single intermediate struct. This makes it
  straightforward to add a new PKI-related field without changing the function
  signature of `init_pki_runtime`.

#### 2026-02-24 Review

- **`TaskExecutor` trait is minimal and correctly object-safe.** `src/scheduler/executor.rs:9` — Single method, `Send + Sync`, works with `Box<dyn TaskExecutor>`.

### Issues

**[SEVERITY: Medium]** `crates/core/controller/src/scheduler/mod.rs:48` — `executors` field is `HashMap<ScheduledTaskType, Box<dyn TaskExecutor>>`; no check for duplicate registration

`Scheduler::register` uses `HashMap::insert`, which silently replaces any
previously registered executor for a given task type with no warning. If two
`sched.register(ScheduledTaskType::VersionCheck, …)` calls appear in `main.rs`
(for example after a refactor), the second silently shadows the first. The method
should either `debug_assert!` that the key is not already present, or return
`Option<Box<dyn TaskExecutor>>` to expose the displacement to the caller.

#### 2026-02-24 Review

**[SEVERITY: Medium]** `crates/core/controller/src/scheduler/executor.rs:9` — `TaskExecutor` trait has no registration or discovery mechanism for new task types

Adding a new scheduled task requires creating a new executor, adding a match arm, and registering manually. Unlike the `register_providers!` macro, there is no compile-time check ensuring all `ScheduledTaskType` variants have executors.

**[SEVERITY: Low]** `crates/core/controller/src/scheduler/claim.rs:160-164` — `find_due_tasks` is scoped to a single `tenant_id`

```rust
.filter(scheduled_task::Column::TenantId.eq(tenant_id))
```

The `SchedulerConfig` carries a `tenant_id` and the query enforces it. Any future
multi-tenant extension will require a redesign of how tasks are discovered and
assigned. This limitation is not documented in the `Scheduler` struct doc-comment.
Adding a comment noting "single-tenant by design; multi-tenant requires scheduler
redesign" would make the limitation visible at the definition site.
