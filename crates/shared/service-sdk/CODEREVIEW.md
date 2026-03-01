# Code Review: uptrakit-service-sdk

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-service-sdk` is the shared lifecycle library used by every agent and MQTT service binary.
It provides enrollment, mTLS reconnection, certificate renewal, CA bundle management, exponential
backoff, TLS configuration, and OS signal handling under a single `run_service_lifecycle` entry
point. All three service binaries (`uptrakit-agent`, `uptrakit-agent-ssh`, `uptrakit-mqtt`) delegate
to this library, meaning any lifecycle fix propagates automatically to all consumers.

The design is sound and the code quality is high. The main concerns are that `ServiceHandler` is
not object-safe (undocumented), the `run_service_lifecycle` core function has zero test coverage,
and the `poll_service_event` cancellation-safety requirement is undocumented. The
`BasicConstraints::Unconstrained` divergence in the test CA helper has been fixed
(`Constrained(0)` now matches production). The enrollment retry loop previously only retried on
`ReceiveClosed`; it now retries all transient network errors (`WebSocket`, `Io` non-cert-expired,
`ConnectionTimeout`) via `EnrollmentError::is_transient_network()`.

## Architecture

### Strengths

- `src/lifecycle.rs` -- `run_service_lifecycle<H: ServiceHandler>` is the single generic entry
  point. All service-specific behavior is injected through trait methods (`on_connected`,
  `on_message`, `on_settings`, `on_service_event`, `on_shutdown`, `poll_service_event`,
  `capabilities`). Every lifecycle fix is inherited by all consumers without duplication.
- `src/cert_handler.rs` -- `CertificateRenewalHandler` handles `RequestCertRenewal`, timer-based
  renewal, `CaBundleUpdated`, and `Certificate` responses. Any service that implements
  `ServiceHandler` receives all of this automatically.
- `src/lifecycle.rs` -- `LoopOutcome` enum (`Reconnect`, `Disconnected`, `Shutdown`, `Restart`)
  makes lifecycle decisions explicit. No boolean flags or string comparisons.
- `src/lifecycle.rs` -- `LoopError` semantic variants (`CertExpired`, `ReceiveClosed`, `Other`)
  prevent error-type leakage.
- `src/tls.rs` -- TOFU implementation correctly layers: skips chain verification but delegates
  TLS 1.2/1.3 signature verification. Comment makes the security invariant explicit.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/backoff.rs:29-43` -- Exponential backoff with jitter prevents thundering herd. Base 2
  seconds, cap 60 seconds, ~25% uniform jitter. On broker/controller restart, services reconnect
  over a spread window.
- `src/event_loop.rs:260-278` -- `dispatch_close_reason` correctly distinguishes cert-rotation
  (immediate reconnect, no backoff) from revocation (reconnect with backoff). Tested by
  dedicated unit tests.
- `src/lifecycle.rs:343-356` -- mTLS connector rebuilt on every reconnect iteration, ensuring
  freshly-written certificate from rotation is picked up.
- `src/cert_handler.rs:92` -- Private key for in-flight renewal held exclusively in
  `CertificateRenewalHandler` via `pending_renewal_key: Option<String>`. Consumed via `take()`,
  preventing it from persisting beyond the renewal handshake.
- Zero `unsafe` in production code.

### Issues

## Code Quality

### Strengths

- Every source file opens with a module doc comment. `cert_handler.rs` documents each public
  function and the invariant that `CertificateRenewalHandler` must be created per connection.
- Named constants throughout: `CERT_RECONNECT_DELAY`, `FAR_FUTURE`,
  `DEFAULT_SHUTDOWN_TIMEOUT`.
- `src/event_loop.rs:212-218` -- `LoopState` struct groups mutable references for
  `handle_service_settings`, keeping the function signature readable.
- Consistent `rootcause` error propagation: `context_to()`, `bail!`, `report!`. No
  `Report::new()` anti-pattern.
- `src/event_loop.rs:94` -- `biased` on `tokio::select!` ensures explicit priority ordering.
  Documented in adjacent comments.

### Issues

**[MEDIUM]** `src/lifecycle.rs` (entire file) -- `run_service_lifecycle` core function has zero
unit test coverage. This is the central bootstrap-enrollment-reconnect loop. The
`ServiceHandler` trait is designed for mockability but no test exercises the lifecycle state
machine.

**[LOW]** `src/signal.rs:104-108` -- `signal_watcher_new` test asserts only `is_ok()` without
exercising signal delivery. Does not test signal dispatch path.

## High Availability

### Strengths

- `src/backoff.rs:29-43` -- Exponential backoff with jitter. Both the authenticated reconnect
  loop and enrollment retry loop use dedicated `Backoff` instances that do not interfere.
- `src/lifecycle.rs:381-386` -- Zero-delay reconnect on cert rotation.
  `reconnect_backoff.reset()` before the fixed 2-second `CERT_RECONNECT_DELAY`. Cert rotation
  does not accumulate backoff from prior disconnection events.
- `src/lifecycle.rs:217-249` -- Certificate expiry detected before attempting mTLS. If on-disk
  certificate is expired, clears enrollment state and falls through to fresh enrollment.
- mTLS TLS connector rebuilt each reconnect iteration from `ServiceIdentityState`.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- `Cargo.toml` -- `edition = "2024"` and workspace version inheritance. All major dependencies
  workspace-pinned.
- Zero `#[allow(clippy::...)]` suppressions.
- `async_trait` used consistently. `ServiceHandler` methods annotated with `#[async_trait]`.
- `thiserror`-derived errors with semantic variants. `LoopError` has three meaningful variants.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `ServiceHandler` externalizes the entire service-specific surface. Adding a new service role
  requires only implementing the trait. `capabilities()` returns `BTreeSet<Capability>`,
  replacing the former `SERVICE_TYPE` constant.
- `type ServiceEvent: Send` associated type, combined with `poll_service_event` and
  `on_service_event`, lets each service add its own `tokio::select!` arm. `Infallible` type
  recommended for services with no custom events.
- `LoopOutcome::Restart` supports SIGHUP-based restarts.
- `LoopError::Other(String)` forward compatibility.

### Issues

**[MEDIUM]** `src/lifecycle.rs:79,89` -- `ServiceHandler` is not object-safe due to associated
constants (`DIR_NAME`, `SERVICE_LABEL`). No `where Self: Sized` guard, no comment, no
documentation. A future implementor attempting `Box<dyn ServiceHandler>` will receive a cryptic
compiler error. Add a `// Note: not object-safe due to associated constants` comment, or
convert to `fn` methods returning `&'static str`.

**[LOW]** `src/lifecycle.rs:76` -- `ServiceHandler` requires `Send` but not `Sync`, inconsistent
with `Plugin` trait. Current constraint is correct for single-owner pattern but warrants a
design-note comment.

**[LOW]** `src/lifecycle.rs:142` -- `poll_service_event` cancellation safety requirement is
undocumented. This future is used as an arm in `tokio::select!`. When another arm fires, Tokio
drops the future mid-poll. The trait documentation should state the cancellation-safety
requirement explicitly.

## Tests

### Strengths

- `src/cert_handler.rs:251-448` -- 14 tests cover the certificate renewal handler: state
  transitions (idle, pending, handling response), the cert-expiry check (`is_expired`),
  `should_renew` threshold logic, and timer-based renewal scheduling. Three tests correctly
  use `#[tokio::test(start_paused = true)]` with `tokio::time::advance` for the timer
  tests; the remaining async tests do not use time APIs and correctly omit `start_paused`.
- `src/error.rs:188-280` -- 13 synchronous tests exercise every `LoopError` and
  `EnrollmentError` variant, the `is_transient_network()` predicate (all transient and
  non-transient paths), and `Display` output for each variant.
- `src/event_loop.rs` -- `dispatch_close_reason` is tested for cert-rotation (immediate
  reconnect path) and revocation (backoff reconnect path), verifying that the
  `LoopOutcome` variant returned is correct for each `CloseReason`.
- `src/signal.rs:91-103` -- Two synchronous tests verify `SignalWatcher` construction and
  that `signal_watcher_new` returns `Ok` on the test platform.

### Issues

**[HIGH]** `src/lifecycle.rs` -- `run_service_lifecycle` has zero test coverage. This is
the entire enrollment-reconnect-shutdown state machine. The `ServiceHandler` trait is
intentionally mockable (all methods have defaults or clear signatures), but no test
exercises the full lifecycle transitions: enrollment success, enrollment failure with retry,
reconnect after `CertExpired`, reconnect after `Disconnected`, or graceful `Shutdown`.

**[LOW]** `src/signal.rs:104-108` -- `signal_watcher_new` test asserts only `is_ok()`
without delivering a signal. The signal dispatch path (e.g., `recv().await` returning after
a simulated SIGTERM) is untested. While sending OS signals from tests is non-trivial,
verifying that the watcher properly closes the channel on signal would increase confidence
in the shutdown path.

## Consistency

### Strengths

- `src/lifecycle.rs:295-314` (enrollment retry) and `src/lifecycle.rs:380-437`
  (reconnect loop) -- Both loops construct a fresh `Backoff::new(2s, 60s)` and call
  `backoff.next_delay()` before `tokio::time::sleep`. Neither loop shares a backoff
  instance between phases, so a long enrollment retry does not accumulate into the
  reconnect backoff and vice versa. The two retry loops are structurally symmetric.
- `src/event_loop.rs:94` -- The `tokio::select!` loop uses `biased` with a documented
  priority ordering. Service-specific events are arm 1 (highest priority), then ping, then
  renewal, then controller messages, then OS signals. This matches the design intent stated
  in the module doc-comment and is applied consistently for a single loop, not split across
  multiple loops.
- `src/connection.rs:100-182` -- `recv()` applies the same validation pipeline (header
  parse → protocol version check → sequence check → full envelope deserialize) identically
  for both `Message::Text` and `Message::Binary` frames. Neither branch skips a step.

### Issues

**[MEDIUM]** `src/lifecycle.rs:297-314` (enrollment backoff) vs `src/lifecycle.rs:423-429`
(reconnect backoff on `Disconnected`) -- In the reconnect loop, `LoopOutcome::Reconnect`
(certificate rotated) resets the backoff with `reconnect_backoff.reset()` before a fixed
2-second delay, while `LoopOutcome::Disconnected` advances the backoff with
`reconnect_backoff.next_delay()`. These two outcomes are explicitly differentiated. In the
enrollment retry loop, all transient errors advance the backoff with
`enrollment_backoff.next_delay()`, including `ReceiveClosed` which is semantically the same
as a clean `Disconnected` event. A cert-rotation–driven enrollment disconnect would
accumulate backoff in the enrollment phase that it would not accumulate in the reconnect
phase. Adding a `ReceiveClosed`-specific reset in the enrollment loop would mirror the
reconnect behavior.

**[LOW]** `src/lifecycle.rs:263-285` (early cert-expiry path) vs `src/lifecycle.rs:274-284`
(run-authenticated-with-reconnect error path) -- Both paths detect cert expiry, log a
warning, and call `identity.clear_enrollment_state()`. The log messages differ slightly:
`"certificate expired, falling back to fresh enrollment"` vs `"certificate expired, falling
back to enrollment"`. These two log messages describe the same event but use slightly
different wording, making log search by pattern inconsistent. Both should use the same
canonical message.

**[LOW]** `src/connection.rs:73-83` (`send`) -- `send` propagates errors via `?` with
`context_to::<EnrollmentError>()`, making the caller responsible for handling send
failures. `send_best_effort` on line 212 wraps `send` and logs at `warn!` on failure.
The agent-core `client.rs` uses `send_best_effort` for update output and `send` for
`VersionCheckResults` and `DiscoveryResults`. The choice between the two is not documented
by a convention: critical responses use `send` (error propagates), status messages use
`send_best_effort` (error absorbed). This distinction is implicit and has no trait-level
documentation in `ServiceHandler`.

## Maintainability

### Strengths

- `src/lifecycle.rs` -- `ServiceHandler` trait methods are all doc-commented, including the
  table in `ShutdownCause` mapping cause to `DisconnectReason` and `LoopOutcome`. A new
  implementor has a complete reference in one place.
- `src/cert_handler.rs` -- Module doc comment lists every public export and explains the
  `CertificateRenewalHandler` per-connection invariant. Adding a new certificate lifecycle
  event is a one-method addition to `CertificateRenewalHandler`.
- Named constants throughout (`FAR_FUTURE`, `CERT_RECONNECT_DELAY`, `DEFAULT_SHUTDOWN_TIMEOUT`)
  with doc comments. No magic numbers in the lifecycle logic.
- `src/event_loop.rs:212-218` -- `LoopState` struct groups mutable references that would
  otherwise form a long parameter list. The struct makes the function signature `handle_service_settings(state, settings)` instead of a 5+ parameter list.

### Issues

**[HIGH]** `src/lifecycle.rs:481` (entire function) -- `run_service_lifecycle` is the central
function of the crate with zero unit tests. The trait is designed for testability (all
service-specific behavior is injected through `ServiceHandler`), yet no test exercises the
enrollment → reconnect → shutdown state machine. A future refactor of the lifecycle transitions
— adding a new `LoopOutcome` variant, changing backoff behavior, modifying enrollment retry
conditions — will have no regression safety net. Adding even a minimal mock `ServiceHandler`
that exercises the happy path (fresh enrollment, authenticated connection, graceful shutdown)
would catch most lifecycle regressions.

**[MEDIUM]** `src/identity.rs` -- `ServiceIdentityState` at 866 lines mixes three distinct
concerns: file I/O for identity state (load/save), cryptographic operations (keypair generation,
CSR generation), and state interrogation accessors (`is_fresh`, `is_enrolled_only`, `is_certified`,
`cert_not_after`, etc.). These are independent operations that could be organized into logical
sections or sub-modules. As written, a maintainer working on CSR generation must navigate past
15+ accessor methods to find the crypto operations.

**[LOW]** `src/lifecycle.rs:79,89` -- `ServiceHandler` has two associated constants
(`DIR_NAME: &'static str`, `SERVICE_LABEL: &'static str`) that make it non-object-safe. The
existing Extensibility review notes this; it is also a maintainability issue because any future
attempt to store a `Box<dyn ServiceHandler>` (e.g., for dynamic service loading) will receive a
cryptic compiler error referencing object safety rather than a clear note in the trait doc. A
comment on the trait explaining the non-object-safety and the reason for the design would
prevent future confusion.

**[LOW]** `src/event_loop.rs:62` -- `DEFAULT_SHUTDOWN_TIMEOUT: u32 = 120` is defined as a
local constant inside `run_event_loop` rather than in a `durations` module. This makes it
invisible to operators looking for tunable timeouts. The constant is sent to the controller as
the initial `ServiceSettings` fallback. Moving it to `src/lib.rs` or a `durations.rs` with a
doc comment would make it discoverable.

**[LOW]** `src/ca.rs` -- The CA bundle management module has no module-level doc comment. The
module handles CA certificate loading, validation, and trust-chain building — operations that
interact with the TLS and TOFU subsystems. A brief doc comment explaining the module's role in
the identity lifecycle would aid navigation.
