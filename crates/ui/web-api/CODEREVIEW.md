# Code Review: `uptrakit-web-api`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

`uptrakit-web-api` is operationally solid and heavily tested, but it still concentrates too much public surface and too much fire-and-forget behavior in a few hot spots. The old auth and token-revocation findings no longer reproduce in this pass.

## Strengths

- Good direct test coverage across routing, middleware, service WebSocket handling, and notifications.
- Current code uses bounded channels and explicit timeout handling instead of unbounded growth.
- The crate continues to keep most DB work delegated into `web-api-queries`.

## Active Findings

### [MEDIUM] Notification dispatch still drops events under backpressure

- Dimension: high availability, consistency
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs`
- Why it matters: the queue is now bounded, which is correct, but the delivery model is still "drop and warn" once the channel is full.
- Failure scenario: bursty update completions or a slow downstream notification path cause some user-visible notifications never to be delivered or retried.

### [MEDIUM] The crate still exports a broad internal module surface publicly

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`
- Why it matters: many modules that read like internal implementation details are still `pub mod`, which makes the crate harder to tighten up internally over time.
- Failure scenario: refactoring route, middleware, or broadcaster internals becomes risky because external code may already depend on implementation modules that should be private.

### [MEDIUM] `deliver_controller_event` remains a complexity hot spot

- Dimension: maintainability
- Scope: `crates/ui/web-api/src/event_delivery.rs:deliver_controller_event`
- Why it matters: Sentrux still flags this function above the configured cyclomatic complexity limit, and it sits on a cross-controller routing path where clarity matters.
- Failure scenario: a new cross-controller event type is added under failure pressure and subtly alters existing routing or local-side effects because the central dispatcher is already too dense.
