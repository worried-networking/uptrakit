# Code Review: `uptrakit-web-api`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

`uptrakit-web-api` is operationally solid and heavily tested. This review cycle added new findings
in three areas: two production `unreachable!()` panics in the enrolled WebSocket handler, test
timing rule violations in the service-WebSocket integration tests, and structural code quality
issues in the route layer (CC violation, large files, and duplicated error response boilerplate).
The old auth and token-revocation findings no longer reproduce.

## Strengths

- Good direct test coverage across routing, middleware, service WebSocket handling, and
  notifications.
- Bounded channels and explicit timeout handling are used throughout instead of unbounded queues.
- The crate delegates all DB work into `web-api-queries`, keeping route handlers focused.
- Dangerous command detection and audit logging are applied consistently in plugin config routes.
- Permission extractors are generated consistently; no hand-written bypass paths were found.

## Active Findings

### [HIGH] `unreachable!()` panics in capability dispatch arms of the enrolled WS handler

- Dimension: fault tolerance, maintainability
- Scope:
  `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`, capability dispatch match arms
- Why it matters: three `unreachable!()` macros protect the assumption that message variants always
  correspond to declared capabilities. If a new message type is added to the wire protocol without
  updating every dispatch arm, the panic is reachable in production and terminates the enrolled
  agent connection.
- Failure scenario: a new `ServiceMessage` variant is added; the developer updates the wire
  definition and the outer router but misses an inner capability dispatch arm; the first enrolled
  agent that sends that message triggers a panic in the handler task.
- Fix: replace `unreachable!()` with `tracing::error!(...)` + return a
  `ControllerMessage::Error` response. Add a capability-to-message contract table (comment or
  const array) that documents which messages require which capability.

### [MEDIUM] Notification dispatch still drops events under backpressure

- Dimension: high availability, consistency
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs`
- Why it matters: the queue is bounded, which is correct for memory safety, but the delivery model
  is still "drop and warn" once the channel fills. No backlog, replay, or explicit reconciliation
  exists.
- Failure scenario: bursty update completions or a slow downstream notification path cause some
  user-visible notifications never to be delivered.

### [MEDIUM] `dispatch_loop` has no timeout on `recv()` and spawns unmonitored delivery tasks

- Dimension: fault tolerance, observability
- Scope: `crates/ui/web-api/src/notifications/dispatcher.rs:dispatch_loop`
- Why it matters: if all event-producer senders are dropped, `rx.recv().await` blocks indefinitely
  without any observable signal. Delivery tasks are spawned with no supervisor; a panic leaves a
  notification log entry stuck in the `pending` state forever with no cleanup.
- Fix: add a `tokio::time::timeout` on the `recv()` call (or use a `select!` with a shutdown
  channel). Track spawned delivery task handles and log panics from `JoinSet` or similar.

### [MEDIUM] The crate still exports a broad internal module surface publicly

- Dimension: architecture, maintainability
- Scope: `crates/ui/web-api/src/lib.rs`
- Why it matters: many modules that read like internal implementation details are still `pub mod`,
  making the crate harder to tighten up internally over time.
- Failure scenario: refactoring route, middleware, or broadcaster internals is constrained because
  external code may already depend on implementation modules that should be private.

### [MEDIUM] `deliver_controller_event` remains a complexity hot spot (CC=37)

- Dimension: maintainability, code quality
- Scope: `crates/ui/web-api/src/event_delivery.rs:deliver_controller_event`
- Why it matters: Sentrux still flags this function above the configured cyclomatic complexity
  limit. The function handles 12 distinct message types with nested conditional logic in several
  arms (notably `WorkloadClaimAnnouncement` with 3+ nesting levels and scattered time-parsing
  fallbacks).
- Fix: extract each message type into a dedicated private function
  (e.g., `handle_workload_claim_announcement`, `handle_token_revoked`). The revocation loop and
  re-grant logic are the strongest candidates for extraction.

### [MEDIUM] `plugin_configs.rs` bundles five distinct concerns in one 1385-line file

- Dimension: code quality, maintainability
- Scope: `crates/ui/web-api/src/routes/plugin_configs.rs`
- Why it matters: HTTP endpoint handlers, dangerous-command detection, command-field traversal,
  audit-log helpers, plugin-config test dispatch, and the test suite are all interleaved in one
  file. Navigating and changing any single concern requires reasoning across 1385 lines.
- Fix: extract `dangerous_patterns.rs` (pattern detection + field traversal) and `audit.rs`
  (audit-log helpers) as sub-modules; move tests to a separate `tests/` integration test file.

### [MEDIUM] HTTP + security policy logic is merged inside route handlers

- Dimension: code quality, testability
- Scope: `crates/ui/web-api/src/routes/plugin_configs.rs:create_plugin_config`,
  `update_plugin_config`
- Why it matters: dangerous command checking and audit logging are embedded in the HTTP handler
  body, making it impossible to test security policy without a full HTTP stack. The same policy
  cannot easily be reused from CLI or batch paths.
- Fix: extract a `PluginConfigSecurityPolicy` service that validates config against dangerous
  patterns and can be called from any path without HTTP context.

### [MEDIUM] `error_response` called 385 times with repetitive boilerplate across route files

- Dimension: code quality, maintainability
- Scope: `crates/ui/web-api/src/routes/` (all route files)
- Why it matters: `not-found`, `db-error`, and `validation-error` response shapes are constructed
  inline in every route handler with nearly identical code. Changing the error response format
  (e.g., adding request correlation IDs) requires touching 20+ files.
- Fix: introduce an `error_builders` module with `not_found_response(resource)`,
  `db_error_response(error)`, and `handle_query_error(report)` helpers.

### [MEDIUM] Four `start_paused` rule violations in service WebSocket integration tests

- Dimension: test correctness
- Scope: `crates/ui/web-api/src/integration_tests/service_ws.rs` (four test functions:
  `anonymous_connect_and_enroll`, `enrolled_reconnect_with_bearer`,
  `service_connection_registry_send`, `service_connection_registry_broadcast`)
- Why it matters: all four functions call `tokio::time::timeout()` without
  `#[tokio::test(start_paused = true)]`, violating the project rule. Tests using any tokio time
  API must declare `start_paused = true` to ensure deterministic timing.
- Fix: add `start_paused = true` to all four test attributes.
