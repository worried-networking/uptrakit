# Code Review: `uptrakit-web-api`

- Review date: 2026-03-17
- Scope: full 14-dimension review of all ~118 .rs files

## Summary

`uptrakit-web-api` is operationally solid and heavily tested. This review cycle revalidated all
prior findings against current code. The three `unreachable!()` panics in the WebSocket capability
dispatch are still present and remain the highest-priority production risk. The notification
dispatcher fire-and-forget semantics, `start_paused` test violations, code quality issues in the
route layer, and the broad public module surface are all still active. One prior finding
(`deliver_controller_event` CC=37) has been downgraded after re-measurement. A new finding was
added for the inconsistent `Validated<>` vs `Json<>` usage across notification routes.

## Strengths

- Good direct test coverage across routing, middleware, service WebSocket handling, and
  notifications.
- Bounded channels and explicit timeout handling are used throughout instead of unbounded queues.
- The crate delegates all DB work into `web-api-queries`, keeping route handlers focused.
- Dangerous command detection and audit logging are applied consistently in plugin config routes.
- Permission extractors are generated consistently; no hand-written bypass paths were found.
- No `unwrap()`, `panic!()`, `todo!()`, or `unimplemented!()` calls exist in production code
  paths. All such calls are confined to `#[cfg(test)]` modules.
- Security headers middleware covers all standard headers (HSTS, X-Frame-Options, CSP
  frame-ancestors, X-Content-Type-Options, Referrer-Policy, Permissions-Policy).
- Rate limiting is applied to all public auth endpoints with a fail-closed local fallback when the
  DB rate limit store is unavailable.
- The WebSocket main dispatch function uses a catch-all arm that returns a proper `Error` response
  and breaks the connection (line 365-368 of `handler/mod.rs`), so unknown top-level messages are
  handled gracefully. The `unreachable!()` problem is confined to the three inner dispatch
  functions.

## Active Findings

### [HIGH] `unreachable!()` panics in capability dispatch arms of the enrolled WS handler

- **Dimension**: fault tolerance, maintainability
- **Scope**: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs:420`, `:444`, `:465`
- **Description**: Three `unreachable!()` macros protect the assumption that message variants
  always correspond to declared capabilities in `dispatch_update_hooks`,
  `dispatch_update_tracking`, and `dispatch_extensions`.
- **Why it matters**: If a new message type is added to the wire protocol without updating every
  inner dispatch arm, the panic is reachable in production and terminates the enrolled agent
  connection. The outer `dispatch()` function already has a proper catch-all at line 365 that
  returns an error response, but these inner functions do not.
- **Failure scenario**: A developer adds a new `ServiceMessage` variant, updates the outer router
  but misses an inner capability dispatch arm. The first enrolled agent that sends that message
  triggers a panic in the handler task, killing the connection with no error message to the agent.
- **Fix**: Replace each `unreachable!()` with `tracing::error!(...)` plus a
  `ProcessorResponse::reply(ControllerMessage::Error(...))` response, matching the pattern already
  used at line 365.

### [MEDIUM] Notification dispatch drops events under backpressure with no recovery

- **Dimension**: high availability, consistency
- **Scope**: `crates/ui/web-api/src/notifications/dispatcher.rs:47-58`
- **Description**: The notification queue is bounded (4096), which is correct for memory safety.
  When the channel fills, events are silently dropped with a warning log. No backlog, replay, or
  explicit reconciliation exists.
- **Why it matters**: Bursty update completions or a slow downstream notification path (e.g., SMTP
  timeout) cause some user-visible notifications never to be delivered.
- **Failure scenario**: A batch update completes for 100 hosts simultaneously. The dispatch loop is
  blocked on a slow Telegram API call for a previous event. The channel fills and some
  `UpdateCompleted` notification events are dropped.

### [MEDIUM] `dispatch_loop` spawns unmonitored delivery tasks

- **Dimension**: fault tolerance, observability
- **Scope**: `crates/ui/web-api/src/notifications/dispatcher.rs:255`
- **Description**: Each notification delivery is `tokio::spawn`-ed as a fire-and-forget task.
  If a delivery task panics (e.g., serde failure, unexpected plugin error), the notification log
  entry remains stuck in `pending` status forever with no cleanup or retry mechanism.
- **Why it matters**: Orphaned `pending` entries accumulate in the notification log table and may
  confuse operators looking at delivery status dashboards.
- **Failure scenario**: A notification plugin panics during `deliver()`. The spawned task exits
  without updating the log entry to `failed`. The entry stays `pending` indefinitely.
- **Fix**: Use `tokio::task::JoinSet` or a similar supervisor pattern. On task completion, check
  the `JoinError` and update any orphaned log entries to `failed`.

### [MEDIUM] The crate still exports a broad internal module surface publicly

- **Dimension**: architecture, maintainability
- **Scope**: `crates/ui/web-api/src/lib.rs`
- **Description**: Many modules that read like internal implementation details are still `pub mod`
  (e.g., `batch_progress_broadcaster`, `device_flow_broadcaster`, `update_output_broadcaster`,
  `notifications`, `ocsp`, `pki_utils`, `config_test_proxy`, `extension_proxy`,
  `extension_registry`, `event_delivery`). Only `app_state`, `router`, `auth`, `settings`,
  `queries`, and a few types should be public API.
- **Why it matters**: Refactoring route, middleware, or broadcaster internals is constrained
  because external code may already depend on implementation modules that should be private.
- **Failure scenario**: A future refactor of the broadcaster system requires changing internal
  module structure, but the controller crate has already imported internal types.

### [MEDIUM] `plugin_configs.rs` bundles five distinct concerns in one 1385-line file

- **Dimension**: code quality, maintainability
- **Scope**: `crates/ui/web-api/src/routes/plugin_configs.rs` (1385 lines)
- **Description**: HTTP endpoint handlers, dangerous-command detection, command-field traversal,
  audit-log helpers, plugin-config test dispatch, and the test suite are all interleaved in one
  file.
- **Why it matters**: Navigating and changing any single concern requires reasoning across the
  entire 1385-line file.
- **Fix**: Extract `dangerous_patterns.rs` (pattern detection + field traversal) and `audit.rs`
  (audit-log helpers) as sub-modules; move tests to a separate `tests/` integration test file.

### [MEDIUM] HTTP + security policy logic is merged inside route handlers

- **Dimension**: code quality, testability
- **Scope**: `crates/ui/web-api/src/routes/plugin_configs.rs:create_plugin_config`,
  `update_plugin_config`
- **Description**: Dangerous command checking and audit logging are embedded in the HTTP handler
  body, making it impossible to test security policy without a full HTTP stack. The same policy
  cannot easily be reused from CLI or batch paths.
- **Why it matters**: Security-critical logic should be independently testable and reusable.
- **Fix**: Extract a `PluginConfigSecurityPolicy` service that validates config against dangerous
  patterns and can be called from any path without HTTP context.

### [MEDIUM] `error_response` called 477 times with repetitive boilerplate across 39 route files

- **Dimension**: code quality, maintainability
- **Scope**: `crates/ui/web-api/src/routes/` (all route files)
- **Description**: `not-found`, `db-error`, and `validation-error` response shapes are constructed
  inline in every route handler with nearly identical code. The count has grown from 385 to 477
  since the previous review.
- **Why it matters**: Changing the error response format (e.g., adding request correlation IDs)
  requires touching 39 files.
- **Fix**: Introduce an `error_builders` module with `not_found_response(resource)`,
  `db_error_response(error)`, and `handle_query_error(report)` helpers.

### [MEDIUM] Four `start_paused` rule violations in service WebSocket integration tests

- **Dimension**: test correctness
- **Scope**: `crates/ui/web-api/src/integration_tests/service_ws.rs` (four test functions:
  `anonymous_connect_and_enroll` line 21, `enrolled_reconnect_with_bearer` line 58,
  `service_connection_registry_send` line 141,
  `service_connection_registry_broadcast` line 171)
- **Description**: All four functions call `tokio::time::timeout()` without
  `#[tokio::test(start_paused = true)]`, violating the project rule that tests using any tokio
  time API must declare `start_paused = true`.
- **Why it matters**: Without `start_paused`, these tests depend on wall-clock time and may flake
  on loaded CI runners.
- **Failure scenario**: A CI runner under high CPU load cannot complete the test within the
  5-second `timeout()` window, causing a spurious test failure.
- **Fix**: Add `start_paused = true` to all four test attributes.

### [MEDIUM] Notification route handlers bypass `Validated<>` extractor

- **Dimension**: coding standards, consistency, input validation
- **Scope**: `crates/ui/web-api/src/routes/notifications.rs:63` (`CreateNotificationChannelRequest`),
  `:179` (`UpdateNotificationChannelRequest`), `:367` (`CreateNotificationRuleRequest`),
  `:494` (`UpdateNotificationRuleRequest`)
- **Description**: All four notification request types use raw `Json<T>` instead of `Validated<T>`.
  The `Validated` extractor pattern is established in 16 other route handlers across the crate
  (plugin configs, auth, scheduler, settings, software items, etc.) but is missing here.
- **Why it matters**: If these request types implement `Validate`, the validation is silently
  skipped. If they do not yet implement it, they should, to enforce schema constraints before
  the data reaches the query layer.
- **Failure scenario**: A client sends a notification channel creation request with an empty
  `channel_type` or a rule with an invalid `event_type`. Without validation, the invalid data
  reaches the database or causes a confusing error downstream.

### [LOW] `parse_capability_str` is a hand-maintained string-to-enum mapping

- **Dimension**: maintainability, consistency
- **Scope**: `crates/ui/web-api/src/event_delivery.rs:45-62`
- **Description**: `parse_capability_str` maps 13 string literals to `Capability` enum variants.
  This mapping must be kept in sync manually with the `Capability` enum definition in the wire
  crate. There is a test (`parse_capability_str_known_values`) that covers all current variants,
  but if a new variant is added to the enum without updating this function, the test will not
  catch the gap (it only tests known values, not exhaustiveness).
- **Why it matters**: A new capability variant added to the wire crate would silently fall through
  to `None`, causing `broadcast_by_capability` to never match for that capability.
- **Failure scenario**: A developer adds a new `Capability::Monitoring` variant. Cross-controller
  events targeted at `"monitoring"` capability are broadcast to all services instead of only
  monitoring-capable ones.
- **Fix**: Add a `KNOWN_VARIANTS` const array in the `Capability` enum (following the project
  pattern for `Other(String)` enums) and a test that asserts every variant in the array is handled
  by `parse_capability_str`.
