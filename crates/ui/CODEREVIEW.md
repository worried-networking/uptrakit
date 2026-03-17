# Code Review: `crates/ui` Aggregate

- Review date: 2026-03-17
- Scope: full 14-dimension review of Rust UI/backend interface crates (`web-api`, `web-api-auth`)

## Summary

The UI-layer Rust crates compile, lint, and test cleanly. Their strongest qualities are typed
boundaries, consistent auth/query separation, and thorough test coverage. This review cycle
revalidated all prior findings against current code. The `unreachable!()` panics in the WebSocket
handler remain the highest-priority production risk. The `start_paused` test violations,
notification dispatcher limitations, and code quality issues are all confirmed active. One prior
finding was downgraded (CC complexity in `deliver_controller_event` is lower than previously
claimed). A new finding was added for inconsistent `Validated<>` usage in notification routes.

## Strengths

- `web-api-auth`, `web-api-queries`, and `web-api` divide responsibilities cleanly. Auth logic,
  DB queries, and HTTP routing are in separate crates with well-defined boundaries.
- The CLI continues to reuse the typed client and shared DTOs rather than drifting into a parallel
  protocol.
- The non-integration test sweep across these crates is strong: auth, JWT, rate limiting, token
  denylist, OIDC flows, security headers, middleware, and route handlers all have dedicated tests.
- Permission extractors are consistently applied; no bypass paths were found in this review.
- Coding standards compliance is verified clean: `#[non_exhaustive]`, `Other(String)` catch-all,
  and `parking_lot::Mutex` patterns are all correct in production code paths.
- No `unwrap()`, `panic!()`, or `todo!()` calls exist in production code paths. All such calls
  are confined to `#[cfg(test)]` modules.
- Security posture is strong: HSTS, CSP, rate limiting on auth endpoints, JWT audience/issuer
  enforcement, OIDC email verification, constant-time password comparison, and CSRF state tokens
  are all implemented correctly.
- Rate limiting has a fail-closed local fallback (in-memory counter) when the DB store is
  unavailable, ensuring auth endpoints remain protected even during DB outages.

## Active Findings

### [HIGH] `unreachable!()` panics in production WebSocket capability dispatch

- **Dimension**: fault tolerance, maintainability
- **Scope**: `crates/ui/web-api/src/routes/service_ws/handler/mod.rs:420`, `:444`, `:465`
- **Description**: Three `unreachable!()` macros guard capability-to-message dispatch in inner
  helper functions. A missed update when adding a new message variant terminates enrolled agent
  sessions with an unrecoverable panic. The outer dispatch function already has a correct
  catch-all, but the inner functions do not.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`

### [HIGH] Update-state recovery is still reconnect-scoped instead of system-scoped

- **Dimension**: high availability, database
- **Scope**: `crates/ui/web-api-queries/src/queries/update_batches/dispatch.rs`,
  `crates/ui/web-api/src/routes/service_ws/handler/updates.rs`
- **Description**: The web/API layer can clean up after an agent reconnects, but not after the
  wider class of failures where no reconnect happens. Agent, controller, DB, or network failure
  can strand an `InProgress` row indefinitely and block the host from future updates.
- **Details**: `crates/ui/web-api-queries/CODEREVIEW.md`

### [MEDIUM] Notification dispatcher drops events under backpressure with no recovery

- **Dimension**: high availability, consistency
- **Scope**: `crates/ui/web-api/src/notifications/dispatcher.rs`
- **Description**: Bounded queues are correct for memory safety, but the current model is
  fire-and-forget: events are silently dropped when the channel fills. Additionally, spawned
  delivery tasks are unmonitored -- a panicking task leaves a notification log entry stuck in
  `pending` status.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] `web-api` still exposes too much internal surface publicly

- **Dimension**: architecture, maintainability
- **Scope**: `crates/ui/web-api/src/lib.rs`
- **Description**: A wide `pub mod` surface makes refactors harder and blurs the intended public
  API of the crate. Many modules (broadcasters, notifications, ocsp, pki_utils, etc.) should be
  `pub(crate)`.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] Validation inconsistency between `Validated<>` and `Json<>` across route handlers

- **Dimension**: coding standards, consistency, input validation
- **Scope**: `crates/ui/web-api/src/routes/notifications.rs` (4 handlers),
  `crates/ui/web-api/src/routes/services.rs:129` (`UpdateServiceRequest`),
  `crates/ui/web-api/src/routes/hosts.rs:122` (`UpdateHostRequest`),
  `crates/ui/web-api/src/routes/users.rs:313` (`UpdateUserRolesRequest`),
  `crates/ui/web-api/src/routes/plugin_configs.rs:268` (`UpdatePluginConfigRequest`),
  `crates/ui/web-api/src/routes/software_items/mod.rs:229` (`UpdateSoftwareItemRequest`),
  and several other update/create handlers across settings, enrollment tokens, discovery
  allowlist, host tags, system services, OIDC providers
- **Description**: 16 route handlers use the `Validated<T>` extractor pattern, but approximately
  22 handlers across notifications, services, hosts, users, settings, and other domains use raw
  `Json<T>` for their request types. This inconsistency means some request types bypass schema
  validation at the HTTP layer.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] Four `start_paused` rule violations in service WebSocket integration tests

- **Dimension**: test correctness
- **Scope**: `crates/ui/web-api/src/integration_tests/service_ws.rs`
- **Description**: Tests using `tokio::time::timeout()` without `start_paused = true` are non-
  deterministic and may flake on loaded CI runners.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] OIDC state store conflates expired and non-existent tokens

- **Dimension**: security, user experience
- **Scope**: `crates/ui/web-api-auth/src/auth/oidc_state.rs:OidcFlowStore::take`
- **Description**: `take()` returns `None` for both an expired CSRF state token and a token that
  was never issued, preventing callers from providing specific error messages.
- **Details**: `crates/ui/web-api-auth/CODEREVIEW.md`

### [MEDIUM] `plugin_configs.rs` bundles five distinct concerns in one 1385-line file

- **Dimension**: code quality, maintainability
- **Scope**: `crates/ui/web-api/src/routes/plugin_configs.rs`
- **Description**: HTTP handlers, security policy, audit-log helpers, config test dispatch, and
  the test suite are all interleaved in one large file.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`

### [MEDIUM] `error_response` boilerplate across 39 files (477 calls)

- **Dimension**: code quality, maintainability
- **Scope**: `crates/ui/web-api/src/routes/` (all route files)
- **Description**: Nearly identical error response construction is repeated across every route
  handler. Changing the error format requires touching 39 files.
- **Details**: `crates/ui/web-api/CODEREVIEW.md`
