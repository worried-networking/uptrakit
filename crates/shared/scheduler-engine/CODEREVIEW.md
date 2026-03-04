# Code Review: uptrakit-shared-scheduler-engine

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-shared-scheduler-engine` (~2,609 LoC) implements the database-backed distributed task
scheduler used by both `uptrakit-controller` (embedded) and `uptrakit-scheduler` (standalone
binary). The core claim/release cycle uses TOCTOU-free optimistic locking and stale-claim recovery,
which are strong correctness properties. The `SchedulerNotifier` ORM coupling has been fixed
(callers now load and pass a `MqttSoftwareStatesPayload`), and the sequential execution model
now uses a `JoinSet` for parallel task execution within each poll cycle. The anonymous tuple
`FetchGroupValue` accumulator in `version_check.rs` has been refactored to named structs
(`AgentAssignmentRow`, `HostPackageAssignmentRow`), eliminating index-fragility. All five executors now have tests.

## Architecture

### Strengths

- `src/claim.rs` -- `try_claim` uses a single `UPDATE WHERE locked_by IS NULL` atomic operation.
  Two concurrent scheduler instances racing to claim the same task are handled correctly by
  the DB engine -- exactly one wins, no separate read or TOCTOU window.
- `src/scheduler.rs` -- Single-tenant by design: `SchedulerConfig` carries a `tenant_id` and all
  `find_due_tasks` queries filter by it.

### Issues

**[MEDIUM]** Depends on `plugin-infrastructure-registry` which compiles all plugins. The
scheduler only needs plugin capabilities (for version-check executor configuration), not
full plugin execution code. This heavyweight dependency pulls the entire plugin tree --
including all release and package-manager plugins -- into the scheduler binary. Extracting
a `plugin-capabilities` trait crate or feature-gating the registry dependency would reduce
compile time and binary size for the standalone scheduler deployment.

## Security and Safety

### Strengths

- Zero `unsafe` blocks. The optimistic locking design means no application-level mutex is
  needed for concurrent access.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/claim.rs` -- Unit tests are thorough: claim acquisition, double-claim rejection, release
  with success, release with error (run_count not incremented), `find_due_tasks` filtering,
  `release_all_claims` scoped to controller, and `trigger_immediate`. All use in-memory SQLite
  with no inter-test leakage.
- `thiserror`-derived error types with typed variants; no `Result<T, String>` at library
  boundaries.

### Issues

**[LOW]** `src/scheduler.rs` -- `DEFAULT_POLL_INTERVAL_SECS` is a bare `const u64` without a
doc comment. Promote to a typed `Duration` constant with documentation, matching the
`durations.rs` pattern used in the controller.

## High Availability

### Strengths

- `src/claim.rs` -- `recover_stale_claims` releases locks held for more than 600 s, providing
  automatic recovery after a scheduler crash without manual intervention. The stale window is a
  named constant with a doc comment.
- `src/scheduler.rs` -- `release_all_claims` is called in the `cancelled()` arm of the shutdown
  loop, ensuring clean shutdown does not leave locks that block the 600 s stale window.

### Issues

**[LOW]** `src/scheduler.rs` -- The shutdown sequence sends the shutdown signal before awaiting
the in-progress task's `JoinHandle`. This creates a window where the task's `release_claim`
DB write may be abandoned mid-execution. Await the handle before propagating shutdown.

## Coding Standards

### Strengths

- Uses workspace lints. Zero `#[allow(clippy::...)]` suppressions.
- `thiserror`-derived errors with semantic variants.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `TaskExecutor` trait enables adding new task types by implementing a single trait method.

### Issues

**[LOW]** `TaskExecutor` trait has no compile-time check ensuring all `ScheduledTaskType`
variants have registered executors. Unlike `register_plugins!`, there is no mechanism to catch
a missing executor at startup -- the scheduler will silently skip tasks of an unregistered type.

## Tests

### Strengths

- `src/claim.rs:200-464` -- Nine `#[tokio::test]` tests (no `start_paused`, correct since no
  Tokio time APIs are used) exercise the claim lifecycle against an in-memory SQLite database:
  `try_claim` acquires a claim, double-claim returns false, `release_claim` on success and on
  error, `find_due_tasks` filters correctly by `next_run_at`, `recover_stale_claims` releases
  old locks, `release_all_claims` scoped to controller, `trigger_immediate` updates
  `next_run_at`, and unknown task types are excluded from `find_due_tasks`. Inter-test
  isolation is achieved via separate `setup_db()` calls.
- `src/cron_utils.rs:40-120` -- Nine synchronous tests cover `next_cron_time` for valid
  expressions, invalid cron strings, six-field expressions, `normalize_cron` prepend behavior,
  and `validate_cron` for valid and invalid inputs.
- `src/software_states.rs:180-222` -- Five tests cover `extract_release_info` including `None`
  input, both fields present, missing fields, only URL, and non-string value handling.
- `src/executors/version_check.rs:612-659` -- Five unit tests cover `merge_config` logic for
  object merge, non-object override, non-object base, empty override, and nested object
  (shallow, not deep merge). These are plain `#[test]` without a runtime, correct since
  `merge_config` is a pure synchronous function.
- `src/executors/crl_renewal.rs:32-91` -- One `#[tokio::test]` (no `start_paused`, correct)
  exercises the `CrlRenewalExecutor::execute` success path with a mock `SchedulerNotifier`,
  verifying `signal_crl_renewal` is called.
- `src/scheduler.rs:282-698` -- Eight scheduler-level tests cover `SchedulerConfig` defaults,
  empty-DB poll cycle, cancellation exit, no-due-tasks cycle, unregistered task type skipping,
  debug-mode double-registration panic, registered task execution with claim release and
  run_count increment, and mid-execution cancellation with claim release verification.
- `src/ca_utils.rs:29-47` -- Three tests cover `should_rotate_ca` for invalid PEM, empty PEM,
  and `cert_not_after` returning `None` for garbage input.

### Issues

**[MEDIUM]** `src/executors/version_check.rs` -- `run_controller_side_fetch_releases` and
`send_agent_assignments` (the two primary async methods) have no tests. These methods
contain the core business logic for version checking and are the most likely source of
regressions when new plugin types or assignment logic are added.

## Consistency

### Strengths

- `src/claim.rs` -- `try_claim`, `release_claim`, `recover_stale_claims`, and
  `release_all_claims` all use the same `scheduled_task::Entity::update_many()` / `.filter()` /
  `.exec(db)` / `.context_to::<SchedulerError>()` pipeline. The DB update pattern is uniform
  across all claim lifecycle operations, making the locking model easy to audit.
- `src/scheduler.rs:199-278` -- Within a single poll cycle, the cancellation-aware task
  execution loop uses the same
  `tokio::select! { biased; _ = token.cancelled() => { release_claim("shutdown") }; res = timeout(executor.execute) => { ... } }`
  structure for every spawned task. Error handling after execution (log on failure, then call
  `release_claim`) is applied identically regardless of which executor ran.
- `src/scheduler.rs:17` and `src/scheduler.rs:24` -- `DEFAULT_POLL_INTERVAL_SECS` and
  `TASK_EXECUTION_TIMEOUT` are the only two numeric constants in the file. Both are named
  module-level constants rather than inline literals, consistent with the `durations.rs`
  pattern in the controller.

### Issues

**[MEDIUM]** `src/scheduler.rs:17` vs `src/claim.rs:12` -- `DEFAULT_POLL_INTERVAL_SECS` is a
bare `const u64` while `STALE_CLAIM_SECONDS` in `claim.rs` is also a bare `const i64`. Neither
is typed as `Duration`, unlike `TASK_EXECUTION_TIMEOUT` on line 24 which correctly uses
`Duration::from_secs(...)`. The two raw-integer constants are inconsistent with the one
`Duration` constant in the same crate and with the controller's `durations.rs` convention of
typed `Duration` constants with doc-comments throughout.

**[LOW]** `src/scheduler.rs:107-113` (cancellation arm, `release_all_claims`) vs
`src/scheduler.rs:217-231` (mid-task cancellation, `release_claim` with shutdown reason) --
The shutdown path in the main loop (`token.cancelled()` outer arm) calls `release_all_claims`
to bulk-release every claim owned by this controller. The shutdown path inside a running task
calls `release_claim` for only the one in-flight task. This is correct, but the outer
`release_all_claims` does not set `last_error` on released tasks, while the inner
`release_claim` records `"scheduler shutdown during execution"` as the error string. A comment
on the outer arm noting that it intentionally does not set `last_error` (to avoid falsely
marking tasks as errored on clean shutdown) would make this asymmetry self-documenting.

**[LOW]** `src/notifier.rs` -- `SchedulerNotifier` has six async methods, but the
`VersionCheckExecutor` uses `send_agent_assignments` (via `notifier.broadcast`) while
`CaRotationCheckExecutor` calls `notifier.signal_ca_rotation` and `CrlRenewalExecutor` calls
`notifier.signal_crl_renewal`. The trait methods for CA and CRL are named `signal_*` (verb
"signal") while the software-state push is named `push_software_states_for_tenant` (verb
"push") and the general delivery methods are `send_*` / `broadcast`. The verb prefix varies
across methods without a consistent naming convention, making the intent of each method harder
to infer from the trait definition alone.

## Maintainability

### Strengths

- `src/cron_utils.rs` -- The cron normalization and validation helpers are cleanly isolated with
  doc comments explaining the 5-field vs 6-field distinction. The module has nine tests covering
  valid/invalid expressions, field normalization, and an invalid expression that returns `None`.
- `src/notifier.rs` -- `SchedulerNotifier` trait methods all have doc comments. The comment on
  `push_software_states_for_tenant` explicitly documents the caller's responsibility to pre-load
  the payload, which is a non-obvious API design decision.
- `src/claim.rs` -- Constants (`STALE_CLAIM_SECONDS`) are documented with rationale.
  The claim-and-release pattern is explained in the struct-level doc.

### Issues

**[MEDIUM]** `src/scheduler.rs:17` -- `DEFAULT_POLL_INTERVAL_SECS: u64 = 15` is a bare numeric
constant with no doc comment explaining its relationship to task latency or the STALE_CLAIM
window. A reader tuning the scheduler cannot tell from the constant alone whether 15 seconds is
a hard constraint or a conservative default. Promote to a typed `Duration` constant with a doc
comment explaining the relationship (poll interval should be much shorter than the stale claim
threshold of 600 s), and move to a `durations` module for discoverability.

**[LOW]** `src/executors/` -- Four executor files (`auth_cleanup.rs`, `crl_renewal.rs`,
`service_cert_check.rs`, `stale_lease_cleanup.rs`) have no module-level doc comment. Each
executor performs a distinct maintenance operation but the file gives no indication of when it
runs, what it touches, or what failure means. A single-sentence module doc (`//! Cleans up
expired OIDC authorization codes and revoked JWT entries older than N days.`) would make the
executor's purpose visible without reading the implementation.

## Database

### Strengths

- `src/claim.rs:24-39` -- `try_claim` is a single-statement `UPDATE WHERE locked_by IS NULL`.
  The WHERE clause and the SET together form an atomic compare-and-swap at the DB engine level.
  Two concurrent schedulers racing on the same task row will produce exactly one winner
  (`rows_affected == 1`) without any application-level mutex or read-before-write. This is the
  correct implementation of optimistic distributed locking with a DB.
- `src/claim.rs:46-100` -- `release_claim` increments `run_count` only on the success branch
  (`if result.is_ok()`). A task that times out or errors is released with `run_count` unchanged,
  making the counter a reliable measure of successful completions rather than total attempts.
  `last_error` is set on the failure branch and cleared (`None`) on success, giving operators a
  single column to inspect for the most recent failure reason.
- `src/claim.rs:103-128` -- `recover_stale_claims` is a bulk `UPDATE WHERE locked_at < cutoff`
  with no per-row iteration. It sets `last_error` to a descriptive string
  (`"released: stale claim (controller may have crashed)"`) so that a post-mortem DB query on
  `scheduled_tasks WHERE last_error IS NOT NULL` reveals which tasks were recovered after a
  crash rather than released normally.
- `src/claim.rs:131-150` -- `release_all_claims` is scoped by `controller_id`. A clean
  shutdown releases only the rows owned by the shutting-down instance, leaving claims held by
  other controllers untouched. This is the correct multi-instance shutdown behaviour.
- `src/claim.rs:158-176` -- `find_due_tasks` pre-computes the set of known `ScheduledTaskType`
  variants via `ScheduledTaskType::iter()` and passes them as an `IS IN (...)` filter before
  the query executes. Unknown task types (written by a newer controller during a rolling upgrade)
  are excluded at the query level rather than causing deserialization failures in the Rust ORM
  layer. The test at `src/claim.rs:424-463` validates this behaviour by injecting a raw SQL row
  with an unknown type and confirming it is absent from the result.
- `src/executors/auth_cleanup.rs` -- `AuthCleanupExecutor` wraps all `DELETE WHERE expires_at <
  now` statements in a single transaction. Each delete is a single DB round-trip with no
  per-row application logic. The transaction makes the cleanup cycle atomic (all-or-nothing)
  and reduces PostgreSQL round-trips. This is the correct bulk-delete pattern for TTL cleanup.
- `src/executors/stale_lease_cleanup.rs:30-36` -- `StaleLeaseCleanupExecutor` deletes all MQTT
  leases with `heartbeat_at < cutoff` in a single `DELETE WHERE` statement. The cutoff constant
  `STALE_AFTER_SECS = 60` matches the heartbeat interval documented in the MQTT lease
  coordinator, making the cleanup boundary self-consistent.
- `src/executors/service_cert_check.rs:36-43` -- The expiry query filters by three predicates
  in a single statement: `revoked_at IS NULL`, `not_after <= renewal_cutoff`, and
  `not_after > now`. The composite predicate avoids a secondary application-side filter and is
  efficient with an index on `(revoked_at, not_after)`.

### Issues

**[MEDIUM]** `src/executors/service_cert_check.rs:36-43` -- The expiry query has no index
coverage guarantee on `(revoked_at, not_after)` for the `service_certificates` table. The
initial migration does not add an index on `not_after` or a composite index on
`(revoked_at, not_after)`. As the `service_certificates` table grows (one row per issued
certificate per service, accumulating over the CA lifecycle), this query will perform a full
table scan on every scheduler poll cycle. An index on `(revoked_at, not_after)` or a partial
index on `not_after WHERE revoked_at IS NULL` should be added in a follow-up migration.

**[LOW]** `src/claim.rs:104` -- `recover_stale_claims` computes the cutoff as
`OffsetDateTime::now_utc() - time::Duration::seconds(STALE_CLAIM_SECONDS)`. The constant
`STALE_CLAIM_SECONDS` is an `i64` (seconds since epoch-duration arithmetic), which differs from
the `Duration`-typed constants used elsewhere in the codebase. The `STALE_CLAIM_SECONDS` value
is not the same unit as `TASK_EXECUTION_TIMEOUT` (which is already a `Duration`). If a future
developer changes `STALE_CLAIM_SECONDS` without knowing that `TASK_EXECUTION_TIMEOUT` should
remain less than the stale threshold, tasks could be reclaimed before their execution timeout
fires. A doc comment on `STALE_CLAIM_SECONDS` noting that it must exceed
`TASK_EXECUTION_TIMEOUT.as_secs()` would make the constraint visible.

**[LOW]** `src/executors/stale_lease_cleanup.rs:11` -- `STALE_AFTER_SECS = 60` is a
module-private constant with no doc comment connecting it to the heartbeat interval configured
in the MQTT lease coordinator. If the heartbeat interval changes, this constant must be updated
in tandem, but there is no cross-reference making that dependency visible. A doc comment
(`/// Must exceed the MQTT lease heartbeat interval (currently 30 s) to avoid evicting healthy
leases`) would make the invariant self-documenting.
