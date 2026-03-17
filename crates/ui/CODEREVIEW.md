# Code Review: `crates/ui` Aggregate

- Review date: 2026-03-17
- Scope: current-state review of Rust UI/backend interface crates

## Summary

The UI-layer Rust crates compile, lint, and test cleanly. Their strongest qualities are typed boundaries and good auth/query separation. The live issues are concentrated in update-state recovery, lossy fire-and-forget signaling, and public/internal API boundaries.

## Strengths

- `web-api-auth`, `web-api-queries`, and `web-api` still divide responsibilities more cleanly than a monolithic HTTP crate would.
- The CLI continues to reuse the typed client and shared DTOs rather than drifting into a parallel protocol.
- The current non-integration test sweep across these crates is strong.

## Active Findings

### [HIGH] Update-state recovery is still reconnect-scoped instead of system-scoped

- Dimension: high availability, database
- Scope: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`, `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Why it matters: the web/API layer can clean up after an agent reconnects, but not after the wider class of failures where no reconnect happens.
- Failure scenario: agent, controller, DB, or network failure strands an `InProgress` row and the host remains blocked indefinitely.

### [MEDIUM] `web-api` still exposes too much internal surface publicly

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`
- Why it matters: a wide `pub mod` surface makes refactors harder and blurs the intended public API of the crate.
- Failure scenario: future backend refactors are constrained by downstream internal-module usage that should never have been public.

### [MEDIUM] Fire-and-forget event paths still trade away convergence under pressure

- Dimension: high availability, consistency
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs`, `crates/ui/web-api/src/service_connections.rs`
- Why it matters: the bounded queues are good for memory safety, but the current choice is still silent drop rather than replay or explicit reconciliation.
- Failure scenario: bursty notification or config-fanout traffic fills the channel and some observers never see an otherwise valid state transition.
