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
