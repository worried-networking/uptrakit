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

A **resumable** update is one where the execute_update plugin explicitly signals that a
restart is required for verification. The plugin returns `resumable: true` in its
`UpdateResultPayload` after completing the update work (binary replacement, package
install, etc.). The controller transitions `InProgress → AwaitingRestart` on receiving
this signal and waits for the agent to reconnect and confirm the new version is running.

Resumability is a runtime signal from the execute_update plugin — not a pre-configured
DB flag. Plugins determine resumability based on what actually happened:

- **Shell plugin (self-update)**: config includes `resumable: true`; always signals it.
- **APT/RPM plugin**: runs `needrestart -b` or `needs-restarting` after the package
  manager finishes; signals `resumable: true` only when a restart is actually needed.
- **Other plugins**: default to `resumable: false`.

Agent disconnect from `InProgress` without a prior `resumable: true` signal always
transitions to `Failed` — unchanged from current behavior.

---

### Data Model

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
- Dedicated field; not derived from `updated_at` (update_history has no `updated_at`).

---

### `UpdateStatus` New Variant

`AwaitingRestart` is added to **both** `UpdateStatus` enums:

- `crates/shared/types/src/update_status.rs` — the canonical DB/SeaORM type
- `crates/shared/web-api-types/src/update_history.rs` — the API response copy

Both follow the existing `#[non_exhaustive]` + `FromStr`/`ParseUpdateStatusError` pattern.
`UpdateStatus` is not a wire-received type and does not use `Other(String)`.

In `crates/shared/types/src/update_status.rs` the new variant requires:

```rust
#[cfg_attr(feature = "sea-orm", sea_orm(string_value = "awaiting_restart"))]
AwaitingRestart,
```

The `as_str()` method returns `"awaiting_restart"`, `FromStr` parses `"awaiting_restart"`, and the
existing round-trip tests cover the new variant automatically via `strum::EnumIter`.

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
InProgress ── UpdateResultPayload { resumable: true } ──► AwaitingRestart ── timeout exceeded ──► Failed
  │                                                              │
  │ agent disconnects                                    detect_version dispatch (each scheduler tick)
  │ (no resumable signal)                                        │
  ▼                                                    ┌─────────┴──────────────────────────┐
Failed                                                 │ not_ready        → stay, retry tick │
                                                       │ error            → stay, retry tick │
                                                       │ installed_version absent → stay     │
                                                       │ version mismatch → Failed           │
                                                       │ version match    → Completed        │
                                                       └─────────────────────────────────────┘
```

All transitions use CAS (`rows_affected == 0` = another controller already acted, skip).

---

### `execute_update` Protocol Extension

`UpdateResultPayload` in `crates/shared/wire/src/payloads.rs` gains a new optional field:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub resumable: Option<bool>,
```

`#[serde(default)]` ensures older controllers that receive this payload from a new agent
parse `resumable` as `None` (non-resumable) — backward compatible.

**Controller behavior on receiving `UpdateResultPayload`:**

`UpdateResultPayload.status` is `UpdateFinalStatus` (a separate enum from `UpdateStatus`).
The resumable check is applied only when `status == UpdateFinalStatus::Completed`.

- `status: Completed, resumable: Some(true)` → call `transition_to_awaiting_restart(db,
  update_history_id)` in `web-api-queries/update_dispatch.rs`. This is a CAS UPDATE that
  filters on `id = update_history_id AND status = 'in_progress' AND
  execution_owner_service_id = <service_id>`, where `<service_id>` is the same `service_id`
  parameter already present in `handle_update_result` — the value stored as
  `execution_owner_service_id` when `handle_update_started` transitioned the record to
  `InProgress`:
  sets `status = 'awaiting_restart'`, `awaiting_restart_since = now()`, preserves
  `execution_owner_service_id`. On `rows_affected == 0` (race / already claimed), log and return.
  Do not call `dispatch_next_in_batch` yet.
- `status: Completed, resumable: None | Some(false)` → existing behavior (`Completed`).
- `status: Failed` → existing behavior (`Failed`), call batch/host progression.
- `status: Failed, resumable: Some(true)` — `resumable` is ignored when `status != Completed`.
  The update transitions to `Failed` normally. `resumable` is only meaningful on `Completed`.

---

### Agent Pipeline Changes

#### Plugin trait

`execute_update()` in the plugin trait returns a new result struct instead of `String`:

```rust
pub struct ExecuteUpdateResult {
    pub output: String,
    pub resumable: bool,
}
```

Existing plugins return `resumable: false` by default (zero breaking change for existing
implementations).

#### `execute_update_pipeline` refactor

Post-hook execution moves out of `execute_update_pipeline` into the caller (`execute_update`
in `agent-core/src/update.rs`). The pipeline returns `PipelineResult { succeeded: bool,
resumable: bool }` without running post-hooks.

#### Early result channel

For resumable updates the result payload must reach the controller **before** the post-update
hook fires (that hook typically restarts the process). The existing return path — task completes,
`client.rs` calls `send_update_result` — is too late: the hook would kill the process first.

The solution is an early-result channel threaded through `execute_update`:

```rust
// New parameter on execute_update
early_result_tx: mpsc::UnboundedSender<UpdateResultPayload>,
```

`InFlightUpdate` in `agent-core/src/client.rs` gains:

```rust
pub early_result_rx: mpsc::UnboundedReceiver<UpdateResultPayload>,
pub early_sent: bool,
```

The agent's main event loop selects on `early_result_rx` alongside `output_rx`. Biased
ordering alone is not sufficient: if the JoinHandle completes between two select loop
iterations, the next `select!` call can return the `Completed` arm before `early_result_rx`
is polled even with `biased`. The correct pattern is to drain `early_result_rx` via
`try_recv()` inside the JoinHandle completion arm before deciding whether to skip the send:

```rust
tokio::select! {
    biased;
    // Priority 1: drain early result.
    Some(early) = update.early_result_rx.recv() => {
        send_early_update_result(conn, early).await;
        update.early_sent = true;
    }
    Some(output) = update.output_rx.recv() => { ... }
    result = &mut update.handle => {
        // Drain any pending early result BEFORE deciding to send.
        while let Ok(early) = update.early_result_rx.try_recv() {
            send_early_update_result(conn, early).await;
            update.early_sent = true;
        }
        if !update.early_sent {
            send_update_result(conn, update.update_history_id, result).await?;
        }
    }
}
```

When the join handle completes, if `early_sent == true` the result payload is discarded (not sent
again).

The caller in `execute_update` branches:

```rust
if result.resumable && succeeded {
    // Sends early via channel — controller transitions to AwaitingRestart.
    let _ = early_result_tx.send(early_payload_with_resumable_true);
    // Post-hooks fire-and-forget; errors logged only.
    tokio::spawn(run_post_hook_plugins(payload.post_update_hook_plugins, ...));
    // Return a sentinel so client.rs knows not to double-send.
    return UpdateExecutionResult { result: sentinel_payload, resumable: true };
} else {
    run_post_hook_plugins(&payload.post_update_hook_plugins, ...).await;
    // Return normally; client.rs sends via the standard path.
    return UpdateExecutionResult { result: normal_payload, resumable: false };
}
```

`UpdateExecutionResult` gains `resumable: bool`. The `resumable` flag on
`UpdateExecutionResult` is informational only — `client.rs` does not use it to decide
whether to skip the transport write. The authoritative guard is `early_sent`: the
`try_recv` drain inside the JoinHandle arm sets it before the `!early_sent` check runs.
`resumable` on `UpdateExecutionResult` exists only for documentation clarity.
`early_sent` is local state on the agent and is never serialized or sent over the wire.

**Why send before post-hooks for resumable updates:** the post-update hook for a resumable
update typically triggers the restart (e.g., `systemctl restart uptrakit`,
`shutdown -r now`). If the result were sent after the hook, the process would be dead before
the payload reaches the controller. Sending first ensures `InProgress → AwaitingRestart`
is committed before the process exits.

**TCP delivery caveat:** an async TCP write returning successfully means the payload entered
the kernel send buffer — not that the controller received it or committed the DB write. The
protocol relies on the window between send and process exit being sufficient for TCP
delivery + controller DB round-trip. For the SO_REUSEPORT path, the connection drain (step 4)
keeps the WebSocket connection live while PID A drains; since controller and agent are
co-located (controller-standalone + embedded agent), loopback RTT is sub-millisecond and
the DB write completes well within the drain window. For the fallback path, the 10-second
`systemd-run` delay is an empirical estimate based on co-located loopback + healthy DB
(typical latency < 100ms). It is NOT a protocol-level guarantee. On loaded systems with
DB latency > 2 seconds or network-attached databases, 10 seconds may not be sufficient.
No application-level acknowledgment is implemented in MVP — this is a documented constraint.
If delivery is not confirmed before process exit, the update falls back to `Failed` (same
as current behavior), which is safe but negates the resumable benefit for that update.

**Post-hook result ignored for resumable:** once the controller is in `AwaitingRestart`,
the update outcome is determined by `detect_version` on reconnect — not by whether
post-hooks succeeded. Logging hook errors is sufficient.

**Timeout interaction:** the `tokio::time::timeout` wraps `execute_update_pipeline` only
(not post-hooks). For resumable updates, post-hooks run in a detached `tokio::spawn` after
the timeout wrapper completes — the timeout cannot cancel them. This is intentional: the
restart hook must be allowed to fire even if the pipeline ran close to the time limit.

**`handle_graceful_shutdown` awareness:** `handle_graceful_shutdown` in `client.rs` must
check `in_flight_update.early_sent` before calling `send_update_result`. When `early_sent
== true`, the transport write is skipped — the controller already received the early payload
and is in `AwaitingRestart`. Sending the sentinel again would be a no-op (CAS guard drops
it), but skipping is cleaner.

#### Shell plugin `resumable` config field

The shell execute_update plugin gains an optional boolean config field:

```rust
#[serde(default)]
pub resumable: bool,
```

When `true`, the plugin always returns `ExecuteUpdateResult { resumable: true }` regardless
of script output. The self-update discovery plugin sets this field `true` in the shell
plugin config it creates. Other users of the shell plugin are unaffected (default `false`).

---

### `detect_version` Protocol Extension

`VersionCheckResult` in `crates/shared/wire/src/payloads.rs` gains a new optional field:

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub not_ready: Option<bool>,
```

Semantics (evaluated in this order):

1. `not_ready: Some(true)` — software is mid-boot or not yet queryable. Stay in
   `AwaitingRestart`, retry next tick. Does **not** count as a version mismatch.
2. `error: Some(_)` — transient infrastructure failure (plugin crash, timeout). Stay,
   retry next tick.
3. `installed_version: None` (with `not_ready` absent/false and `error` absent) — old
   plugin that does not set the version field, or plugin returned an empty result. Treat
   as `not_ready` — stay, retry next tick. Do **not** treat as mismatch.
4. `installed_version: Some(v)` where `v != to_version` — genuine failure. Transition to
   `Failed`.
5. `installed_version: Some(v)` where `v == to_version` — success. Transition to
   `Completed`.

Plugin implementations return `not_ready: true` when they can detect the process is starting
but cannot yet determine its version (e.g., an HTTP health-endpoint plugin receives a 503
while the service is booting). The shell `detect_version` plugin does not use `not_ready`
— it relies on exit codes and output parsing. Transient failures (non-zero exit, timeout,
malformed output) are reported via `error` or `installed_version: None` (treated as retry
per rule 3 above). `not_ready` is reserved for plugins with richer service-state semantics.

**Handler location:** `service_ws/handler/messages.rs` processes `VersionCheckResultsPayload`
which contains `results: Vec<VersionCheckResult>`. The handler already iterates the results
in a loop. The `AwaitingRestart` correlation check runs **inside that same loop, per result**
— not as a post-loop step. For each `VersionCheckResult` in the payload:

1. Run the existing normal version-check state update (writes `installed_version` to
   `host_software_items`).
2. Then, in the same iteration, look up `update_history` where
   `host_software_item_id = result.host_software_item_id AND status = 'awaiting_restart'`.
   Load the full record including `batch_id` and `to_version` for the comparison.
3. Apply the terminal-transition logic (evaluation order above).

The version comparison compares `result.installed_version` from the **incoming payload**
against `update_history.to_version` from the **loaded DB record**. It does not read from
`host_software_items` after step 1 runs — that avoids TOCTOU where a concurrent write
could change `host_software_items.installed_version` between steps 1 and 3.

If `host_software_item_id` is absent from the `VersionCheckResult` (old agent, partial
payload), skip the `AwaitingRestart` correlation — do not attempt an ambiguous host-level
scan. The scheduler will retry on the next tick.

**Dispatch after terminal transition:** `handle_version_check_results` in `messages.rs`
already receives `&Arc<AppState>`. After a `Completed` or `Failed` transition, it constructs
a `DispatchContext` from `state` (using `state.controller_update_protection()` and
`state.notifier()`) and calls either `dispatch_next_in_batch` (if `batch_id` is set on the
record) or `dispatch_next_queued_for_host` (if standalone). This is the same pattern used
in `handle_update_result` in `updates.rs`. `dispatch_next_in_batch` calls `maybe_complete_batch` unconditionally on every invocation;
`maybe_complete_batch` is a no-op when non-terminal items remain. No additional call to
`maybe_complete_batch` is needed at the handler site.

**CAS loser must not dispatch:** when `rows_affected == 0` on any terminal transition CAS
(another controller already acted), return immediately without calling any dispatch
progression function. The winning controller is responsible for progression; a duplicate
call from the loser would dispatch the next update twice.

---

### Reconnect Handler

**No changes required.** Agent disconnect from `InProgress` → `Failed` (existing behavior,
unchanged). When an update is resumable, the agent sends `UpdateResultPayload { resumable:
true }` before triggering the restart; the controller is already in `AwaitingRestart` when
the agent disconnects. Disconnect from `AwaitingRestart` is a no-op — the controller
continues waiting.

`mark_owned_in_progress_as_failed_on_reconnect` and `mark_all_in_progress_as_failed_for_rollout`
filter on `status = InProgress` only. `AwaitingRestart` records are not `InProgress` and are
unaffected by these functions — no extension required.

**service_id stability across execve:** for the SO_REUSEPORT + execve protocol, PID A execs
itself into the new binary (same PID). The embedded agent in the new binary reconnects and
re-registers. `provision_embedded_tenant_service` uses `embedded_owner_key` for idempotent
provisioning — the same key produces the same `service_id` on every start. The
`execution_owner_service_id` stored on the `AwaitingRestart` record therefore matches the
reconnecting agent's `service_id`. The `AwaitingRestartExecutor` dispatches `detect_version`
to the correct service without any update to the stored owner field. This invariant must
hold: if `provision_embedded_tenant_service` ever generates a new `service_id` for the same
controller instance (e.g., due to a key rotation), the stored `execution_owner_service_id`
will become stale and `detect_version` dispatches will be silently dropped. No mechanism
exists in this spec to detect or recover from that scenario.

---

### Scheduler Changes

The scheduler gains two new cross-tenant responsibilities — **not** as new `ScheduledTaskType`
rows (which are per-tenant). The existing `Scheduler` dispatches executors keyed by
`ScheduledTaskType` from the `scheduled_tasks` DB table. A non-DB executor does not fit this
map. The solution is a new `TickExecutor` abstraction alongside the existing `TaskExecutor`:

```rust
// crates/shared/scheduler-engine/src/tick_executor.rs
#[async_trait::async_trait]
pub trait TickExecutor: Send + Sync {
    async fn execute_tick(&self, db: &DatabaseConnection) -> error::Result<()>;
}
```

`Scheduler` gains:

```rust
tick_executors: Vec<Arc<dyn TickExecutor>>,
```

with a `register_tick_executor(executor: Box<dyn TickExecutor>)` method. On each `poll_cycle`
the scheduler runs all `tick_executors` concurrently in a **separate** `JoinSet` (not the
same JoinSet used for DB-driven `TaskExecutor`s). TickExecutors are NOT subject to
`TASK_EXECUTION_TIMEOUT` (2 hours) — they are bounded by a shorter 60-second per-tick
timeout instead, after which the tick is abandoned and the next poll cycle retries.
`TickExecutor`s receive the DB connection directly; they use the `SchedulerNotifier` injected
at construction time. `JoinSet::join_next` errors — including panics surfaced as
`JoinError::is_panic()` — are logged and the tick is abandoned; the scheduler continues
normally on the next poll cycle. Panics in `TickExecutor` must not crash the scheduler.

The `AwaitingRestartExecutor` implements `TickExecutor` and is constructed with
`(db, notifier)` — the same dependencies as other executors (e.g., `DiscoverSoftwareExecutor`).
It is registered once in the controller's scheduler setup, not per tenant.

#### 1. Verification polling

For all `AwaitingRestart` records across all tenants:

- Load records with `status = 'awaiting_restart'`. For each record:
  1. Use `execution_owner_service_id` (stored on the row) as the target `service_id` for
     `notifier.send_to_service`. Do **not** perform a fresh `service_host` join — the owner
     is already on the record, and a fresh join could resolve a different (newer) agent.
     If `execution_owner_service_id IS NULL`, log a warning and skip.
  2. Look up the `detect_version` plugin assignment: query `host_software_item_plugin` where
     `host_software_item_id = record.host_software_item_id AND role = 'detect_version'`.
     If no assignment exists (plugin config was deleted after the update started), log a
     warning and skip — the scheduler will retry on the next tick.
  3. Dispatch `detect_version` using the loaded plugin assignment config via the normal
     plugin dispatch path.
- The MQTT/channel layer handles delivery: immediate if agent is connected, queued in
  the outbox if offline (delivered on next reconnect).
- Handles the case where the agent stays connected after restart but returns `not_ready`
  repeatedly — re-dispatches on each scheduler tick without waiting for a reconnect event.
- If both a reconnect-triggered dispatch and a scheduler tick dispatch `detect_version`
  simultaneously, the controller handles both responses idempotently via CAS.
- Tick interval: same as existing scheduler poll interval (default 15 seconds).

#### 2. Timeout enforcement

For all `AwaitingRestart` records across all tenants where
`awaiting_restart_since IS NOT NULL AND now > awaiting_restart_since + awaiting_restart_timeout`:

- CAS transition to `Failed` with reason "Awaiting restart timed out".
- The `IS NOT NULL` filter is required: SQL `now > NULL + X` evaluates to NULL (not true),
  so records with a missing `awaiting_restart_since` would never time out and silently
  accumulate. The invariant is that `transition_to_awaiting_restart` always sets
  `awaiting_restart_since = now()` atomically in the same UPDATE — but the filter guards
  against any future bug that violates this invariant.
- Records where `awaiting_restart_since IS NULL` (invariant violation) are logged by the
  executor as warnings on each tick but are otherwise left unchanged.
- Uses `awaiting_restart_since` as the anchor; `awaiting_restart_timeout` is read from
  the linked `software_item`, falling back to the global default (600 seconds) when NULL.
- After each terminal transition: signal the controller to trigger batch/host progression
  for the affected host via a new `SchedulerNotifier` method:

  ```rust
  async fn signal_host_progression(&self, host_id: Uuid, tenant_id: Uuid);
  ```

  The method is added to `SchedulerNotifier` in `notifier.rs`. The embedded-controller
  implementation calls `dispatch_next_queued_for_host` directly (it has access to
  `ServiceNotifier` and `DispatchContext`). The NATS-only implementation publishes a
  `HostProgressionNeeded { host_id, tenant_id }` message on the existing internal NATS
  controller subject (internal-only, same subject used for other cross-controller signals,
  not part of the agent wire protocol); the receiving controller dispatches inline. The `NoopSchedulerNotifier` (test-only, in
  `notifier.rs` behind `#[cfg(test)]`) also gets a no-op implementation. This keeps
  `scheduler-engine` decoupled from `web-api-queries`.
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
   transition does not complete an update. The `AwaitingRestart → Completed/Failed`
   transitions in the `VersionCheckResult` handler and the scheduler timeout enforcer must
   trigger host/batch progression:
   - Batch items: call `dispatch_next_in_batch`.
   - Non-batch items: call `dispatch_next_queued_for_host`.
   The WS handler has `AppState` and constructs `DispatchContext` directly. The scheduler
   timeout enforcer signals progression via `SchedulerNotifier::signal_host_progression`
   (see Scheduler Changes section).

4. **Unique constraint violation on dispatch insert:** when `dispatch_next_in_batch` or
   `dispatch_next_queued_for_host` inserts a new `update_history` row and the DB returns a
   unique constraint violation on `uix_update_history_host_active`, treat this as "another
   controller already dispatched to this host" — log at debug level and return without
   error. Do not log as an error. This is the same race-handling pattern applied to the
   existing `has_active_update_for_host` check: the check is a fast-path optimization;
   the unique constraint is the authoritative guard.

---

## Part 2: Uptrakit Self-Update Discovery Plugin

### Overview

A new discovery plugin (`crates/plugins/discovery/uptrakit-self-update/`) implements the
`Discoverer` trait. It auto-discovers uptrakit services running on the current host and
creates software items with pre-wired plugin assignments. The shell execute_update plugin
assignment is configured with `resumable: true` so the plugin signals resumability at
runtime. No manual configuration required.

### Service Metadata Interface

The discovery plugin must not hardcode binary paths or deployment assumptions. Instead, it
queries the controller for its own metadata at discovery time. In controller-standalone, the
discovery plugin runs in the embedded agent — the metadata query is an in-process call, not
an HTTP or RPC request.

The controller exposes a `ServiceMetadata` structure to the embedded agent via a
`ServiceMetadataProvider` trait object injected at plugin construction time through the
existing `HostRuntime` injection path. The controller-standalone constructs the plugin with
`Some(Arc<dyn ServiceMetadataProvider>)`. Standalone agents (no controller) construct it
with `None`.

`detect_host_compatibility` checks whether the provider is `Some` — if `None`, returns
`Incompatible("not running as embedded agent in controller-standalone")`. This is the
primary gating mechanism; no binary inspection is required.

If `std::env::current_exe()` fails at discovery time, `discover_software` logs a warning
and emits no software items for the affected service. Symlinks are resolved by the
execute_update script at runtime rather than at discovery time, so `binary_path` may be a
symlink path — this is acceptable. For `DeploymentTopology::UnixBinary`, `binary_path` is
required — if it is `None` (current_exe failure or incorrect topology detection),
`discover_software` logs a warning and skips that service.

```rust
pub struct ServiceMetadata {
    pub service_name: String,           // e.g. "uptrakit-controller-standalone"
    pub binary_path: Option<PathBuf>,   // std::env::current_exe(); None if Docker
    pub version: String,                // current running version
    pub deployment_topology: DeploymentTopology,
    pub reuseport_configured: bool,     // whether SO_REUSEPORT takeover protocol is active
    pub pid_file: Option<PathBuf>,      // path where binary writes its PID on startup
}

pub enum DeploymentTopology {
    /// Unix only (Linux + macOS). Windows deferred — see MVP Scope.
    UnixBinary,
    DockerContainer { image: String, container_name: String },
}

/// Implemented by the controller-standalone; injected into the plugin at construction.
pub trait ServiceMetadataProvider: Send + Sync {
    fn get_metadata(&self) -> ServiceMetadata;
}
```

`ServiceMetadata`, `ServiceMetadataProvider`, and `DeploymentTopology` live in
`crates/plugins/infrastructure/core/src/service_metadata.rs` — alongside the other
plugin-infrastructure types (`HostRuntime`, `Discoverer`, etc.) that the discovery plugin
already imports.

The binary writes its PID to `pid_file` on startup (standard Unix PID file). The update
script reads the PID at runtime — not from `ServiceMetadata.pid` baked at discovery time,
since the process may have restarted between discovery and update execution.

The `HostRuntime` trait gains a default method returning `None`:

```rust
fn metadata_provider(&self) -> Option<Arc<dyn ServiceMetadataProvider>> { None }
```

The controller-standalone constructs a `MetadataAwareHostRuntime` that wraps the standard
runtime and overrides this method. `StandardHostRuntime::new(executor, caps)` is unchanged —
call sites that don't need the provider pass through normally. Only the self-update discovery
plugin's `Discoverer` role construction path uses `MetadataAwareHostRuntime`.

The plugin struct holds `metadata_provider: Option<Arc<dyn ServiceMetadataProvider>>`,
resolved from `runtime.metadata_provider()` at `new()` time.
`detect_host_compatibility` returns `Incompatible` when `metadata_provider` is `None`.
Future services (agent, agent-ssh, mqtt, scheduler) will implement the same provider trait
when self-update support is extended to them.

### Plugin Assignment Matrix

For each discovered service, the plugin creates assignments:

| Role | Plugin | Config |
| --- | --- | --- |
| `detect_version` | Shell | `version_command: "<binary_path> --version"` (binary path embedded literally at discovery time; no `{package_identifier}` substitution); `version_regex: "(?P<version>\d+\.\d+\.\d+)"` to strip the binary name prefix from output like `"uptrakit-controller-standalone 1.2.3"` |
| `fetch_releases` | `releases_github` | uptrakit GitHub repo; `tag_strip_prefix: "v"`; tag filter per service |
| `execute_update` | Shell (binary) or Docker plugin | `resumable: true`; binary path or container identity |

`binary --version` output format: `"<service-name> <semver>"` (e.g., `"uptrakit-controller-standalone 1.2.3"`). The
shell `detect_version` plugin extracts the version using `version_regex`. If the binary ever changes its
`--version` output format (e.g., adds build metadata like `1.2.3+abc123`), the regex must be updated in
the discovery plugin. No normalization is applied to the extracted version — the regex capture group must
return exactly the string stored as `to_version` (bare semver, no `v` prefix, since `tag_strip_prefix: "v"`
already strips the prefix from release tags).

`package_identifier` for self-update software items is the service name string (e.g.
`"uptrakit-controller-standalone"`). It is used as the display identifier and for
deduplication, not for command substitution in the detect_version shell command (the
binary path is embedded literally in `version_command`).

### Deployment Topology Detection

The discovery plugin uses `DeploymentTopology` from the metadata interface:

- **`UnixBinary`**: execute_update = shell plugin with `resumable: true`. The update
  script is generated at discovery time from `ServiceMetadata` and branches on
  `reuseport_configured`. The takeover protocol itself is implemented inside the binary —
  the script's job is just to replace the binary on disk and signal the running process.

  **`reuseport_configured = true` (preferred — zero-downtime, supervisor-transparent, no
  dead-control-plane risk):**

  `BINARY_PATH` and `PID_FILE` are embedded literally at discovery time from
  `ServiceMetadata.binary_path` and `ServiceMetadata.pid_file`. If `pid_file` is `None`
  on a `UnixBinary` service, `discover_software` logs a warning and skips that service —
  the takeover protocol requires a PID file.

  The script:

  ```bash
  # Download to a temp file on the same filesystem as the binary (atomic rename requires same FS).
  # /tmp is typically a separate tmpfs mount — using it causes non-atomic mv across filesystems.
  BINARY_PATH="<binary_path>"
  TMP_PATH="${BINARY_PATH}.new-$$"
  curl -L "$RELEASE_URL" -o "$TMP_PATH"
  chmod +x "$TMP_PATH"
  # Ad-hoc codesign required on Apple Silicon; no-op on Linux (codesign absent)
  command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"
  mv "$TMP_PATH" "$BINARY_PATH"
  # Signal takeover — binary handles the rest
  kill -USR2 "$(cat "<pid_file>")"
  ```

  On receiving SIGUSR2, the binary executes the combined SO_REUSEPORT + execve protocol.
  **Spawning the child process safely in a multi-threaded async runtime:** direct `fork()`
  from a SIGUSR2 signal handler or from within the Tokio runtime is unsafe — background
  threads may hold allocator or libc locks, causing the child to deadlock before it reaches
  `exec()`. The correct approach: the SIGUSR2 handler performs only an async-signal-safe
  operation (writes one byte to a pre-created pipe). A dedicated Tokio task reads from that
  pipe and spawns the child via `tokio::process::Command::new()`, which uses a
  carefully-managed fork+exec path that avoids the multi-threaded lock hazard.

  1. **Spawn** PID B via `tokio::process::Command::new(binary_path)` with
     `--reuseport --post-takeover-child --notify-fd <pipe_write_fd>` and the listening
     socket fd explicitly inherited (via `CommandExt::pre_exec` to clear `O_CLOEXEC`).
  2. **PID B** (new binary) binds the listening port via `SO_REUSEPORT` alongside the old
     process (PID A). Both are now accepting connections. PID B writes `ready` to the pipe
     when its async runtime is up.
  3. **PID A** reads `ready` from the pipe. Sends `UpdateResultPayload { resumable: true }`
     via the embedded agent (the early result channel). Stops calling `accept()` — OS routes
     all new connections to PID B.
  4. **PID A** drains in-flight connections (waits for active requests to complete, bounded
     by a short grace period).
  5. **PID A** calls `execve(binary_path, ["--reuseport", "--post-takeover-parent",
     "--notify-fd", "<pipe_write_fd>"])` — PID A is now running the new binary with the
     original PID. The supervisor still tracks PID A; it never saw a process exit.
  6. **PID A** (new binary) writes `done` to the pipe.
  7. **PID B** reads `done` and exits cleanly.
  8. **PID A** is now the sole process: new binary, original PID, supervisor-transparent.

  **Dead-control-plane protection:** if the new binary (PID B, step 2) crashes at startup,
  it never writes `ready` to the pipe. PID A detects child exit via `waitpid`, aborts the
  takeover, remains running with the old binary, and logs an error. The `AwaitingRestart`
  early result payload was not yet sent (step 3 never fired), so the update remains
  `InProgress`. The agent then sends a failure result; the controller marks the update
  `Failed` and unblocks the host. No control plane outage.

  **Pipe protocol vocabulary:** three messages are used on the coordination pipe:
  - `ready` — sent PID B → PID A: new binary is up and accepting connections.
  - `done` — sent PID A → PID B: execve succeeded; PID B should exit cleanly (exit 0).
  - `abort` — sent PID A → PID B: takeover failed (execve error); PID B should exit with error (exit 1).

  PID B reads one message and branches on value. Any value that is not `done` is treated as
  `abort`. If the pipe's write end is closed (PID A crashed before writing), `read()` returns
  0 bytes (EOF); PID B treats EOF as `abort` and exits with error (exit 1). This is standard
  POSIX pipe behavior and requires no special handling beyond checking the read length.

  **execve failure recovery:** `execve(2)` is atomic — it either replaces the process image
  (on success, never returns) or returns an error code with the process unchanged. If step 5
  fails (e.g., `ENOEXEC`, `ENOMEM`), PID A is still running the old binary. PID A writes
  `abort` to the pipe and waits for PID B to exit with a 5-second timeout. If PID B does
  not exit within the timeout, PID A sends `SIGKILL` to PID B and waits for it to be
  reaped. PID A then logs the error. At this point the
  controller is already in `AwaitingRestart` (early result was sent in step 3). On the next
  scheduler tick, `detect_version` detects the old version, sees a mismatch, and transitions
  the update to `Failed`. The control plane stays live throughout. No special recovery code
  is needed in the controller.

  **Graceful shutdown during takeover:** if SIGTERM arrives after step 3 (early result sent,
  early_sent = true) but before step 5 (execve), PID A is mid-drain. Graceful shutdown
  completes the drain, writes `abort` to the pipe (signaling PID B to exit), and exits. The
  controller is already in `AwaitingRestart`. On the next scheduler tick, `detect_version`
  runs: PID B may have started successfully (returns new version → `Completed`) or may have
  exited with `abort` (returns old version or error → `Failed`). No special recovery code
  required — the scheduler resolves the race. The early result having been sent before drain
  ensures `handle_graceful_shutdown` skips the redundant transport write.

  **Supervisor interaction:** no MAINPID notification or special unit configuration needed.
  PID A never exits during the protocol — the supervisor always tracks a live process.
  Works with systemd (`Type=simple` or `Type=exec`), s6, runit, supervisord, and any other
  supervisor that tracks a single PID.

  **Drain grace period:** step 4 is bounded by the existing graceful-shutdown timeout
  (the same value used for `handle_graceful_shutdown` in `client.rs`). When the timeout
  expires, remaining in-flight connections are force-closed and PID A proceeds to step 5.
  This prevents a stalled WebSocket session from blocking the takeover indefinitely.

  **`reuseport_configured = false` (fallback — requires supervisor + deferred restart):**

  The script downloads and replaces the binary, then issues a deferred supervisor restart.
  `SERVICE_NAME` and `BINARY_PATH` are embedded literally at discovery time from
  `ServiceMetadata.service_name` and `ServiceMetadata.binary_path`:

  ```bash
  # Download to a temp file on the same filesystem as the binary.
  BINARY_PATH="<binary_path>"
  TMP_PATH="${BINARY_PATH}.new-$$"
  curl -L "$RELEASE_URL" -o "$TMP_PATH"
  chmod +x "$TMP_PATH"
  command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"
  mv "$TMP_PATH" "$BINARY_PATH"
  systemd-run --on-active=10s systemctl restart "<service_name>"
  ```

  The 10-second delay ensures the embedded agent sends the early result payload and the
  controller commits the `AwaitingRestart` DB write before the process exits. Immediate
  `systemctl restart` is explicitly prohibited — it races with the DB commit and leaves
  `InProgress` records that resolve as `Failed`. This path has a brief connectivity gap
  (old process exits, new process starts) and no dead-control-plane protection — if the
  new binary fails to start, the supervisor restarts it, but there is a gap. Operators
  are strongly encouraged to enable `SO_REUSEPORT`.

  The `reuseport_configured` flag is read from `ServiceMetadata` at discovery time. The
  discovery plugin generates the appropriate `update_command` inline in the shell plugin
  config; no separate template files are needed.

- **`DockerContainer`**: execute_update = existing Docker plugin. The Docker plugin gains
  the same `#[serde(default)] pub resumable: bool` config field as the shell plugin. The
  self-update discovery plugin sets it `true` in the generated Docker plugin config. The
  correct operation sequence is: pull new image → stop container → remove container → start
  new container with the new image tag. `docker restart` alone does not change the image
  and must not be used — it causes `detect_version` to return the old version and transition
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
time. The `releases_github` plugin is solely responsible for normalization — it returns the
stripped string; by the time `to_version` is written to `update_history`, the prefix is
already gone. No normalization is needed in the trigger handler or the version comparison
logic. Setting `tag_strip_prefix: "v"` in the self-update discovery plugin's `releases_github`
target config is the complete fix — no additional code required.

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

fn build_software_item(&self, metadata: &ServiceMetadata) -> DiscoveredSoftware {
    // ... plugin assignment matrix as described in Plugin Assignment Matrix section ...
    // MUST set awaiting_restart_timeout: Some(120) — 2 minutes.
    // The global default (600 seconds) is too long for self-update: if the controller dies
    // mid-update, the scheduler must time out quickly on the next restart, not after 10 minutes.
    DiscoveredSoftware {
        // ...
        awaiting_restart_timeout: Some(120),
        // ...
    }
}

async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
    // Returns Incompatible when enabled == false.
    // Returns Incompatible when metadata_provider is None (not embedded agent).
    // Returns Compatible only when enabled == true AND running as embedded agent.
    // Future (enabled == true only): also Compatible when other local uptrakit services detected.
}
```

### Plugin Config

The plugin has a single config field:

```rust
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UptrakitSelfUpdateConfig {
    /// Enable self-update discovery. Defaults to false.
    ///
    /// Self-update auto-discovery can trigger unattended updates via
    /// scheduled batch tasks. Opt-in required — operators must explicitly
    /// set `enabled: true` to activate this plugin.
    #[serde(default)]
    pub enabled: bool,
}
```

The default shipped config for this plugin sets `enabled: false`. Operators must explicitly
set `enabled: true` to activate self-update discovery. This applies to controller-standalone
as well — it ships with `enabled: false` in the bundled default plugin config for this plugin;
no deployment has self-update enabled by default.

`discover_software` returns `Ok(vec![])` immediately when `enabled == false`.
`detect_host_compatibility` returns `Incompatible("self-update disabled by config")` when
`enabled == false`. This prevents spurious "plugin is compatible but produces no items"
states in the registry and avoids running compatibility checks against hosts where the
feature is intentionally disabled.

### MVP Scope

MVP covers controller-standalone on **Linux and macOS (Intel + Apple Silicon)** only.
The `detect_host_compatibility` check returns `Incompatible(reason)` on hosts where no
supported uptrakit service is detected.

The `UnixBinary` topology uses `fork()`, `execve()`, `SO_REUSEPORT`, and `SIGUSR2` — all
POSIX primitives unavailable or semantically different on Windows. Windows is explicitly
out of scope for this spec.

**macOS Apple Silicon note:** binaries downloaded by the update script via `curl` do not
receive the quarantine flag and do not require notarization. However, all binaries on
Apple Silicon must carry at minimum an ad-hoc signature to execute — the OS kills unsigned
binaries at load time. The update script must call `codesign --sign - --force <binary>`
after download and before `mv`. This is a documentation and script-generation requirement
only; no changes to the binary build pipeline are in scope for this spec. Release pipeline
codesigning (Developer ID, notarization) is a separate concern outside this spec's scope.

Future iterations (not in this spec):

- agent, agent-ssh, mqtt, scheduler as additional software items
- Multi-service coordination (e.g., update ordering when controller and agent are separate)
- Windows service topology: requires `CreateProcess()` + named-pipe coordination +
  `SO_REUSEADDR`-based hand-off; no `execve()` equivalent; SCM manages service identity
  by name rather than PID so supervisor-transparency is not a concern

---

## Migration

Three migrations required:

1. Add `awaiting_restart_timeout INTEGER NULL` to `software_item`.
2. Add `awaiting_restart_since TIMESTAMPTZ NULL` to `update_history`.
3. Recreate the `uix_update_history_host_active` partial unique index to include
   `'awaiting_restart'` in its filter:

   ```sql
   DROP INDEX IF EXISTS uix_update_history_host_active;
   CREATE UNIQUE INDEX uix_update_history_host_active ON update_history (host_id)
     WHERE status IN ('pending', 'in_progress', 'awaiting_restart');
   ```

   Without this, the DB-level "at most one active update per host" guarantee does not
   cover `AwaitingRestart`. A new update could be inserted for a host mid-reboot if the
   code-level `has_active_update_for_host` check fails (race between read and insert in
   multi-controller deployments). This is a safety migration, not a schema addition.

Migrations 1–2 are additive, backward-compatible, non-destructive. Migration 3 drops and
recreates an existing index.

---

## Known Limitations

The following known risks are accepted for the MVP and deferred to follow-on work:

**Multi-controller simultaneous self-update:** If all controllers self-update at the same
time, the entire control plane goes offline simultaneously. This is not prevented by the
current design. Document as unsupported; operators must roll controllers manually in
multi-controller deployments.

**Failed self-update and the control plane:** When `SO_REUSEPORT` is configured (preferred
path), a failed new binary (PID B) never writes `ready` to the coordination pipe. The old
process (PID A) detects child exit, aborts the takeover, remains running with the old binary,
and sends a failure result. The dead-control-plane risk is fully eliminated on this path —
PID A only execs itself into the new binary after PID B has confirmed it is up.

When `SO_REUSEPORT` is not configured (fallback path), the old process exits before the new
one is confirmed running. If the new binary fails to start, the control plane is briefly dead
until the supervisor restarts it. The self-update discovery plugin checks for a running
supervisor (systemd unit or Docker restart policy) during `discover_software` and logs a
warning when none is detected — this check is **warning-only and never blocks discovery**;
the software item is always emitted regardless of supervisor presence. Supervisor detection
is inherently unreliable (wrapper scripts, s6, runit, symlinks, Docker containers all
produce false negatives), so blocking on it would silently suppress self-update in valid
deployments. Operators using the fallback path are responsible for ensuring a supervisor is
in place; operators using `--reuseport` have no supervisor dependency.

**Natural recovery after failed self-update:** No startup flag is needed to clean up
`AwaitingRestart` records on controller restart. The natural flow resolves them correctly:

- **Self-update succeeded**: new binary starts → `AwaitingRestartExecutor` runs on first
  scheduler tick → dispatches `detect_version` to embedded agent → returns new version →
  matches `to_version` → transitions to `Completed`.
- **Self-update failed, operator recovers with old binary**: old binary starts →
  `AwaitingRestartExecutor` runs → `detect_version` returns old version → mismatch →
  transitions to `Failed`.
- **Self-update failed, operator recovers with fixed binary**: same as success path above.

No explicit "fail all pending" flag is required and would be harmful: if applied on a normal
post-self-update restart, it would mark the successful update `Failed` before `detect_version`
can confirm the new version.

**AwaitingRestart timeout orphaning:** When the embedded scheduler stops (controller death),
no process enforces `AwaitingRestart` timeouts. Records stay in `AwaitingRestart` until the
controller restarts and the `AwaitingRestartExecutor` runs. Mitigated by using a short
`awaiting_restart_timeout` for self-update items (e.g., 120 seconds) so that on restart,
records time out quickly via the scheduler's first tick.

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

**Batch blocking for large rollouts:** A multi-host batch where all hosts are simultaneously
in `AwaitingRestart` (e.g., 500-host kernel update) holds the batch in a non-terminal state
for the duration of all reboot windows. Future work: introduce a batch "dispatched" phase
that separates update execution from restart verification.

**Zero-config auto-discovery surprise:** The self-update plugin auto-creates software items
without explicit user action. Existing automation (e.g., scheduled batch updates) will apply
to the newly discovered items. Operators should be aware of this behavior; a future opt-in
gate would address the surprise.

**Supervisor restart race (fallback path, `reuseport_configured = false` only):** The
combined SO_REUSEPORT + execve path has no restart race — PID A remains alive throughout
the entire protocol and only execs after the early result payload is sent and PID B is
confirmed running. For the supervisor fallback path, an immediate restart (`systemctl
restart`) races with the controller's DB commit of `AwaitingRestart`. This is a spec
constraint: the fallback update script **must** use deferred restart (`systemd-run
--on-active=10s`) — immediate supervisor restart is not valid on this path. Operators
using the fallback path without deferred restart may observe updates incorrectly marked
`Failed`.

**Existing connection drops on `reuseport_configured = false`:** The fallback path
(supervisor restart) drops in-flight WebSocket and HTTP connections. Agents reconnect
automatically. Web UI sessions reconnect on drop. This is acceptable for a self-update
event and is not tracked as an open issue.

**`post-takeover-child` startup race:** PID B writes `ready` to the pipe when its async
runtime is initialized, but before it has accepted the first real connection. There is a
brief window where both PID A (draining) and PID B (starting) may drop a new connection
that arrives between PID B's bind and its first `accept()` call. In practice the OS queues
these in the accept backlog; they are served once PID B's accept loop starts. Not a known
issue in practice but documented for completeness.

**macOS Apple Silicon — ad-hoc signing required in update script:** On Apple Silicon,
the kernel refuses to load an unsigned binary (`SIGKILL` at exec time). Both generated
scripts include `command -v codesign >/dev/null 2>&1 && codesign --sign - --force "$TMP_PATH"`
before the atomic `mv`. `command -v codesign` is false on Linux (codesign not present),
making this a portable no-op on non-macOS hosts. No Developer ID or notarization needed
for self-update — ad-hoc signing is sufficient. Notarization is only required for initial
browser-based distribution (Gatekeeper), outside this spec.

**`tag_strip_prefix: "v"` assumption:** the self-update discovery plugin hardcodes
`tag_strip_prefix: "v"` in the generated `releases_github` config. If uptrakit ever
adopts a tag scheme without a `v` prefix (CalVer, bare numbers), `to_version` will carry
a `v` prefix that `detect_version` (which runs `binary --version`) will not return —
every `AwaitingRestart → Completed` check will see a mismatch and fail. This assumption
is acceptable for MVP; revisit if the release tag format changes.

**Windows — not supported:** The `UnixBinary` topology relies on `fork()`, `execve()`,
`SO_REUSEPORT`, and `SIGUSR2`, none of which exist on Windows. `detect_host_compatibility`
returns `Incompatible("Windows is not supported")` when the target OS is Windows.
A future `WindowsService` topology variant will use `CreateProcess()` with an inherited
socket handle and named-pipe coordination in place of the Unix protocol.

**Stale plugin assignment after config deletion:** if the `detect_version` plugin assignment
is deleted and recreated while an update is in `AwaitingRestart`, the executor may dispatch
with the old config (or skip entirely if deleted without replacement). This resolves itself
at `awaiting_restart_timeout`. No mitigation in MVP. Future: record the assignment ID on
`update_history` at dispatch time so the executor uses a stable reference.

**`ServiceMetadata` extensibility:** The in-process metadata query works only for the
embedded agent. Future support for external services (agent, mqtt, scheduler) requires a
different transport (local HTTP endpoint or well-known config file). The interface design
will need to be revisited at that point.

---

## Quality Gates

Standard gates apply. Additionally:

- Unit tests for `UpdateResultPayload { resumable: true }` → `InProgress → AwaitingRestart`
  transition on the controller side.
- Unit tests for `AwaitingRestart` → `Completed` and `AwaitingRestart` → `Failed`
  transitions via `detect_version` responses.
- Unit tests for `detect_version` `not_ready` handling — verifies controller stays in
  `AwaitingRestart` and does not transition to `Failed`.
- Unit tests for agent pipeline branching: resumable path sends result before post-hooks;
  non-resumable path sends result after post-hooks.
- Unit test for early-result channel race: simulate JoinHandle completing before
  `early_result_rx` is polled; verify `try_recv` drain prevents double-send.
- Unit tests for timeout enforcement (use `start_paused = true`; only for tests calling
  tokio time APIs).
- Unit test for `transition_to_awaiting_restart` CAS: rows_affected == 0 on race.
- Unit test for `has_active_update_for_host`: `AwaitingRestart` status blocks dispatch.
- Unit test for batch completion check: batch with one `AwaitingRestart` item must not be
  marked complete.
- Unit test for `AwaitingRestartExecutor` skipping records where `execution_owner_service_id IS NULL`.
- Unit test for self-update plugin's generated `releases_github` assignment config: asserts
  `tag_strip_prefix == "v"` is present (guards against the most common version-mismatch failure).
- Unit test for `transition_to_awaiting_restart` CAS: UPDATE is a no-op when
  `execution_owner_service_id` does not match the calling service_id (prevents cross-service
  hijack — a different controller must not be able to claim another's in-flight update).
- Unit test for `AwaitingRestartExecutor` skipping records where no `detect_version` plugin
  assignment exists (plugin config deleted after update started); verifies warning is logged
  and scheduler retries next tick.
- Unit test verifying that receiving `UpdateResultPayload { resumable: true }` does not call
  `dispatch_next_in_batch` or `dispatch_next_queued_for_host` — the `InProgress →
  AwaitingRestart` transition must not trigger host/batch progression.
- The self-update discovery plugin integration test requires a running controller-standalone
  instance; mark `#[ignore]` and document in testing.md.
