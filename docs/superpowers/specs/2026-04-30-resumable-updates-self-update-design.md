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

---

### `UpdateStatus` New Variant

```rust
AwaitingRestart,
```

Added to the existing `UpdateStatus` enum in `crates/shared/types`. Follows all existing
extensibility patterns (`#[non_exhaustive]`, wire-safe serialization).

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
AwaitingRestart ──── awaiting_restart_timeout exceeded ──► Failed
  │                                                         ▲
  │ agent reconnects,                                       │
  │ detect_version returns "not ready"                      │
  │  └─► stay AwaitingRestart, retry next reconnect         │
  │                                                         │
  │ agent reconnects,                                       │
  │ detect_version returns version mismatch ────────────────┘
  │
  │ agent reconnects,
  │ detect_version matches to_version
  ▼
Completed
```

All transitions use CAS (`rows_affected == 0` = another controller already acted, skip).

---

### `detect_version` Protocol Extension

`VersionCheckResult` gains a new optional field:

```rust
pub not_ready: Option<bool>,  // true = software is initializing, retry later
```

Semantics:

- `not_ready: Some(true)` — software is mid-boot or not yet queryable. Controller stays in
  `AwaitingRestart` and retries. Does **not** count as a version mismatch.
- `error: Some(_)` — transient infrastructure failure (plugin crash, timeout). Controller
  stays in `AwaitingRestart` and retries on next reconnect.
- `installed_version: Some(v)` where `v != to_version` — genuine failure. Transition to
  `Failed`.
- `installed_version: Some(v)` where `v == to_version` — success. Transition to `Completed`.

Plugin implementations return `not_ready: true` when they can detect the process is starting
but cannot yet determine its version (e.g., service not yet accepting connections).

---

### Reconnect Handler Changes

**Location:** `service_ws/handler/updates.rs` → calls into
`update_batches/dispatch.rs::mark_owned_in_progress_as_failed_on_reconnect`

Current behavior: all `InProgress` records owned by the reconnecting agent's `service_id` →
`Failed`.

New behavior — before the mass-fail, per-record branch:

1. Load all `InProgress` update_history records for the reconnecting `machine_id`.
2. For each record, join to `host_software_item_plugin` (role = `execute_update`) to read
   `resumable`.
3. If `resumable = true`: CAS transition `InProgress → AwaitingRestart`.
4. If `resumable = false`: `Failed` (unchanged).
5. After branching: load all `AwaitingRestart` records for that `machine_id` (including
   newly transitioned ones) → dispatch `detect_version` for each.

**Standalone self-update hole:** When controller-standalone restarts, the controller and
embedded agent die together. No surviving controller transitions the `InProgress` record.
When the new process comes up and the embedded agent reconnects, step 1 above finds the
stale `InProgress` resumable record and handles it correctly. No separate startup scan
is needed.

---

### Scheduler Changes

The scheduler gains two new periodic responsibilities for `AwaitingRestart` records:

#### 1. Active verification polling

For each `AwaitingRestart` record where the agent is currently connected:

- Dispatch `detect_version` via the existing plugin assignment.
- Handles the case where the agent stays connected but returns `not_ready` repeatedly —
  the scheduler re-dispatches on each tick rather than waiting for another reconnect event.
- Tick interval: same as existing scheduler poll interval (default 15 seconds).

#### 2. Timeout enforcement

For each `AwaitingRestart` record where `now > updated_at + awaiting_restart_timeout`:

- CAS transition to `Failed` with reason "Awaiting restart timed out".
- Fires regardless of agent connection state.

Both responsibilities execute within the existing scheduler poll cycle.

---

### Batch Sequencing

`AwaitingRestart` is treated as an active update. A host with an item in `AwaitingRestart`
does not promote the next `Queued` item. The per-host sequential guarantee is fully
maintained. This is critical for kernel/firmware updates where the host is mid-reboot and
dispatching another update would be unsafe.

---

## Part 2: Uptrakit Self-Update Discovery Plugin

### Overview

A new discovery plugin (`crates/plugins/discovery/uptrakit-self-update/`) implements the
`Discoverer` trait. It auto-discovers uptrakit services running on the current host and
creates software items with pre-wired plugin assignments, including `resumable = true` on
the execute_update assignment. No manual configuration required.

### Service Metadata Interface

The discovery plugin must not hardcode binary paths or deployment assumptions. Instead, it
queries the controller (and future: other services) for their own metadata at discovery time.

The controller exposes a `ServiceMetadata` structure queryable by the embedded agent via an
internal interface:

```rust
pub struct ServiceMetadata {
    pub service_name: String,       // e.g. "uptrakit-controller-standalone"
    pub binary_path: Option<PathBuf>, // from std::env::current_exe(); None if Docker
    pub version: String,            // current running version
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

- **`StandaloneBinary`**: execute_update = shell plugin; script downloads release asset from
  GitHub, replaces binary at queried path, triggers restart via system supervisor
  (systemd, launchd, etc.). The restart mechanism is part of the execute_update script
  config, also sourced from service metadata where applicable.
- **`DockerContainer`**: execute_update = existing Docker plugin; container identity from
  metadata; image tag updated and container restarted via Docker socket.

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

async fn detect_host_compatibility(&self) -> HostCompatibility {
    // Compatible only when running as embedded agent in controller-standalone
    // Future: also compatible when any uptrakit service is detected locally
}
```

### MVP Scope

MVP covers controller-standalone only. The `detect_host_compatibility` check returns
`Incompatible` (or `Unsupported`) on hosts where no supported uptrakit service is detected.

Future iterations (not in this spec):

- agent, agent-ssh, mqtt, scheduler as additional software items
- Multi-service coordination (e.g., update ordering when controller and agent are separate)
- Windows service topology

---

## Migration

Two new migrations required:

1. Add `resumable BOOL NOT NULL DEFAULT FALSE` to `host_software_item_plugin`.
2. Add `awaiting_restart_timeout INTEGER NULL` to `software_item`.

Both are additive, backward-compatible, non-destructive.

---

## Quality Gates

Standard gates apply. Additionally:

- Unit tests for reconnect handler branching (resumable vs non-resumable InProgress records).
- Unit tests for `AwaitingRestart` → `Completed` and `AwaitingRestart` → `Failed` transitions.
- Unit tests for `detect_version` "not ready" handling — verifies controller stays in
  `AwaitingRestart` and does not transition to `Failed`.
- Unit tests for timeout enforcement (use `start_paused = true`; only for tests calling
  tokio time APIs).
- The self-update discovery plugin integration test requires a running controller-standalone
  instance; mark `#[ignore]` and document in testing.md.
