# Code Review: `crates/core` Aggregate

- Review date: 2026-03-17
- Scope: current-state review of runtime binaries only. `crates/core/integration-tests` was excluded.

## Summary

The runtime crates are operationally solid in the common path: they share the same lifecycle model via `service-sdk`, compile and test cleanly, and consistently use cancellation-aware async patterns. The remaining issues are failure-recovery gaps and a few places where definitive status is still sent as best-effort.

## Strengths

- All runtime binaries use the same enrollment and reconnect machinery from `uptrakit-service-sdk`.
- Cancellation and shutdown flow are broadly consistent across controller, agents, scheduler, and MQTT service.
- The core crates remain free of `unsafe` in production paths.

## Active Findings

### [HIGH] Orphaned in-progress updates still lack a generic recovery path

- Dimension: high availability, database
- Scope: controller, scheduler, agent, and agent-ssh interaction around `update_history`
- Why it matters: reconnect-time cleanup exists, but the runtime layer still has no scheduler-driven or age-driven cleanup for updates stranded by broader failures.
- Failure scenario: any node crashes or the network dies mid-update and the original agent never reconnects. The host remains logically locked.

### [MEDIUM] `agent-ssh` still uses best-effort transport for some definitive failure messages

- Dimension: fault tolerance, consistency
- Scope: `crates/core/agent-ssh/src/client.rs`, `crates/core/agent-ssh/src/main.rs`
- Why it matters: setup failures and shutdown-timeout failures can be dropped when the controller link is already degraded, leaving the controller without an authoritative terminal result.
- Failure scenario: remote host is unreachable while the controller connection is also lagging or resetting. The user sees a stuck update until separate recovery logic runs.

### [MEDIUM] MQTT status and command fan-out still drops events on a full channel

- Dimension: high availability, observability
- Scope: `crates/core/mqtt/src/mqtt_client.rs`, `crates/core/mqtt/src/tenant_manager.rs`
- Why it matters: the bounded channel protects memory, but controller-visible status changes can be lost during broker churn or bursty workloads.
- Failure scenario: broker reconnect storm or slow downstream processing fills the event channel and the service drops `Status`, `Reconnected`, or command events.
