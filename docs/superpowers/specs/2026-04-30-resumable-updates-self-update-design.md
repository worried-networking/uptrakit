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
InProgress ──── UpdateResultPayload { resumable: true } received ──► AwaitingRestart
  │                                                                        │      │
  │ agent disconnects (no resumable signal received)                       │      │ awaiting_restart_since
  ▼                                                                        │      │ + timeout exceeded
Failed                                                           detect_version   ▼
                                                                    result:    Failed
                                                                       │
                                                               not_ready ──► stay (retry next tick)
                                                               error    ──► stay (retry next tick)
                                                               mismatch ──► Failed
                                                               match    ──► Completed
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
  update_history_id, service_id, runtime_instance_id)` in `web-api-queries/update_dispatch.rs`.
  This is a CAS UPDATE on the owned `InProgress` row:
  `status = 'awaiting_restart'`, `awaiting_restart_since = now()`, preserving
  `execution_owner_service_id`. On `rows_affected == 0` (race / already claimed), log and return.
  Do not call `dispatch_next_in_batch` yet.
- `status: Completed, resumable: None | Some(false)` → existing behavior (`Completed`).
- `status: Failed` → existing behavior (`Failed`), call batch/host progression.

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

`UpdateExecutionResult` gains `resumable: bool`. When `resumable == true`, `send_update_result`
in `client.rs` skips the transport write (early_sent already set true by the select loop).

**Why send before post-hooks for resumable updates:** the post-update hook for a resumable
update typically triggers the restart (e.g., `systemctl restart uptrakit`,
`shutdown -r now`). If the result were sent after the hook, the process would be dead before
the payload reaches the controller. Sending first ensures `InProgress → AwaitingRestart`
is committed before the process exits.

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
`host_software_item_id + status = AwaitingRestart`. The full record (including `batch_id`)
is loaded during the lookup so that the correct progression function can be chosen without
an additional DB round-trip after the CAS.

The version comparison uses `VersionCheckResult.installed_version` read directly from the
incoming payload — not from `host_software_items` after the normal state-update step has
run. This avoids a TOCTOU ambiguity where a concurrent update to `host_software_items`
could cause a mismatch.

If `host_software_item_id` is absent from the `VersionCheckResult` (old agent, partial
payload), skip the `AwaitingRestart` correlation — do not attempt an ambiguous host-level
scan. The scheduler will retry on the next tick.

**Dispatch after terminal transition:** `handle_version_check_results` in `messages.rs`
already receives `&Arc<AppState>`. After a `Completed` or `Failed` transition, it constructs
a `DispatchContext` from `state` (using `state.controller_update_protection()` and
`state.notifier()`) and calls either `dispatch_next_in_batch` (if `batch_id` is set on the
record) or `dispatch_next_queued_for_host` (if standalone). This is the same pattern used
in `handle_update_result` in `updates.rs`.

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
at construction time.

The `AwaitingRestartExecutor` implements `TickExecutor` and is constructed with
`(db, notifier)` — the same dependencies as other executors (e.g., `DiscoverSoftwareExecutor`).
It is registered once in the controller's scheduler setup, not per tenant.

#### 1. Verification polling

For all `AwaitingRestart` records across all tenants:

- Load records with `status = 'awaiting_restart'`. For each record, use
  `execution_owner_service_id` (already stored on the row from when the agent claimed
  the update) as the target `service_id` for `notifier.send_to_service`. Do **not**
  perform a fresh `service_host` join — the owner is already on the record, and a fresh
  join could resolve a different (newer) agent connection for the same host.
  If `execution_owner_service_id IS NULL` on an `AwaitingRestart` record (should not
  happen given the transition invariant, but defensive programming applies), log a warning
  and skip — the scheduler will retry on the next tick.
- Dispatch `detect_version` via the existing plugin assignment for each record.
- The MQTT/channel layer handles delivery: immediate if agent is connected, queued in
  the outbox if offline (delivered on next reconnect).
- Handles the case where the agent stays connected after restart but returns `not_ready`
  repeatedly — re-dispatches on each scheduler tick without waiting for a reconnect event.
- If both a reconnect-triggered dispatch and a scheduler tick dispatch `detect_version`
  simultaneously, the controller handles both responses idempotently via CAS.
- Tick interval: same as existing scheduler poll interval (default 15 seconds).

#### 2. Timeout enforcement

For all `AwaitingRestart` records across all tenants where
`now > awaiting_restart_since + awaiting_restart_timeout`:

- CAS transition to `Failed` with reason "Awaiting restart timed out".
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
  `HostProgressionNeeded { host_id, tenant_id }` NATS message on the controller subject;
  the receiving controller dispatches inline. The `NoopSchedulerNotifier` (test-only, in
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
symlink path — this is acceptable.

```rust
pub struct ServiceMetadata {
    pub service_name: String,           // e.g. "uptrakit-controller-standalone"
    pub binary_path: Option<PathBuf>,   // std::env::current_exe(); None if Docker
    pub version: String,                // current running version
    pub deployment_topology: DeploymentTopology,
    pub reuseport_enabled: bool,        // whether SO_REUSEPORT takeover is active
    pub pid: u32,                       // current process PID for --takeover-from
}

pub enum DeploymentTopology {
    StandaloneBinary,
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

```rust
```

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
| `detect_version` | Shell | `version_command: "<binary_path> --version"` (binary path embedded literally at discovery time; no `{package_identifier}` substitution) |
| `fetch_releases` | `releases_github` | uptrakit GitHub repo; `tag_strip_prefix: "v"`; tag filter per service |
| `execute_update` | Shell (binary) or Docker plugin | `resumable: true`; binary path or container identity |

`package_identifier` for self-update software items is the service name string (e.g.
`"uptrakit-controller-standalone"`). It is used as the display identifier and for
deduplication, not for command substitution in the detect_version shell command (the
binary path is embedded literally in `version_command`).

### Deployment Topology Detection

The discovery plugin uses `DeploymentTopology` from the metadata interface:

- **`StandaloneBinary`**: execute_update = shell plugin with `resumable: true`. The update
  script is generated at discovery time from `ServiceMetadata` and branches on
  `reuseport_enabled`:

  **`reuseport_enabled = true` (preferred, zero-downtime, no dead-control-plane risk):**
  The script downloads the new release asset, replaces the binary at `binary_path`, then
  executes `<binary_path> --reuseport --takeover-from <pid>` (where `pid` is captured from
  `ServiceMetadata.pid` at discovery time and refreshed per-run via a `$UPTRAKIT_PID`
  env var injected by the shell plugin). The new binary binds the port via `SO_REUSEPORT`
  alongside the old process. The old process detects the takeover and drains active
  connections before exiting. If the new binary fails to start (crash, missing dependency,
  invalid config), it never successfully binds the port and the old process detects the
  child exit — **the old process remains running and continues serving**. No outage occurs.
  This is the primary dead-control-plane mitigation: the old process is the safety net and
  only exits after the new process is confirmed running.

  The `UpdateResultPayload { resumable: true }` is sent by the embedded agent before the
  post-update hook fires. The post-update hook is the takeover command. Because the old
  process survives a bad new binary, the controller receives the early result payload, and
  the embedded agent in the old process handles it — even if the new binary never starts.
  On the next scheduler tick, `detect_version` runs against whatever binary is current
  and determines `Completed` or `Failed` accordingly.

  **`reuseport_enabled = false` (fallback, requires supervisor and deferred restart):**
  When `SO_REUSEPORT` is not configured, the update script downloads and replaces the
  binary, then the post-update hook triggers a supervisor restart. For this path, the hook
  **must** use a deferred restart (e.g., `systemd-run --on-active=10s`), not an immediate
  one. The deferred gap serves two purposes: (1) the agent sends the early result payload
  and the controller commits the `AwaitingRestart` DB write before the process exits;
  (2) the supervisor can detect a fast-crashing new binary and restart the old one before
  the deferred timer fires. Immediate `systemctl restart` is explicitly prohibited on the
  fallback path — it races with the DB commit and can leave `InProgress` records that
  resolve as `Failed`. This path has a brief connectivity gap and does not provide the
  dead-control-plane protection that `--reuseport` does. It is supported but operators
  are strongly encouraged to enable `SO_REUSEPORT`.

  The `reuseport_enabled` flag is populated from `ServiceMetadata` at discovery time. The
  discovery plugin generates the appropriate update script inline in the shell plugin config
  `update_command`; no separate template files are needed.

- **`DockerContainer`**: execute_update = existing Docker plugin with `resumable: true`.
  The correct operation sequence is: pull new image → stop container → remove container →
  start new container with the new image tag. `docker restart` alone does not change the
  image and must not be used — it causes `detect_version` to return the old version and
  transition the update to `Failed`. The container name and image repository are sourced
  from `ServiceMetadata`.

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

async fn detect_host_compatibility(&self) -> Result<HostCompatibility> {
    // Returns Compatible only when running as embedded agent in controller-standalone.
    // Returns Incompatible otherwise.
    // Future: also Compatible when any uptrakit service is detected locally.
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
    /// scheduled batch tasks. Opt-in required; the controller-standalone
    /// enables it explicitly in its default plugin config for this plugin.
    #[serde(default)]
    pub enabled: bool,
}
```

`discover_software` returns `Ok(vec![])` immediately when `enabled == false`.
`detect_host_compatibility` returns `Incompatible("self-update disabled by config")` when
`enabled == false`. This prevents spurious "plugin is compatible but produces no items"
states in the registry and avoids running compatibility checks against hosts where the
feature is intentionally disabled.

### MVP Scope

MVP covers controller-standalone only. The `detect_host_compatibility` check returns
`Incompatible(reason)` on hosts where no supported uptrakit service is detected.

Future iterations (not in this spec):

- agent, agent-ssh, mqtt, scheduler as additional software items
- Multi-service coordination (e.g., update ordering when controller and agent are separate)
- Windows service topology

---

## Migration

Three migrations required:

1. Add `awaiting_restart_timeout INTEGER NULL` to `software_item`.
2. Add `awaiting_restart_since TIMESTAMPTZ NULL` to `update_history`.
3. Recreate the `uix_update_history_host_active` partial unique index to include
   `'awaiting_restart'` in its filter:

   ```sql
   -- current:  WHERE status IN ('pending', 'in_progress')
   -- new:       WHERE status IN ('pending', 'in_progress', 'awaiting_restart')
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

**Failed self-update and the control plane:** When `SO_REUSEPORT` is enabled (preferred),
a failed new binary never kills the old process — the old process is the safety net and
only exits after confirming the new binary has bound the port. The dead-control-plane risk
is eliminated for this path. When `SO_REUSEPORT` is not enabled (fallback path), the old
process exits before the new one is confirmed running. If the new binary fails to start,
the control plane is briefly dead until the supervisor restarts it. The self-update
discovery plugin checks for a running supervisor (systemd unit or Docker restart policy)
during `discover_software` and logs a warning when none is detected — this check is
**warning-only and never blocks discovery**; the software item is always emitted regardless
of supervisor presence. Supervisor detection is inherently unreliable (wrapper scripts, s6,
runit, symlinks, Docker containers all produce false negatives), so blocking on it would
silently suppress self-update in valid deployments. Operators using the fallback path are
responsible for ensuring a supervisor is in place; operators using `--reuseport` have no
supervisor dependency.

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

**Supervisor restart race (fallback path, `reuseport_enabled = false` only):** The
`--reuseport` takeover path has no restart race — the old process remains alive throughout
the takeover, so it can send the early result payload at any time without risk of being
killed. For the supervisor fallback path, an immediate restart (`systemctl restart`) races
with the controller's DB commit of `AwaitingRestart`. This is a spec constraint: the
post-update hook on the fallback path **must** use deferred restart (`systemd-run
--on-active=10s`) — immediate supervisor restart is not a valid configuration for resumable
updates on the fallback path. Operators using the fallback path without deferred restart
may observe updates incorrectly marked `Failed`.

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
- The self-update discovery plugin integration test requires a running controller-standalone
  instance; mark `#[ignore]` and document in testing.md.
