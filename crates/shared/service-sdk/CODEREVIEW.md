# CODEREVIEW — uptrakit-service-sdk

Reviewed: 2026-02-23
Reviewer: senior Rust engineer (automated phase-2 pass)
Scope: `crates/shared/service-sdk/` — all source files and `Cargo.toml`

---

## Summary

`uptrakit-service-sdk` is the shared lifecycle library used by every agent and
MQTT service binary. It provides enrollment, mTLS reconnection, certificate
renewal, CA bundle management, exponential backoff, TLS configuration, and OS
signal handling under a single `run_service_lifecycle` entry point. All three
service binaries (`uptrakit-agent`, `uptrakit-agent-ssh`, `uptrakit-mqtt`)
delegate to this library, meaning any lifecycle fix propagates automatically
to all consumers.

The design is sound and the code quality is high. There are two actionable
issues of Medium severity — a misplaced production dependency on
`tracing-subscriber` and an `.await` inside a settings handler that briefly
suspends the event loop — plus a Low-severity enrollment retry gap and one
undocumented trait constraint. No Critical or High findings apply specifically
to this crate.

---

## Architecture

### Strengths

**Clean `ServiceHandler` trait + `run_service_lifecycle` driver.**
`lifecycle.rs` defines a single generic entry point
`run_service_lifecycle<H: ServiceHandler>`. All service-specific behaviour is
injected through eight trait methods (`on_connected`, `on_message`,
`on_settings`, `on_service_event`, `on_shutdown`, `poll_service_event`,
`capabilities`). This means every lifecycle correctness fix — expiry fallback,
cert-rotation reconnect, backoff reset — is inherited by all three consumers
without duplication. Binaries stay at roughly 200 LoC, delegating all plumbing
to the SDK.

**Enrollment, mTLS reconnect, and cert renewal centralised in one crate.**
The full bootstrap sequence (directory setup → identity load → CA bootstrap →
enrollment loop → authenticated reconnect loop) lives in one place. The
`CertificateRenewalHandler` in `cert_handler.rs` handles `RequestCertRenewal`,
timer-based renewal, `CaBundleUpdated`, and `Certificate` responses. Any
service that implements `ServiceHandler` receives all of this automatically.

**`LoopOutcome` enum makes lifecycle decisions explicit.**
`Reconnect`, `Disconnected`, `Shutdown`, and `Restart` are distinct typed
values returned from every decision point. There is no boolean flag or string
comparison driving reconnect logic.

**`LoopError` semantic variants prevent error-type leakage.**
`CertExpired`, `ReceiveClosed`, and `Other` let callbacks express meaning
without requiring services to construct SDK-internal error types. The
`impl_report_conversion!` macro then maps `EnrollmentError` variants into the
correct `LoopError` variant at the lifecycle boundary.

**TOFU implementation is correctly layered.**
`build_tofu_client_config` in `tls.rs` uses a custom `ServerCertVerifier` that
intentionally skips chain verification but still delegates TLS 1.2 and 1.3
signature verification to the installed crypto plugin. The comment makes the
security invariant explicit: "security relies on fingerprint verification after
download."

### Issues

---

## Security & Safety

### Strengths

**Exponential backoff with jitter prevents thundering herd.**
`backoff.rs:29-43` implements `next_delay()` as `current + jitter` where
jitter is uniform-random in `[0, current/4]`. The cap is 60 seconds (base 2
seconds). On a broker restart or controller restart, all connected services
will reconnect over a spread window rather than simultaneously. The
implementation is correct: `current` doubles before jitter is added, `reset()`
returns to the base delay, and the zero-base edge case does not panic.

**`dispatch_close_reason` correctly distinguishes cert-rotation from
revocation.**
`event_loop.rs:260-278` maps `CloseReason::CertificateRotated` to
`LoopOutcome::Reconnect` (immediate reconnect, no backoff, cert on disk is
valid) and `CloseReason::CertificateRevoked` to `LoopOutcome::Disconnected`
(reconnect with backoff, avoids hammering the controller after a deliberate
revocation). This distinction is semantically important and is tested by
dedicated unit tests at `event_loop.rs:281-313`.

**mTLS connector rebuilt on every reconnect iteration.**
`lifecycle.rs:343-356` in `run_authenticated_with_reconnect` unconditionally
rebuilds the `TlsConnector` at the top of each loop iteration, ensuring the
freshly-written certificate and key from a rotation are picked up without any
in-memory caching inconsistency.

**Zero `unsafe` in production code.**
Confirmed across all source files in this crate.

**Private key for in-flight renewal held exclusively in `CertificateRenewalHandler`.**
`cert_handler.rs:92` stores `pending_renewal_key: Option<String>`. The key is
consumed (via `take()`) in `handle_certificate`, preventing it from persisting
beyond the renewal handshake.

### Issues

**[SEVERITY: Medium]** `tls.rs:234` — `BasicConstraints::Unconstrained` used
in the test CA helper.

The `generate_test_ca()` helper at `tls.rs:227-237` sets:

```rust
params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
```

`Unconstrained` means the issued certificate carries no path-length constraint,
which permits any holder to issue further intermediate CA certificates. The
correct value for a leaf-signing CA is `BasicConstraints::Constrained(0)`
(path length 0). Because this is test-only code the immediate risk is confined
to tests, but this pattern is mirrored in the production `pki.rs:497` and
`cert_signer.rs:174` (noted as Medium severity in the workspace-wide security
review). Aligning the test helper with the recommended `Constrained(0)` value
removes the divergence and prevents the test CA from being used as an
intermediate issuer if test artefacts are ever leaked or reused.

---

## Code Quality

### Strengths

**Module-level documentation is complete and accurate.**
Every source file opens with a module doc comment that correctly describes what
the module contains and how its public API should be used. `cert_handler.rs`
documents each public function and the invariant that `CertificateRenewalHandler`
must be created per connection.

**Named constants, no magic numbers.**
`CERT_RECONNECT_DELAY` (`lifecycle.rs:329`), `FAR_FUTURE` (`cert_handler.rs:33`),
and `DEFAULT_SHUTDOWN_TIMEOUT` (`event_loop.rs:62`) give every significant
numeric literal a name and a doc comment explaining its purpose.

**`LoopState` struct groups mutable references for `handle_service_settings`.**
Rather than passing seven individual `&mut` parameters, `event_loop.rs:212-218`
defines a short-lived `LoopState<'a>` struct. This keeps the function signature
readable and avoids lifetime complexity in the call site.

**Consistent `rootcause` error propagation.**
`context_to::<LoopError>()`, `bail!`, and `report!` are used throughout. There
is no `Report::new()` anti-pattern and no `Result<T, String>`.

**`biased` on the `tokio::select!` in `event_loop.rs:94`.**
The `biased` keyword ensures service events are polled before the ping timer,
which before controller messages, which before signals. This explicit priority
ordering prevents starvation of higher-priority arms under load and is
correctly documented in the adjacent comments.

#### 2026-02-24 Review

#### Issues

**[SEVERITY: Low]** `src/event_loop.rs:105` — `expect()` on `ping_timer` inside `tokio::select!` arm is not an approved exception

Guarded by `is_some()` check, so will never fire. Refactor to pattern match to eliminate the `expect()`.

### Issues

---

## Tests

### Strengths

**`start_paused = true` used correctly for all time-dependent tests.**
`cert_handler.rs:385`, `cert_handler.rs:397`, and `cert_handler.rs:411` each
use `#[tokio::test(start_paused = true)]` and drive time forward with
`tokio::time::advance`. This makes the renewal-timer tests deterministic and
fast regardless of CI load.

**`CertificateRenewalHandler` is thoroughly unit-tested.**
`cert_handler.rs` tests cover: no pending key, happy path with real cert,
`initiate_renewal` when not enrolled, `handle_ca_bundle_updated` persistence,
and all `compute_renewal_delay` boundary cases (no cert, future cert, cert
already in renewal window, expired cert, zero window).

**`dispatch_close_reason` has exhaustive unit tests.**
`event_loop.rs:281-313` tests `CertificateRotated`, `CertificateRevoked`,
`Unknown`, and `None`, confirming each maps to the correct `LoopOutcome`. These
are pure synchronous unit tests with no I/O.

**`Backoff` has property-based unit tests.**
`backoff.rs:51-105` covers doubling behaviour, cap at max, reset, and the
zero-base edge case. Tests verify both the lower bound (no jitter) and upper
bound (base + 25% jitter) of each delay range.

**TLS builder tests are self-contained.**
`tls.rs:217-406` uses `rcgen` to generate real in-process test CAs and client
certificates. No file I/O or network calls are needed, and
`install_crypto_provider()` is called explicitly in each test that requires it,
making the tests safe to run in parallel.

#### 2026-02-24 Review

#### Issues

**[SEVERITY: Medium]** `src/lifecycle.rs` (entire file) — `run_service_lifecycle` core function has zero unit test coverage

The central bootstrap-enrollment-reconnect loop has no tests. The `ServiceHandler` trait is designed for mockability.

**[SEVERITY: Medium]** `src/cert_handler.rs:293,312,324,345,436` — Five `CertificateRenewalHandler` tests use bare `#[tokio::test]`

Same file has correctly-annotated tests at lines 385, 397, 411 — inconsistency within one file.

**[SEVERITY: Low]** `src/signal.rs:104-108` — `signal_watcher_new` test asserts only `is_ok()` without exercising signal delivery

Does not test signal dispatch path.

---

## High Availability

### Strengths

**Exponential backoff with jitter in `backoff.rs:29-43`.**
Base 2 seconds, cap 60 seconds, ~25% uniform jitter per call. Both the
authenticated reconnect loop (`lifecycle.rs:341`) and the enrollment retry loop
(`lifecycle.rs:259`) use dedicated `Backoff` instances, so the two loops do not
interfere with each other's backoff state.

**Zero-delay reconnect on cert rotation (`LoopOutcome::Reconnect`).**
`lifecycle.rs:381-386` calls `reconnect_backoff.reset()` before the fixed
2-second `CERT_RECONNECT_DELAY`. This means a cert rotation — which is an
expected, coordinated operation — does not accumulate backoff from any prior
disconnection events. Services return to the controller as quickly as possible
after rotation.

**Certificate expiry detected before attempting mTLS.**
`lifecycle.rs:217-249` checks `cert_not_after_ms()` against `now_millis()`
before entering the authenticated loop. If the on-disk certificate is already
expired, it clears enrollment state and falls through to fresh enrollment
rather than attempting an mTLS handshake that would be rejected.

**mTLS TLS connector rebuilt each reconnect iteration.**
The `ca_pem`, `cert_pem`, and `key_pem` are re-read from `ServiceIdentityState`
at the top of each iteration of `run_authenticated_with_reconnect`. This
ensures a certificate written during a previous rotation cycle is always loaded
without requiring any cache invalidation mechanism.

### Issues

**[SEVERITY: Low]** `lifecycle.rs:263-275` — Enrollment retry only catches
`ReceiveClosed`, not transient network errors.

```rust
Err(e) => {
    if is_receive_closed_report(&e) {
        let delay = enrollment_backoff.next_delay();
        tracing::info!("disconnected during enrollment, reconnecting in {delay:?}");
        tokio::time::sleep(delay).await;
        identity.load().await?;
        continue;
    }
    return Err(e);
}
```

Only `LoopError::ReceiveClosed` (clean WebSocket close) is retried. DNS
resolution failure, TCP connect timeout, and TLS handshake errors all
propagate immediately out of the enrollment loop via `return Err(e)`, causing
the process to exit. In environments with intermittent DNS or network
instability this means the service will terminate and require an external
restart (e.g. systemd) rather than retrying autonomously. The backoff
infrastructure is already in place; extending the retry condition to cover
transient network errors would improve resilience with minimal additional code.

---

## Database

### Strengths

This crate does not use a database directly. No findings apply.

### Issues

No findings apply to this crate.

---

## Coding Standards

### Strengths

**Edition 2024 and workspace version inheritance.**
`Cargo.toml:2-7` correctly uses `edition = "2024"` and inherits `license`,
`authors`, `repository`, and `version` from workspace defaults. All major
dependencies except `uptrakit-build-info` and `uptrakit-directories` are
workspace-pinned.

**No `#[allow(clippy::...)]` suppressions.**
The crate contains zero clippy suppressions, consistent with the workspace-wide
standard.

**`async_trait` used consistently.**
`ServiceHandler` methods are annotated with `#[async_trait]`, matching the
established pattern used by `Plugin`, `CommandExecutor`, and `TaskExecutor`
across the codebase.

**`thiserror`-derived errors with semantic variants.**
`LoopError` in `lifecycle.rs:30-41` uses `thiserror::Error` with three
meaningful variants that carry exactly the information needed by the lifecycle
to decide what to do next.

### Issues

---

## Extensibility

### Strengths

**`ServiceHandler` externalises the entire service-specific surface.**
The trait provides eight well-defined extension points. Adding a new service
type requires implementing the trait — nothing inside the SDK needs to change.
The default implementations of `on_settings` and `capabilities` keep
simple services concise.

**`ServiceEvent` associated type supports heterogeneous event sources.**
The `type ServiceEvent: Send` associated type, combined with
`poll_service_event` and `on_service_event`, lets each service add its own
`tokio::select!` arm without forking the event loop. The `Infallible` type is
recommended for services with no custom events.

**`LoopOutcome::Restart` variant supports SIGHUP-based restarts.**
The agent can return `LoopOutcome::Restart` from `on_service_event` to
request a clean process exit for external restart without encoding this logic
in the SDK.

**`LoopError::Other(String)` forward compatibility.**
New transient error conditions can be reported through `Other` without
requiring an SDK change, preserving backward compatibility.

#### 2026-02-24 Review

#### Strengths

- **Event loop cleanly separates SDK-managed from service-delegated messages.** `src/event_loop.rs:54-208` — New `ControllerMessage` variants are automatically forwarded to handlers.

#### Issues

**[SEVERITY: Low]** `src/lifecycle.rs:76` — `ServiceHandler` requires `Send` but not `Sync`, inconsistent with `Plugin` trait

Current constraint is correct for single-owner pattern but warrants a design-note comment.

### Issues

**[SEVERITY: Medium]** `lifecycle.rs:79, 89` — `ServiceHandler` is not
object-safe due to associated constants; this is undocumented.

```rust
const DIR_NAME: &'static str;
const SERVICE_LABEL: &'static str;
const SERVICE_TYPE: ServiceType;
```

Associated constants prevent `dyn ServiceHandler` from being used as a trait
object. There is no `where Self: Sized` guard on the trait or its methods, no
`// not object-safe` comment, and no documentation explaining this constraint.
A future implementor who attempts to store a `Box<dyn ServiceHandler>` or
`Arc<dyn ServiceHandler>` will receive a cryptic compiler error pointing at
the `const` items rather than a clear explanation.

The fix is either to add a `// Note: ServiceHandler is not object-safe due to
associated constants DIR_NAME, SERVICE_LABEL, and SERVICE_TYPE.` comment
directly above the trait definition, or — if object-safety is ever required —
to convert the constants to `fn` methods returning `&'static str` /
`ServiceType` (which are object-safe).

**[SEVERITY: Low]** `lifecycle.rs:142` — `poll_service_event` cancellation
safety requirement is undocumented.

```rust
async fn poll_service_event(&mut self) -> Self::ServiceEvent;
```

This future is used as an arm in `tokio::select!`. When another arm fires,
Tokio drops the `poll_service_event` future mid-poll. If an implementation
holds a lock, modifies shared state, or issues I/O inside `poll_service_event`,
that work is silently discarded. The trait documentation should state the
cancellation-safety requirement explicitly, matching the pattern used for
`tokio::io::AsyncRead` and similar traits in the ecosystem. For example:

```
/// # Cancellation Safety
///
/// This future MUST be cancellation-safe: it is dropped by `tokio::select!`
/// whenever another arm fires. Implementations must not perform non-idempotent
/// side effects inside this method.
```
