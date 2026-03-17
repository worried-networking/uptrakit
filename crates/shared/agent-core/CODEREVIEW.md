# Code Review: `uptrakit-agent-core`

- Review date: 2026-03-17
- Scope: current-state review (full 14-dimension)

## Summary

`uptrakit-agent-core` remains the computational heart of version checks, update execution, and
software discovery. The crate is functionally solid and heavily tested, but two reliability
concerns persist: terminal update failures use best-effort transport in edge cases, and the
`start_update` path clones the full payload unnecessarily.

## Strengths

- Shared batch and single-update logic keeps the agent binaries thin.
- Test coverage is substantially better than older review snapshots, especially around update
  batching, timeout behavior, and unknown plugin type handling.
- The crate uses the service transport abstraction cleanly and keeps command execution behind
  injected executors.
- `MAX_OUTPUT_BYTES` (10 MB) cap with `append_bounded` prevents OOM from runaway commands.
- Background task spawning via `spawn_background` prevents WebSocket write timeouts by running
  long operations off the event loop.
- Graceful shutdown drains in-flight update output before sending `Disconnecting`.
- Interactive update support is cleanly feature-gated behind `#[cfg(feature = "interactive")]`.

## Active Findings

### [MEDIUM] Some terminal update failures still use best-effort transport

- **Dimension**: high availability, consistency
- **Scope**: `crates/shared/agent-core/src/client.rs:386-394` (concurrent-update rejection),
  `crates/shared/agent-core/src/client.rs:191-198` (shutdown-timeout fallback)
- **Description**: Normal `UpdateResult` delivery now uses the reliable send path
  (`transport_send`), but the concurrent-update rejection and shutdown-timeout fallback paths
  still use `transport_send_best_effort`.
- **Why it matters**: these are the two edge cases where the controller most needs to learn
  about a terminal state to avoid leaving an update record stuck as "in_progress".
- **Failure scenario**: the agent is already on a degraded controller link when it rejects a
  second update or abandons an in-flight update during shutdown. The failure frame is dropped
  and the controller does not immediately converge to a terminal state.

### [LOW] `start_update` clones the entire `ExecuteUpdatePayload` before applying context

- **Dimension**: allocation, performance
- **Scope**: `crates/shared/agent-core/src/client.rs:302`
- **Description**: `start_update` calls `payload.clone()` on the entire `ExecuteUpdatePayload`
  (including nested `serde_json::Value` plugin configs and optional `ReleaseInfo` with asset
  lists) solely to mutate the config fields with connection context.
- **Why it matters**: for updates with large plugin configs or many release assets, this
  allocates an unnecessary deep copy on every dispatch. The original `payload` is never used
  after the clone.
- **Failure scenario**: not a correctness issue, but increases GC pressure on the agent for
  every update dispatch.
