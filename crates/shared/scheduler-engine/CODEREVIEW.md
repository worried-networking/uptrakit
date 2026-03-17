# Code Review: `uptrakit-scheduler-engine`

- Review date: 2026-03-17
- Scope: full 14-dimension review of all ~18 source files in the scheduler engine crate

## Summary

The scheduler engine is a well-designed HA-safe task execution framework. The claim/release
mechanism, stale-lease recovery, concurrent task execution via JoinSet, and shutdown semantics
(drain vs. abort) are all solid. Test coverage is strong for the claim lifecycle, cancellation
behavior, and external-task yielding. This review cycle validated the existing finding about
missing orphaned-update cleanup and added one new finding about the `StaleLeaseCleanupExecutor`
being a no-op stub, plus observations about the controller-side fetch architecture.

## Strengths

- Optimistic task claiming (`try_claim`) is a single atomic `UPDATE ... WHERE locked_by IS NULL`
  -- no TOCTOU race between check and claim.
- Stale-claim recovery (`recover_stale_claims`) runs on every poll cycle with a 10-minute cutoff,
  correctly shorter than the 2-hour execution timeout, so crashed controllers release tasks
  within minutes.
- Shutdown semantics are explicit: `drain` stops new claims but lets running tasks finish;
  `abort` interrupts running tasks and releases their claims. Both paths call
  `release_all_claims`.
- The `find_due_tasks` filter for known `ScheduledTaskType` variants ensures forward
  compatibility during rolling upgrades -- unknown task types from newer controller versions
  are silently skipped.
- JoinSet-based concurrent execution within a poll cycle prevents slow executors from blocking
  other due tasks.
- `release_claim` correctly differentiates success (increments `run_count`, sets `last_run_at`)
  from failure (records `last_error`, does not increment `run_count`).
- External-task yielding (`should_yield_external` + `is_internal()`) cleanly separates
  controller-internal tasks (CRL, CA, cert check) from tasks that an external scheduler can
  handle.
- `FetchReleasesExecutor` runs Phase A (controller-side) and Phase B (agent-side) concurrently
  via `tokio::join!`, reducing overall wall-clock time.
- Phase A group key correctly includes `assignment_config` to prevent Docker platform-specific
  digest mixing across different architectures sharing the same plugin config.
- `DiscoverSoftwareExecutor` uses bulk queries for host rows, allowlists, and plugin configs
  with `tokio::try_join!` for parallel loading.
- Test coverage exercises: claim/release lifecycle, CAS contention, stale recovery,
  cancellation mid-execution, double-registration detection, external-task skipping, and
  internal-task execution under yield conditions.

## Active Findings

### [HIGH] The engine still has no executor for orphaned update cleanup

- **Dimension**: high availability, database
- **Scope**: `crates/shared/scheduler-engine/src/executors/mod.rs`
- **Description**: the engine owns stale scheduler-lease cleanup and is the natural place to
  recover stale update state. No executor exists to scan for `InProgress` update_history records
  older than a configurable threshold and transition them to `Failed`.
- **Why it matters**: `mark_in_progress_as_failed()` in web-api-queries only runs when an agent
  reconnects. If the agent never reconnects (dead host, permanent network partition), the
  `InProgress` record is never cleaned up. Queued updates behind it are never promoted.
  Software states MQTT feed shows `update_in_progress: true` indefinitely.
- **Failure scenario**: agent crashes permanently during an update. The update_history record
  stays `InProgress` forever. The host appears permanently busy. Scheduled maintenance keeps
  running but the host's update queue is permanently blocked.

### [MEDIUM] `StaleLeaseCleanupExecutor` is a no-op stub registered for backward compatibility

- **Dimension**: maintainability, crate structure
- **Scope**: `crates/shared/scheduler-engine/src/executors/stale_lease_cleanup.rs`
- **Description**: the executor is a registered no-op that exists solely to prevent scheduler
  errors from pre-existing `scheduled_tasks` rows with `task_type = 'stale_lease_cleanup'`.
  It consumes a poll-cycle claim slot, writes a successful `run_count` increment, and advances
  `next_run_at` every interval -- all for zero useful work.
- **Why it matters**: this is dead code that accumulates `run_count` and `last_run_at` updates
  in the database on every execution interval. The `StaleLeaseCleanupExecutor::new` takes a
  `DatabaseConnection` parameter that is immediately discarded, which is misleading.
- **Failure scenario**: no correctness issue, but operational confusion: monitoring dashboards
  show `stale_lease_cleanup` running successfully every N minutes, suggesting active work is
  being done when none is. A future developer may add logic to this executor not realizing the
  underlying table was dropped.

### [LOW] `query_agent_assignment_rows` joins through 5 tables without tenant scoping on host

- **Dimension**: performance, security
- **Scope**: `crates/shared/scheduler-engine/src/executors/queries.rs:query_agent_assignment_rows` (line 49-123)
- **Description**: the query filters by `software_item.tenant_id` but does not filter by
  `host.tenant_id`. The join path `host_software_item_plugin -> host -> service_host -> service`
  does not include a `host.tenant_id` filter. This is not a security issue because the
  `software_item.tenant_id` filter effectively scopes the result set (host_software_item_plugins
  are always created in a tenant context), but the missing filter means the DB planner cannot
  use a tenant-scoped index on the `hosts` table to narrow the join early.
- **Why it matters**: for multi-tenant deployments with many hosts across tenants, the join
  may scan more host rows than necessary before the software_item filter narrows the result.
  Adding `host.tenant_id.eq(tenant_id)` would allow the planner to use a tenant index on
  the hosts table.

### [LOW] Controller-side fetch Phase A DB update loop is sequential per (host_id, software_item_id)

- **Dimension**: performance
- **Scope**: `crates/shared/scheduler-engine/src/executors/fetch_releases.rs` (line ~409-443)
- **Description**: after concurrent fetch jobs complete, the DB update loop iterates over each
  `(host_id, software_item_id)` target and issues one `UPDATE` per target. For a plugin config
  with 100 hosts tracking the same package, this is 100 sequential single-row UPDATEs.
- **Why it matters**: the fetch phase correctly parallelizes API calls, but the DB write phase
  serializes them. For large deployments with many hosts per software item, the write phase
  becomes the bottleneck. A single `UPDATE ... WHERE (host_id, software_item_id) IN (...)`
  batch update would reduce round-trips significantly.
- **Failure scenario**: a GitHub plugin config tracking a popular package across 200 hosts
  issues 200 sequential UPDATE statements after a single `batch_fetch_releases` call. At
  ~1ms per UPDATE on PostgreSQL, this adds ~200ms of sequential DB work that could be reduced
  to ~2ms with a batched update.
