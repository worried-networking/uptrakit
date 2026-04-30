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
  │                                                                        │
  │ agent disconnects (no resumable signal received)                       │ awaiting_restart_since
  ▼                                                              ──────────┤ + timeout exceeded
Failed                                                           │         ▼
                                                                 │      Failed
                                                  AwaitingRestart│
                                                      │          │
                                          agent reconnects or    │
                                          scheduler tick,        │
                                          detect_version:        │
                                                      │          │
                                            not_ready │──────────┘ (stay, retry)
                                                      │
                                             version mismatch ──► Failed
                                                      │
                                             version matches
                                                      ▼
                                                 Completed
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

- `status: Completed, resumable: Some(true)` → CAS `InProgress → AwaitingRestart`,
  set `awaiting_restart_since = now`. Do not call `dispatch_next_in_batch` yet.
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

The caller branches on `resumable`:

```rust
let result = execute_update_pipeline(&payload, &output_tx, executor, &mut output).await;

if result.resumable {
    // Send result before post-hooks — controller transitions to AwaitingRestart.
    send_update_result(&result_tx, resumable: true).await;
    // Post-hooks fire-and-forget: result ignored, errors logged only.
    tokio::spawn(run_post_hook_plugins(payload.post_update_hook_plugins, ...));
} else {
    // Normal path: post-hooks complete before result is sent.
    run_post_hook_plugins(&payload.post_update_hook_plugins, ...).await;
    send_update_result(&result_tx, resumable: false).await;
}
```

**Why send before post-hooks for resumable updates:** the post-update hook for a resumable
update typically triggers the restart (e.g., `systemctl restart uptrakit`,
`shutdown -r now`). If the result were sent after the hook, the process would be dead before
the payload reaches the controller. Sending first ensures `InProgress → AwaitingRestart`
is committed before the process exits.

**Post-hook result ignored for resumable:** once the controller is in `AwaitingRestart`,
the update outcome is determined by `detect_version` on reconnect — not by whether
post-hooks succeeded. Logging hook errors is sufficient.

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
`host_software_item_id + status = AwaitingRestart`.

If `host_software_item_id` is absent from the `VersionCheckResult` (old agent, partial
payload), skip the `AwaitingRestart` correlation — do not attempt an ambiguous host-level
scan. The scheduler will retry on the next tick.

---

### Reconnect Handler

**No changes required.** Agent disconnect from `InProgress` → `Failed` (existing behavior,
unchanged). When an update is resumable, the agent sends `UpdateResultPayload { resumable:
true }` before triggering the restart; the controller is already in `AwaitingRestart` when
the agent disconnects. Disconnect from `AwaitingRestart` is a no-op — the controller
continues waiting.

---

### Scheduler Changes

The scheduler gains two new cross-tenant responsibilities — **not** as new `ScheduledTaskType`
rows (which are per-tenant). These are implemented as a new executor registered once on the
`Scheduler` (not tied to a `ScheduledTaskType` DB row), using the `SchedulerNotifier`
already available to all executors.

#### 1. Verification polling

For all `AwaitingRestart` records across all tenants:

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

Future services (agent, agent-ssh, mqtt, scheduler) will implement the same metadata
interface when self-update support is extended to them.

### Plugin Assignment Matrix

For each discovered service, the plugin creates assignments:

| Role | Plugin | Config |
| --- | --- | --- |
| `detect_version` | Shell | `binary_path --version` (queried at discovery time) |
| `fetch_releases` | `releases_github` | uptrakit GitHub repo; tag filter per service |
| `execute_update` | Shell (binary) or Docker plugin | `resumable: true`; binary path or container identity |

### Deployment Topology Detection

The discovery plugin uses `DeploymentTopology` from the metadata interface:

- **`StandaloneBinary`**: execute_update = shell plugin with `resumable: true`. The script:
  downloads the new release asset, replaces the binary at the queried path, then starts
  the new binary with `--reuseport --takeover-from <current_pid>` using the existing
  graceful restart mechanism (`docs/development/graceful-restart.md`). This gives
  zero-downtime restart — the old process drains connections and exits cleanly. The agent
  sends `UpdateResultPayload { resumable: true }` before the post-update hook fires. The
  post-update hook (shell or systemd plugin) triggers the restart. A supervisor restart
  (`systemctl restart`) is an acceptable fallback when `--reuseport` is not configured,
  but requires the post-update hook to use a deferred restart (e.g.,
  `systemd-run --on-active=10s`) so the process stays alive long enough to send the
  `UpdateResultPayload`.

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
time. Implementors must verify that the version string passed to
`CreateUpdateRecordParams::to_version` at trigger time is the stripped form, not the raw
tag.

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

**Supervisor restart race (fallback path only):** When the graceful restart mechanism is
not available and the post-update hook uses an immediate supervisor restart, there is a
small window where the process can be killed before `UpdateResultPayload { resumable: true }`
reaches the controller. The update is then marked `Failed` even though it succeeded.
Mitigated by using deferred restart (`systemd-run --on-active=10s`) in the hook.

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
- Unit tests for timeout enforcement (use `start_paused = true`; only for tests calling
  tokio time APIs).
- Unit test for batch completion check: batch with one `AwaitingRestart` item must not be
  marked complete.
- The self-update discovery plugin integration test requires a running controller-standalone
  instance; mark `#[ignore]` and document in testing.md.
