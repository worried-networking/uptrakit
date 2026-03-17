# Code Review: `uptrakit-agent-core`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

`uptrakit-agent-core` remains the computational heart of version checks and update execution. The crate is functionally solid and heavily tested, but a few fallback paths still send definitive update failures best-effort rather than reliably.

## Strengths

- Shared batch and single-update logic keeps the agent binaries thin.
- Test coverage is substantially better than older review snapshots, especially around update batching and failure behavior.
- The crate uses the service transport abstraction cleanly and keeps command execution behind injected executors.

## Active Findings

### [MEDIUM] Some terminal update failures still use best-effort transport

- Dimension: high availability, consistency
- Scope: `crates/shared/agent-core/src/client.rs`
- Why it matters: normal `UpdateResult` delivery now uses the reliable send path, but the concurrent-update rejection and shutdown-timeout fallback paths still use `transport_send_best_effort`.
- Failure scenario: the agent is already on a degraded controller link when it rejects a second update or abandons an in-flight update during shutdown. The failure frame is dropped and the controller does not immediately converge to a terminal state.
