# Code Review: crates/core — Aggregate Summary

- **Review dates**: 2026-03-02, 2026-03-06, 2026-03-10, 2026-03-15
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure|14-dimension-parallel)
- **Branch**: docs/codereview-backend

## Overview

`crates/core/` contains the five runtime binaries and one integration-test crate that form the
operational heart of the Uptrakit workspace. Each sub-crate has its own detailed `CODEREVIEW.md`.
This file provides a cross-crate summary of the highest-severity findings and a quick-reference
index to the per-crate reviews.

## Sub-Crate Index

| Crate | File | Purpose |
| --- | --- | --- |
| `uptrakit-controller` | [controller/CODEREVIEW.md](controller/CODEREVIEW.md) | Primary binary: migrations, PKI, HA scheduler, web API |
| `uptrakit-agent` | [agent/CODEREVIEW.md](agent/CODEREVIEW.md) | Local agent: update execution, software discovery |
| `uptrakit-agent-ssh` | [agent-ssh/CODEREVIEW.md](agent-ssh/CODEREVIEW.md) | SSH-backed agent: remote provisioning, credential store |
| `uptrakit-mqtt` | [mqtt/CODEREVIEW.md](mqtt/CODEREVIEW.md) | MQTT bridge: per-tenant broker pool, HA discovery |
| `uptrakit-scheduler` | [scheduler/CODEREVIEW.md](scheduler/CODEREVIEW.md) | External scheduler: optimistic locking, task dispatch |
| `uptrakit-integration-tests` | [integration-tests/CODEREVIEW.md](integration-tests/CODEREVIEW.md) | Docker-based end-to-end and reverse-proxy tests |

## Cross-Crate Architecture Strengths

- All five binaries implement `ServiceHandler` from `uptrakit-service-sdk`, giving a consistent
  enrollment, reconnection, and shutdown lifecycle across the fleet.
- `CancellationToken` hierarchy is used uniformly for cooperative shutdown: root token in each
  binary propagated to all background tasks and spawned futures.
- `parking_lot::Mutex` used correctly everywhere — lock guards never held across `.await` points.
- `SecretString` at credential boundaries in MQTT and SSH crates; `Zeroizing<[u8; 32]>` for
  master keys in the controller and agent-ssh.
- Zero `unsafe` blocks across all five binaries.
- `#[non_exhaustive]` + wildcard arm with `tracing::warn!` on all public wire enums, consistent
  with the workspace standard.

## Cross-Crate Issues — Critical / High

### [CRITICAL] No crash recovery for in-flight updates (agent / agent-ssh)

If an agent process crashes while executing an update, the `update_history` row stays
`InProgress` indefinitely. The partial unique index on `host_id` blocks all subsequent updates
for that host until manual DB cleanup. A `StaleUpdateCleanup` scheduled task (transitioning
`InProgress` rows older than `TASK_EXECUTION_TIMEOUT` to `Failed`) is the recommended fix.
Tracked in: [agent/CODEREVIEW.md](agent/CODEREVIEW.md) — HA section.

### [HIGH] `agent-core` depends on `plugin-infrastructure-registry` (crate boundary violation)

`uptrakit-agent-core` imports `plugin-infrastructure-registry`, which bundles all 21 plugin
crates. The agent-core library should depend only on `plugin-infrastructure-core` (traits);
plugin selection belongs at the binary boundary.
Tracked in: [agent/CODEREVIEW.md](agent/CODEREVIEW.md) — Architecture section.

### [HIGH] `uptrakit-web-api` layering inversion from `uptrakit-controller`

`uptrakit-controller` directly depends on the UI-layer crate `uptrakit-web-api`. The controller
is a lower-layer binary; the web-api is higher-layer. This creates a layering inversion that
prevents clean separation of controller-vs-web-api concerns.
Tracked in: [controller/CODEREVIEW.md](controller/CODEREVIEW.md) — Architecture section.

## Cross-Crate Issues — Coding Standards (allow suppression violations)

The following `#[allow(clippy::too_many_arguments)]` suppressions are present without a
feature-gating justification. Each should be refactored with a parameter struct.

| Location | Crate |
| --- | --- |
| `src/tasks.rs:610` (`spawn_background_tasks`) | controller |
| `src/commands/bootstrap.rs:364` | agent-ssh |
| `src/ha_discovery/device.rs:88` | mqtt |

## Cross-Crate Issues — High Availability

- **Bounded event channel backpressure** (mqtt): 512-capacity channel between `TenantManager`
  and `MqttHandler` drops events silently under backpressure.
- **No circuit breaker** (mqtt): broker unavailability causes per-message timeouts with no
  backoff or fast-fail at the dispatch level.
- **Scheduler race window** (scheduler): up to one 15-second poll cycle where tasks are
  neither executed by the (disconnected) external scheduler nor by the embedded scheduler.
- **Sequential `BackgroundTasks::shutdown`** (controller): shutdown awaits tasks one by one;
  worst-case duration is the sum of all per-task timeouts rather than the maximum.

## Cross-Crate Test Gaps

- `AgentHandler::on_message` machine-ID validation — zero unit tests (agent).
- `spawn_background_tasks` path in controller — low unit coverage.
- `SchedulerHandler::on_connected` / `on_message` — only exercised by live integration (scheduler).
- Protocol-to-SSH bridge handlers in agent-ssh — only exercised by live integration.

## Approved `#[allow]` Suppressions (feature-gated, with comment)

| Location | Suppression | Status |
| --- | --- | --- |
| `agent/src/client.rs:31,43,44` | `#[allow(unused_variables)]` | Approved — feature-gated with comment |
| `agent/src/client.rs:37` | `#[allow(unreachable_code)]` | Approved — explains interactive feature |
| `agent-ssh/src/client.rs:512` | `#[allow(unused_mut)]` | Approved — feature-gated |

## 14-Dimension Parallel Review — 2026-03-15

Findings from a 14-dimension parallel review covering architecture, security, quality,
high-availability, coding standards, extensibility, testing, consistency, maintainability,
Rust idioms, crate structure, database, documentation, and cross-cutting concerns.

### Controller

| Severity | Dim | Location / Finding |
| --- | --- | --- |
| HIGH | D7 | `#[allow(clippy::too_many_arguments)]` at `main.rs:610` without comment |
| MEDIUM | D1 | `run()` function is 365 lines — numbered phases should be extracted to `startup/` module functions |
| MEDIUM | D12 | `reconcile_all_settings` (288 lines, nesting 7) — repetitive per-setting boilerplate, macro/helper candidate |
| LOW | D5 | `ca_key_store.write().await` in `spawn_ca_reload`/`spawn_ca_rotation` not protected by timeout |
| LOW | D5 | PKI HTTP server uses `track_abort()` rather than `track()` — in-flight OCSP/CRL requests may be interrupted |
| LOW | D3 | Production `.expect()` at `main.rs:85` — `tracing_journald::layer().expect("failed to connect to journald")` |
| LOW | D4 | Controller startup and PKI code has zero unit tests (6 files, ~1,200 lines) |
| INFO | D5 | Graceful shutdown well-architected with ordered teardown, service drain, scattered restart notifications |
| INFO | D5 | Zero-downtime restart via `SO_REUSEPORT` and `--takeover-from` PID signaling |
| INFO | D5 | DB connection pool properly configured (connect_timeout 8s, acquire_timeout 8s, idle_timeout 300s) |

### Agent

| Severity | Dim | Location / Finding |
| --- | --- | --- |
| INFO | D5 | Background task channel uses bounded `mpsc::channel(32)` — appropriate backpressure |
| INFO | D5 | Freeze file mechanism provides operator kill switch persisting across restarts |
| LOW | D5 | Update cooldown rate limiter only protects against rapid-fire, not sustained throughput |

### Agent-SSH

| Severity | Dim | Location / Finding |
| --- | --- | --- |
| MEDIUM | D12 | `run_proxmox_bootstrap` 387 lines, nesting 5 — extract named phases |
| MEDIUM | D1 | Contains own embedded database layer (entity definitions, 13 migrations) — migration machinery duplicated from `shared-db` |
| LOW | D5 | SSH connection pool has no maximum size limit — no eviction of idle sessions except on next `acquire()` |
| LOW | D10 | `Vec<&String>` at `bootstrap.rs:429` — `Vec<&str>` would be more idiomatic |
| LOW | D4 | SSH commands lack unit tests (`host.rs`, `bootstrap_proxmox.rs`, `sync.rs`) |

### MQTT

| Severity | Dim | Location / Finding |
| --- | --- | --- |
| MEDIUM | D7 | `#[allow(clippy::too_many_arguments)]` at `ha_discovery/device.rs:88` without comment |
| LOW | D5 | Event reporter uses `try_send()` dropping events on full channel — could miss command events |
| INFO | D5 | Reconnection uses exponential backoff (2s base, 60s cap) with `CancellationToken` integration |
| INFO | D5 | Operations have 5s timeout; `publish_or_abort!` prevents cascading delays |
| INFO | D5 | `pending_online`/`pending_ha_subscribe` retry avoids self-deadlock in poll callback |

### Scheduler

| Severity | Dim | Location / Finding |
| --- | --- | --- |
| INFO | D5 | Optimistic locking for task claiming — HA-safe across multiple instances |
| INFO | D5 | Stale claim recovery (600s timeout) handles crashed controllers |
| INFO | D5 | External scheduler handoff via `EmbeddedServiceNotifier` coexistence policy |
| LOW | D14 | Dead `oidc = []` feature flag — gates no code |

### Integration Tests

| Severity | Dim | Location / Finding |
| --- | --- | --- |
| INFO | D4 | Docker-based tests properly `#[ignore]`d with clear run instructions |

### Cross-Crate Strengths (New)

- All five binaries implement `ServiceHandler` from `service-sdk` — consistent
  enrollment/reconnection/shutdown lifecycle.
- `CancellationToken` hierarchy propagated to all background tasks uniformly.
- Zero `unsafe` blocks across all five binaries.
- Exponential backoff with jitter across all reconnection paths.
- Interruptible backoff sleeps respond to shutdown signals within one interval.
