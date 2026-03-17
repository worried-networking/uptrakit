# Code Review: `crates/core` Aggregate

- Review date: 2026-03-17
- Scope: current-state review of runtime binaries only. `crates/core/integration-tests` excluded.

## Summary

The runtime crates are operationally solid in the common path: they share the same lifecycle model
via `service-sdk`, compile and test cleanly, and consistently use cancellation-aware async patterns.
This review cycle confirmed the existing failure-recovery gaps and added new findings in the
notification dispatcher's supervision model, MQTT's partial-state buffer lifetime, cross-crate code
duplication, and a CRL numbering concern for multi-controller HA.

## Strengths

- All runtime binaries use the same enrollment and reconnect machinery from `uptrakit-service-sdk`.
- Cancellation and shutdown flow are broadly consistent across controller, agents, scheduler, and
  MQTT service.
- The core crates remain free of `unsafe` in production paths (the single `unsafe` in
  `master_key.rs` is documented as startup-only, single-threaded).
- MQTT reconnection uses exponential backoff (2 s-60 s) with reset on successful ConnAck.
- WS write path is guarded by a `SEND_TIMEOUT` (30 s) to prevent indefinite TCP send-buffer
  blocking.
- SSH connection pooling uses TTL-based eviction, reconnects outside the lock, and respects
  `parking_lot::Mutex` per workspace coding standards.

## Active Findings

### [HIGH] Orphaned in-progress updates still lack a generic recovery path

- Dimension: high availability, database
- Scope: controller, scheduler, agent, and agent-ssh interaction around `update_history`
- Why it matters: reconnect-time cleanup exists, but the runtime layer still has no scheduler-
  driven or age-driven cleanup for updates stranded by broader failures such as simultaneous
  controller + agent restart.
- Failure scenario: any node crashes or the network dies mid-update and the original agent never
  reconnects. The host remains logically locked.

### [MEDIUM] `agent-ssh` still uses best-effort transport for some definitive failure messages

- Dimension: fault tolerance, consistency
- Scope: `crates/core/agent-ssh/src/client.rs`, `crates/core/agent-ssh/src/main.rs`
- Why it matters: setup failures and shutdown-timeout failures can be dropped when the controller
  link is already degraded, leaving the controller without an authoritative terminal result.
- Failure scenario: remote host is unreachable while the controller connection is also lagging or
  resetting. The user sees a stuck update until separate recovery logic runs.

### [MEDIUM] MQTT status and command fan-out still drops events on a full channel

- Dimension: high availability, observability
- Scope: `crates/core/mqtt/src/mqtt_client.rs`, `crates/core/mqtt/src/tenant_manager.rs`
- Why it matters: the bounded channel protects memory, but controller-visible status changes can
  be lost during broker churn or bursty workloads.
- Failure scenario: broker reconnect storm or slow downstream processing fills the event channel
  and the service drops `Status`, `Reconnected`, or command events.

### [MEDIUM] `dispatch_loop` blocks indefinitely if all event-producer senders drop

- Dimension: fault tolerance, observability
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs:dispatch_loop`
- Why it matters: `rx.recv().await` has no timeout. If every event-producer task exits (e.g., due
  to a controller subsystem failure), the dispatch loop stalls without any observable signal.
  Spawned delivery tasks are detached; a delivery panic leaves notification log entries permanently
  `pending`.
- Fix: add a shutdown channel or timeout on the `recv()` call; use `JoinSet` to supervise delivery
  tasks and log panics.

### [MEDIUM] Incomplete multi-page `SoftwareStates` buffers are never garbage-collected

- Dimension: fault tolerance, memory
- Scope: `crates/core/mqtt/src/tenant_manager.rs`, partial_states buffer
- Why it matters: multi-page `SoftwareStates` payloads are buffered per client. If a client
  receives page 0 but the controller disconnects before page 1 arrives, the incomplete entry lives
  in memory indefinitely with no TTL or cleanup pass.
- Failure scenario: a flapping MQTT connection generates many orphaned partial-state entries; over
  time they accumulate and waste memory without any operator-visible signal.
- Fix: add a TTL (e.g., 5 minutes) to each `PartialSoftwareStates` entry and run a periodic
  cleanup pass.

### [MEDIUM] `init_tracing` is duplicated across 4 non-controller binaries

- Dimension: maintainability, crate structure
- Scope: `crates/core/agent/src/main.rs`, `crates/core/agent-ssh/src/main.rs`,
  `crates/core/mqtt/src/main.rs`, `crates/core/scheduler/src/main.rs`
- Why it matters: all four binaries contain a near-identical `init_tracing` function (same
  verbosity mapping, same `EnvFilter` construction). Any change to the tracing setup needs to be
  replicated four times.
- Failure scenario: a tracing filter fix lands in one binary but not the others, causing
  inconsistent log output or missing debug information during incident triage.
- Fix: move `init_tracing` into `uptrakit-service-sdk` alongside the existing lifecycle helpers.

### [MEDIUM] `register_column_aad_mappings` is duplicated between controller and scheduler

- Dimension: maintainability, security
- Scope: `crates/core/controller/src/main.rs`, `crates/core/controller/src/reencrypt.rs`,
  `crates/core/controller/src/pki.rs`, `crates/core/scheduler/src/handler.rs`
- Why it matters: the controller and scheduler each maintain their own copy of the AAD column
  registration calls. If a new encrypted column is added and only one site is updated, decryption
  or re-encryption will use the wrong AAD context, producing silent data corruption.
- Failure scenario: a new encrypted column is added to a SeaORM entity. The controller registers
  its AAD mapping but the scheduler copy is missed. The scheduler's `init_data_key_ring` then
  uses a zero/default AAD for that column, and any re-encryption pass silently corrupts the
  ciphertext.
- Fix: extract a single shared `register_all_aad_mappings()` function into a common crate
  (e.g., `uptrakit-crypto` or a new `uptrakit-db-crypto` crate).

### [INFO] Controller startup does not retry on transient database errors

- Dimension: fault tolerance
- Scope: `crates/core/controller/src/main.rs`, startup migration and DB-init path
- Why it matters: a transient DB unavailability at boot (e.g., PostgreSQL not yet ready in a
  container startup race) immediately fails the controller process, requiring an external restart
  loop.
- Recommendation: add an exponential-backoff retry loop around the DB migration and initialization
  step for deployments where DB readiness is not guaranteed before the controller starts.
