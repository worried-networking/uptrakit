# Resumable Updates & Uptrakit Self-Update

**Date:** 2026-04-30  
**Status:** Approved

## Problem

Two related gaps:

1. Updates that require a process or host restart (self-update, kernel, firmware) cannot
   complete cleanly — the agent dies mid-execute, the controller marks the record `Failed`
   on reconnect even though the update may have succeeded. No mechanism exists to verify
   success after the process comes back.

2. There is no way to update Uptrakit itself via Uptrakit. Controller-standalone users must
   update out-of-band. Resumable updates are a prerequisite for self-update to be reliable.

---

## Goals

- Updates that require restart are not incorrectly marked `Failed` on agent disconnect.
- After restart, the controller verifies the new version is running and marks the update
  terminal accordingly.
- A discovery plugin auto-discovers uptrakit services on the local host and wires up
  self-update with zero manual configuration.
- No hardcoded binary paths — the plugin queries the running service for its own metadata.
- Existing update semantics for non-resumable updates are unchanged.

---

## Part 1: Resumable Updates

### Concept

A **resumable** update is one where agent disconnect is an expected part of the update
procedure (e.g., the update script replaces the running binary and triggers a restart).
Instead of transitioning `InProgress → Failed` on disconnect, the controller transitions to
a new `AwaitingRestart` status and waits for the agent to reconnect and confirm the new
version is running.

Resumability is a property of the execute_update plugin assignment, set by discovery plugins
at assignment time. It is not user-configurable and not visible in the UI.

---

### Data Model

#### `host_software_item_plugin` — new column

```sql
resumable BOOL NOT NULL DEFAULT FALSE
```

- Meaningful only on rows where `role = 'execute_update'`.
- Set by discovery plugins when creating assignments; never written by user-facing routes.
- Read by the reconnect handler and dispatch layer.

#### `software_item` — new column

```sql
awaiting_restart_timeout INTEGER NULL  -- seconds; NULL = use global default
```

- Controls how long the controller waits in `AwaitingRestart` before giving up.
- Global default: 600 seconds (10 minutes). Suitable for self-update and service restarts.
- Set to larger values (e.g., 604800 = 7 days) for kernel/firmware items where the reboot
  may be deferred to a maintenance window.
- Separate from the existing execution timeout — the two phases have different time budgets.

#### `update_history` — new column

```sql
awaiting_restart_since TIMESTAMPTZ NULL
```

- Set to the current timestamp when the record transitions to `AwaitingRestart`.
- Used as the timeout anchor for `awaiting_restart_timeout` enforcement.
- NULL for all non-resumable updates and all updates that never reach `AwaitingRestart`.
- Not reset by other update_history writes; dedicated field prevents fragile `updated_at`
  dependencies (update_history has no `updated_at` column).

---

### `UpdateStatus` New Variant

`AwaitingRestart` is added to **both** `UpdateStatus` enums:

- `crates/shared/types/src/update_status.rs` — the canonical DB/SeaORM type
- `crates/shared/web-api-types/src/update_history.rs` — the API response copy

Both follow the existing `#[non_exhaustive]` + `FromStr`/`ParseUpdateStatusError` pattern.
`UpdateStatus` is not a wire-received type and does not use `Other(String)`.

`AwaitingRestart` is not a terminal status. It is not a failure. Batch sequencing treats it
as active (blocks the next `Queued` item on the same host).

---

### State Machine

```text
Queued
  │ (host free)
  ▼
Pending
  │ (orchestrator picks up)
  ▼
InProgress ──── agent disconnects, resumable=false ──► Failed
  │
  │ agent disconnects, resumable=true
  ▼
AwaitingRestart ──── awaiting_restart_since + timeout exceeded ──► Failed
  │                                                                  ▲
  │ agent reconnects or scheduler ticks,                             │
  │ detect_version returns "not ready"                               │
  │  └─► stay AwaitingRestart, retry on next tick/reconnect         │
  │                                                                  │
  │ agent reconnects or scheduler ticks,                             │
  │ detect_version returns version mismatch ────────────────────────┘
  │
  │ agent reconnects or scheduler ticks,
  │ detect_version matches to_version
  ▼
Completed
```

All transitions use CAS (`rows_affected == 0` = another controller already acted, skip).

---

### `detect_version` Protocol Extension

`VersionCheckResult` in `crates/shared/wire/src/payloads.rs` gains a new optional field:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub not_ready: Option<bool>,
```

`#[serde(default)]` is required for backward compatibility: older controllers that receive
a `VersionCheckResult` from a new agent without this field parse it as `None`.

Semantics:

- `not_ready: Some(true)` — software is mid-boot or not yet queryable. Controller stays in
  `AwaitingRestart` and retries. Does **not** count as a version mismatch.
- `error: Some(_)` — transient infrastructure failure (plugin crash, timeout). Controller
  stays in `AwaitingRestart` and retries.
- `installed_version: Some(v)` where `v != to_version` — genuine failure. Transition to
  `Failed`.
- `installed_version: Some(v)` where `v == to_version` — success. Transition to `Completed`.

Plugin implementations return `not_ready: true` when they can detect the process is starting
but cannot yet determine its version (e.g., binary executes but service not yet accepting
connections).

**Handler location:** `service_ws/handler/messages.rs` processes `VersionCheckResultsPayload`.
This handler must be extended: after updating the software state for normal version checks,
also look up any `AwaitingRestart` update_history records for the relevant
`host_software_item_id` and apply the terminal-transition logic above. The correlation key
is `VersionCheckResult.host_software_item_id` → `update_history` filtered by
`host_software_item_id + status = AwaitingRestart`.

If `host_software_item_id` is absent from the `VersionCheckResult` (old agent, partial
payload), skip the `AwaitingRestart` correlation entirely for that result — do not attempt
an ambiguous host-level scan. The scheduler will retry on the next tick.

---

### Reconnect Handler Changes

**Location:** `service_ws/handler/updates.rs` → calls into
`update_batches/dispatch.rs::mark_owned_in_progress_as_failed_on_reconnect`

**Current behavior:** loads all `InProgress` records owned by
`(execution_owner_service_id, execution_owner_instance_id)` from the previous runtime
instance of the reconnecting service, marks them `Failed`.

**New behavior** — before the per-record fail loop, branch on `resumable`:

1. Load all `InProgress` candidates via the existing `load_owned_reconnect_candidates`
   function (scoped by `service_id` + `runtime_instance_id` pair, matching records owned
   by a previous instance).
2. For each candidate, join through `host_software_item_id → host_software_item →
   host_software_item_plugin` (role = `execute_update`) to read `resumable`.
3. If `resumable = true`: CAS transition `InProgress → AwaitingRestart`, set
   `awaiting_restart_since = now`.
4. If `resumable = false`: transition to `Failed` (unchanged).
5. After branching: dispatch `detect_version` for all `AwaitingRestart` records associated
   with the reconnecting service's hosts (including newly transitioned ones).

**Standalone self-update hole:** `load_owned_reconnect_candidates` filters by
`execution_owner_service_id = reconnecting_service_id` AND
`execution_owner_instance_id != current_runtime_instance_id` (or IS NULL). This finds
`InProgress` records owned by *previous instances* of the same service — i.e., records
left by the old binary before it exited. When controller-standalone restarts, the embedded
agent reconnects with the same `service_id` but a new `runtime_instance_id`. The function
finds the stale `InProgress` record, and the resumable branch transitions it to
`AwaitingRestart`. No separate startup scan is needed.

This relies on the embedded agent's `service_id` being stable across restarts. The existing
provisioning path (`provision.rs`) looks up the service by `app_name + EmbeddedOwnerKey`
and reuses the persisted UUID, so stability holds under normal conditions. If the lookup
fails (e.g., DB corruption on first boot), a new UUID is allocated and the stale `InProgress`
record would be missed — it would time out via `awaiting_restart_timeout` and be marked
`Failed` by the scheduler, which is the correct safe fallback.

---

### Scheduler Changes

The scheduler gains two new cross-tenant responsibilities — **not** as new `ScheduledTaskType`
rows (which are per-tenant). These are implemented as a new executor registered once on the
`Scheduler` (not tied to a `ScheduledTaskType` DB row), using the `SchedulerNotifier`
already available to all executors. The existing `DetectVersionExecutor` is per-tenant and
cannot be reused here.

#### 1. Verification polling

For all `AwaitingRestart` records across all tenants:

- Dispatch `detect_version` via the existing plugin assignment for each record.
- The MQTT/channel layer handles delivery: immediate if agent is connected, queued in
  the outbox if offline (delivered on next reconnect).
- Handles the case where the agent stays connected after restart but returns `not_ready`
  repeatedly — re-dispatches on each scheduler tick without waiting for a reconnect event.
- If both the reconnect handler and the scheduler dispatch `detect_version` simultaneously,
  the controller handles both responses idempotently via CAS on the `AwaitingRestart →
  Completed/Failed` transition.
- Tick interval: same as existing scheduler poll interval (default 15 seconds).

#### 2. Timeout enforcement

For all `AwaitingRestart` records across all tenants where
`now > awaiting_restart_since + awaiting_restart_timeout`:

- CAS transition to `Failed` with reason "Awaiting restart timed out".
- Uses `awaiting_restart_since` as the anchor; `awaiting_restart_timeout` is read from
  the linked `software_item`, falling back to the global default (600 seconds) when NULL.
- After each terminal transition: signal the controller to trigger batch/host progression
  for the affected host via a new `SchedulerNotifier` method (e.g.,
  `signal_host_progression(host_id, tenant_id)`). The controller's implementation of
  `SchedulerNotifier` calls `dispatch_next_queued_for_host` using its existing
  `ServiceNotifier` and `DispatchContext`. This keeps `scheduler-engine` decoupled from
  `web-api-queries`.
- Fires regardless of agent connection state.

---

### Batch Sequencing

`AwaitingRestart` is not terminal. Three explicit changes are required:

1. **Batch completion check** — the non-terminal status filter in the batch completion query
   (`maybe_complete_batch` in `update_batches/dispatch.rs`, currently `{Queued, Pending,
   InProgress}`) must include `AwaitingRestart`. Without this, a batch containing an
   `AwaitingRestart` item would be incorrectly marked complete.

2. **`has_active_update_for_host` in `update_dispatch.rs`** — currently checks only
   `{Pending, InProgress}`. Must include `AwaitingRestart`. Without this, a new update can
   be dispatched to a host that already has an `AwaitingRestart` item, violating per-host
   sequential dispatch.

3. **`dispatch_next_in_batch` must not fire on `InProgress → AwaitingRestart`** — this
   transition does not complete an update. `dispatch_next_in_batch` is only called on
   terminal transitions (`Completed`, `Failed`). The `AwaitingRestart → Completed/Failed`
   transitions in the `VersionCheckResult` handler and the scheduler timeout enforcer must
   trigger host/batch progression:
   - Batch items: call `dispatch_next_in_batch`.
   - Non-batch items: call `dispatch_next_queued_for_host`.
   The WS handler has `AppState` and constructs `DispatchContext` directly. The scheduler
   timeout enforcer signals progression via `SchedulerNotifier::signal_host_progression`
   (see Scheduler Changes section).

---

## Part 2: Uptrakit Self-Update Discovery Plugin

### Overview

A new discovery plugin (`crates/plugins/discovery/uptrakit-self-update/`) implements the
`Discoverer` trait. It auto-discovers uptrakit services running on the current host and
creates software items with pre-wired plugin assignments, including `resumable = true` on
the execute_update assignment. No manual configuration required.

### Service Metadata Interface

The discovery plugin must not hardcode binary paths or deployment assumptions. Instead, it
queries the controller for its own metadata at discovery time. In controller-standalone, the
discovery plugin runs in the embedded agent — the metadata query is an in-process call, not
an HTTP or RPC request.

The controller exposes a `ServiceMetadata` structure accessible to the embedded agent via
a shared context or trait object injected at startup. If `std::env::current_exe()` fails
(e.g., chroot, unusual process environment), `detect_host_compatibility` returns
`Incompatible` with an explanatory message and discovery emits no software items. Symlinks
are resolved by the execute_update script at runtime rather than at discovery time, so
`binary_path` may be a symlink path — this is acceptable.

```rust
pub struct ServiceMetadata {
    pub service_name: String,           // e.g. "uptrakit-controller-standalone"
    pub binary_path: Option<PathBuf>,   // std::env::current_exe(); None if Docker
    pub version: String,                // current running version
    pub deployment_topology: DeploymentTopology,
}

pub enum DeploymentTopology {
    StandaloneBinary,
    DockerContainer { image: String, container_name: String },
}
```

The discovery plugin calls this interface to build plugin configs dynamically. The
`detect_version` shell command uses the queried `binary_path`. The execute_update config
uses `binary_path` (binary topology) or container identity (Docker topology).

Future services (agent, agent-ssh, mqtt, scheduler) will implement the same metadata
interface when self-update support is extended to them.

### Plugin Assignment Matrix

For each discovered service, the plugin creates assignments:

| Role | Plugin | Config source |
| --- | --- | --- |
| `detect_version` | Shell | `binary_path --version` (queried at discovery time) |
| `fetch_releases` | `releases_github` | uptrakit GitHub repo; tag filter per service |
| `execute_update` | Shell (binary) or Docker plugin | binary path or container identity |

The execute_update assignment is created with `resumable = true`.

### Deployment Topology Detection

The discovery plugin uses `DeploymentTopology` from the metadata interface:

- **`StandaloneBinary`**: execute_update = shell plugin. The script: downloads the new
  release asset, replaces the binary at the queried path, then starts the new binary with
  `--reuseport --takeover-from <current_pid>` using the existing graceful restart mechanism
  (`docs/development/graceful-restart.md`). This gives zero-downtime restart — the old
  process drains connections and exits cleanly without dropping agent WebSocket sessions.
  A supervisor restart (`systemctl restart`) is an acceptable fallback when `--reuseport`
  is not configured, but incurs a full reconnect storm; the script template should prefer
  the graceful path when available.

- **`DockerContainer`**: execute_update = existing Docker plugin. The correct operation
  sequence is: pull new image → stop container → remove container → start new container
  with the new image tag. `docker restart` alone does not change the image and must not
  be used, as it would cause `detect_version` to return the old version and transition
  the update to `Failed`. The container name and image repository are sourced from
  `ServiceMetadata`.

### Version Normalization

`to_version` written to `update_history` at trigger time must use the same format that
`detect_version` returns. GitHub release tags often include a `v` prefix (e.g., `v1.2.3`)
while `binary --version` returns the bare version string (`1.2.3`). If these strings differ,
`AwaitingRestart → Completed` will never fire — the comparison always sees a mismatch and
transitions to `Failed`.

The `releases_github` plugin's `tag_strip_prefix` config strips the prefix from the
fetched version before it is stored as `latest_version` and used as `to_version` at trigger
time. This must be confirmed to work end-to-end for the self-update case. Implementors
must verify that the version string passed to `CreateUpdateRecordParams::to_version` at
trigger time is the stripped form, not the raw tag.

### Discoverer Implementation

```rust
// Implements Discoverer trait
async fn discover_software(&self) -> Result<Vec<DiscoveredSoftware>> {
    let metadata = self.query_controller_metadata().await?;
    let mut results = vec![];

    // MVP: controller-standalone only
    results.push(self.build_software_item(&metadata));

    // Future: query other local services and extend results
    Ok(results)
}

async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
    // Returns Compatible only when running as embedded agent in controller-standalone.
    // Returns Incompatible otherwise.
    // Future: also Compatible when any uptrakit service is detected locally.
}
```

### MVP Scope

MVP covers controller-standalone only. The `detect_host_compatibility` check returns
`Incompatible(reason)` on hosts where no supported uptrakit service is detected.

Future iterations (not in this spec):

- agent, agent-ssh, mqtt, scheduler as additional software items
- Multi-service coordination (e.g., update ordering when controller and agent are separate)
- Windows service topology

---

## Migration

Four migrations required:

1. Add `resumable BOOL NOT NULL DEFAULT FALSE` to `host_software_item_plugin`.
2. Add `awaiting_restart_timeout INTEGER NULL` to `software_item`.
3. Add `awaiting_restart_since TIMESTAMPTZ NULL` to `update_history`.
4. Recreate the `uix_update_history_host_active` partial unique index to include
   `'awaiting_restart'` in its filter:

   ```sql
   -- current:  WHERE status IN ('pending', 'in_progress')
   -- new:       WHERE status IN ('pending', 'in_progress', 'awaiting_restart')
   ```

   Without this, the DB-level "at most one active update per host" guarantee does not
   cover `AwaitingRestart`. A new update could be inserted for a host mid-reboot if the
   code-level `has_active_update_for_host` check fails (race between read and insert in
   multi-controller deployments). This is a safety migration, not a schema addition.

Migrations 1–3 are additive, backward-compatible, non-destructive. Migration 4 drops and
recreates an existing index.

---

## Known Limitations

The following known risks are accepted for the MVP and deferred to follow-on work:

**Multi-controller simultaneous self-update:** If all controllers self-update at the same
time, the entire control plane goes offline simultaneously. This is not prevented by the
current design. Document as unsupported; operators must roll controllers manually in
multi-controller deployments.

**`AwaitingRestart` verification polling at scale:** The cross-tenant AwaitingRestart
executor dispatches `detect_version` every 15 seconds for all records across all
controllers. In a multi-controller deployment with many `AwaitingRestart` hosts, each
agent receives N×M dispatches (N controllers × 1 per tick). CAS prevents state corruption
but creates unnecessary MQTT chatter. Mitigation for a future iteration: rate-limit
verification dispatches per record (e.g., once per minute via `awaiting_restart_last_check`
on `update_history`), or limit dispatch to the controller that owns the agent's WebSocket
connection.

**`not_ready` infinite retry:** A `detect_version` plugin that always returns
`not_ready: true` (plugin bug or genuinely stuck software) holds the record in
`AwaitingRestart` until `awaiting_restart_timeout` expires. There is no maximum `not_ready`
retry count. A future iteration can add `not_ready_count` to `update_history` and fail
early after a configurable threshold.

**Batch blocking for large rollouts:** A multi-host batch where all hosts are
simultaneously in `AwaitingRestart` (e.g., 500-host kernel update) holds the batch in a
non-terminal state for the duration of all reboot windows. Users see the batch as perpetually
in-progress. Future work: introduce a batch "dispatched" phase that separates update
execution from restart verification, so the UI can show "all dispatched, N awaiting restart."

**Zero-config auto-discovery surprise:** The self-update plugin auto-creates software items
without explicit user action. Existing automation (e.g., scheduled batch updates) will apply
to the newly discovered items. Operators should be aware of this behavior; a future
opt-in gate (requiring explicit plugin activation) would address the surprise.

**`ServiceMetadata` extensibility:** The in-process metadata query works only for the
embedded agent. Future support for external services (agent, mqtt, scheduler) requires a
different transport (local HTTP endpoint or well-known config file). The interface design
will need to be revisited at that point.

---

## Quality Gates

Standard gates apply. Additionally:

- Unit tests for reconnect handler branching (resumable vs non-resumable InProgress records).
- Unit tests for `AwaitingRestart` → `Completed` and `AwaitingRestart` → `Failed` transitions.
- Unit tests for `detect_version` "not ready" handling — verifies controller stays in
  `AwaitingRestart` and does not transition to `Failed`.
- Unit tests for timeout enforcement (use `start_paused = true`; only for tests calling
  tokio time APIs).
- Unit test for batch completion check: batch with one `AwaitingRestart` item must not be
  marked complete.
- The self-update discovery plugin integration test requires a running controller-standalone
  instance; mark `#[ignore]` and document in testing.md.
