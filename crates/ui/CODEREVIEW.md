# Code Review: `crates/ui` Aggregate

- Review date: 2026-03-17
- Scope: current-state review of Rust UI/backend interface crates

## Summary

The UI-layer Rust crates compile, lint, and test cleanly. Their strongest qualities are typed
boundaries and good auth/query separation. This review cycle confirmed the existing active findings,
added new issues in the service WebSocket handler (production `unreachable!()` panics and
`start_paused` test violations), and found three consistency gaps across the route layer.

## Strengths

- `web-api-auth`, `web-api-queries`, and `web-api` divide responsibilities more cleanly than a
  monolithic HTTP crate would.
- The CLI continues to reuse the typed client and shared DTOs rather than drifting into a parallel
  protocol.
- The non-integration test sweep across these crates is strong.
- Permission extractors are consistently applied; no bypass paths were found in this review.
- Coding standards compliance is verified clean: `#[non_exhaustive]`, `Other(String)` catch-all,
  and `parking_lot` patterns are all correct.

## Active Findings

### [HIGH] Update-state recovery is still reconnect-scoped instead of system-scoped

- Dimension: high availability, database
- Scope: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`,
  `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- Why it matters: the web/API layer can clean up after an agent reconnects, but not after the
  wider class of failures where no reconnect happens.
- Failure scenario: agent, controller, DB, or network failure strands an `InProgress` row and the
  host remains blocked indefinitely.
- Details: `crates/ui/web-api-queries/CODEREVIEW.md`

### [HIGH] `unreachable!()` panics in production WebSocket capability dispatch

- Dimension: fault tolerance, maintainability
- Scope: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`
- Why it matters: three `unreachable!()` macros guard capability-to-message dispatch. A missed
  update when adding a new message variant terminates enrolled agent sessions with an unrecoverable
  panic.
- Details: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] `web-api` still exposes too much internal surface publicly

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`
- Why it matters: a wide `pub mod` surface makes refactors harder and blurs the intended public
  API of the crate.
- Failure scenario: future backend refactors are constrained by downstream internal-module usage
  that should never have been public.

### [MEDIUM] Fire-and-forget event paths still trade away convergence under pressure

- Dimension: high availability, consistency
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs`,
  `crates/ui/web-api/src/service_connections.rs`
- Why it matters: the bounded queues are good for memory safety, but the current choice is still
  silent drop rather than replay or explicit reconciliation.
- Failure scenario: bursty notification or config-fanout traffic fills the channel and some
  observers never see an otherwise valid state transition.
- Details: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] Validation is applied inconsistently between create and update handlers

- Dimension: coding standards, consistency
- Scope: `crates/ui/web-api/src/routes/software_items/mod.rs:update_software_item`,
  `crates/ui/web-api/src/routes/plugin_configs.rs:update_plugin_config`
- Why it matters: create handlers use `Validated<T>` extractors; update handlers for the same
  types use raw `Json<T>`, silently bypassing schema constraints. Invalid input reaches the query
  layer.
- Details: `crates/ui/web-api-queries/CODEREVIEW.md`

### [MEDIUM] Four `start_paused` rule violations in service WebSocket integration tests

- Dimension: test correctness
- Scope: `crates/ui/web-api/src/integration_tests/service_ws.rs`
- Why it matters: tests using `tokio::time::timeout()` without `start_paused = true` are non-
  deterministic and may flake on loaded CI runners.
- Details: `crates/ui/web-api/CODEREVIEW.md`
