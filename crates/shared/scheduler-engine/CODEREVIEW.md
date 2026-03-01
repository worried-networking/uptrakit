# Code Review: uptrakit-shared-scheduler-engine

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-shared-scheduler-engine` (~2,172 LoC) implements the database-backed distributed task
scheduler used by both `uptrakit-controller` (embedded) and `uptrakit-scheduler` (standalone
binary). The core claim/release cycle uses TOCTOU-free optimistic locking and stale-claim recovery,
which are strong correctness properties. The `SchedulerNotifier` ORM coupling has been fixed
(callers now load and pass a `MqttSoftwareStatesPayload`), and the sequential execution model
now uses a `JoinSet` for parallel task execution within each poll cycle.

## Architecture

### Strengths

- `src/claim.rs` -- `try_claim` uses a single `UPDATE WHERE locked_by IS NULL` atomic operation.
  Two concurrent scheduler instances racing to claim the same task are handled correctly by
  the DB engine -- exactly one wins, no separate read or TOCTOU window.
- `src/scheduler.rs` -- Single-tenant by design: `SchedulerConfig` carries a `tenant_id` and all
  `find_due_tasks` queries filter by it.

### Issues

No architectural issues found.

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
