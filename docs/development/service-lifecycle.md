# Service Lifecycle

The `uptrakit-service-sdk` crate provides a `ServiceHandler` trait and `run_service_lifecycle()` function that encapsulate the entire
bootstrap-enrollment-reconnect flow shared by all Uptrakit services (agent, SSH agent, MQTT, and any future service types).

## Overview

Building a new Uptrakit service requires implementing a set of callbacks on the `ServiceHandler` trait plus three associated constants. The SDK owns the
entire event loop (`tokio::select!`) and handles all common plumbing: CLI argument parsing, directory resolution, identity management, CA bootstrap,
enrollment with backoff, certificate renewal, ping/pong keepalive, signal handling, CA staleness checks, and reconnection with exponential backoff.

## The `ServiceHandler` trait

```rust
#[async_trait]
pub trait ServiceHandler: Send {
    const DIR_NAME: &'static str;
    const SERVICE_LABEL: &'static str;
    const SERVICE_TYPE: ServiceType;

    type ServiceEvent: Send;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &ServiceIdentityState,
    ) -> Result<(), LoopError>;

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> Result<Option<LoopOutcome>, LoopError>;

    async fn on_settings(&mut self, _settings: &ServiceSettingsPayload) {}

    async fn poll_service_event(&mut self) -> Self::ServiceEvent;

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> Result<Option<LoopOutcome>, LoopError>;

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        signal: Signal,
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome;

    fn ping_interval(&self) -> Duration {
        Duration::from_secs(300) // default 5 minutes
    }
}
```

### Associated constants

| Constant | Purpose |
| --- | --- |
| `DIR_NAME` | Directory name for platform-specific resolution (e.g. `"agent"`, `"mqtt"`). |
| `SERVICE_LABEL` | Human-readable label for log messages (e.g. `"uptrakit-agent service"`). |
| `SERVICE_TYPE` | `ServiceType::Agent`, `ServiceType::Mqtt`, or `ServiceType::SshAgent`. |

### `ServiceEvent` associated type

Each service declares an associated type representing events from its custom `select!` arm. Use `std::convert::Infallible` for services with no custom
events (the SDK will call `std::future::pending()` and the arm will never fire).

### Callbacks

#### `on_connected`

Called after the WebSocket connection is established. Use this to send initial messages (e.g. `ReportHosts`, `Register`).

#### `on_message`

Handle a `ControllerMessage` not handled by the SDK. The SDK handles: `Pong`, `Certificate`,
`ServiceSettings`, `CaBundleUpdated`, `RequestCertRenewal`, and `ServerRestarting`. Everything else
is delegated to this callback. Return `Ok(Some(outcome))` to break the loop, `Ok(None)` to continue.

#### `on_settings`

Called after the SDK processes the shared `ServiceSettings` fields (protocol version check, renewal
schedule, shutdown timeout, CA staleness). Override for service-specific settings processing.
Default is a no-op.

#### `poll_service_event`

Poll for service-specific events (additional `select!` arm). Return `std::future::pending()` if the
service has no custom events. The returned future is dropped when another `select!` arm fires,
releasing the `&mut self` borrow.

#### `on_service_event`

Handle a resolved service event from `poll_service_event`. Return `Ok(Some(outcome))` to break the loop, `Ok(None)` to continue.

#### `on_shutdown`

Graceful shutdown handler. Called when an OS signal is received. Send `Disconnecting` and drain in-flight work. The `signal` parameter distinguishes
`Signal::Hangup` (restart) from `Signal::Interrupt`/`Signal::Terminate` (shutdown). `shutdown_timeout_seconds` comes from the latest `ServiceSettings`.

#### `ping_interval`

Override to change the keepalive ping interval. Default is 300 seconds. The MQTT service overrides this with its configurable value.

### `LoopOutcome`

| Variant | Meaning | SDK behavior |
| --- | --- | --- |
| `Shutdown` | SIGINT/SIGTERM received | Exit the lifecycle cleanly |
| `Reconnect` | Certificate rotated | Reconnect immediately (reset backoff) |
| `Disconnected` | Connection closed | Reconnect with exponential backoff |
| `Restart` | Graceful restart via SIGHUP | Exit the lifecycle |

### `LoopError`

Callbacks return `Result<_, LoopError>`. `LoopError` carries semantic flags (`cert_expired`, `receive_closed`) so the lifecycle can decide whether to
re-enroll, reconnect with backoff, or propagate the error. A `From<Report<EnrollmentError>>` impl enables using `?` on SDK connection operations.

### Why `#[async_trait]`

All async trait methods use the `#[async_trait]` macro, which desugars `async fn` into `Pin<Box<dyn Future + Send + '_>>` return types. This matches the
established pattern used across the codebase (Provider, CommandExecutor, TaskExecutor, CertSigner, etc.) and eliminates the manual `Pin<Box<...>>` /
`Box::pin(async move { ... })` boilerplate that was previously required in the trait definition and all implementations.

## SDK-managed event loop

The SDK owns the `tokio::select!` loop in `event_loop::run_event_loop()`. Services no longer write their own select loop. The event loop handles (in
biased priority order):

1. **Service events** (`handler.poll_service_event()`) — highest priority
1. **Ping timer** — sends `Ping` messages at `handler.ping_interval()` intervals
1. **Renewal timer** — proactive certificate renewal
1. **Controller messages** (`conn.recv()`) — dispatched to SDK handlers or `handler.on_message()`
1. **OS signals** (`signals.recv()`) — `handler.on_shutdown()`

### `EventLoopContext`

Passed to the event loop by the lifecycle. Contains connection metadata:

```rust
pub struct EventLoopContext<'a> {
    pub base_url: &'a str,
    pub pki_addr: Option<&'a str>,
    pub ca_pem: Option<&'a [u8]>,
}
```

## `run_service_lifecycle()`

The single entry point that replaces per-service `run()` functions:

```rust
pub async fn run_service_lifecycle(
    args: &CommonServiceArgs,
    handler: &mut impl ServiceHandler,
) -> Result<()>
```

It executes the following sequence:

1. Parse URL from CLI arguments.
1. Resolve application directories and create them with secure permissions (0o700).
1. Load identity state from disk.
1. Handle `--force-enroll` by clearing existing enrollment state.
1. Bootstrap CA certificate (cached, file, PKI endpoint, TOFU, or system trust).
1. If already certified: check expiry, try authenticated loop, fall back to enrollment on `CertificateExpired`.
1. Build TLS connector for enrollment (server-auth only, no client cert).
1. Run enrollment with exponential backoff on disconnects.
1. Enter the authenticated loop with reconnection (backoff for `Disconnected`, immediate reconnect for `Reconnect`, exit for `Shutdown`/`Restart`).

The mTLS connector is rebuilt on each reconnect iteration inside the loop, because certificates may have been rotated since the last connection.

## Main helpers

The SDK provides shared initialization and error-handling functions to reduce boilerplate in `main()`:

| Function | Purpose |
| --- | --- |
| `init_tracing(directive)` | Initialize `tracing_subscriber` with the given filter directive. |
| `init_crypto()` | Install the `aws-lc-rs` rustls crypto provider. |
| `print_build_info(name, version, features)` | Print build metadata for `--version`. |
| `run_lifecycle_and_handle_errors(name, args, handler)` | Run the lifecycle and handle errors (log + exit code). |

## Example: minimal service

```rust
use async_trait::async_trait;
use uptrakit_internal_wire::{ControllerMessage, ServiceType};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, ServiceHandler,
    ServiceIdentityState, Signal,
};

struct MyHandler;

#[async_trait]
impl ServiceHandler for MyHandler {
    const DIR_NAME: &'static str = "my-service";
    const SERVICE_LABEL: &'static str = "uptrakit-my-service";
    const SERVICE_TYPE: ServiceType = ServiceType::Agent;

    type ServiceEvent = std::convert::Infallible;

    async fn on_connected(
        &mut self,
        _conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> Result<(), LoopError> {
        Ok(())
    }

    async fn on_message(
        &mut self,
        _msg: ControllerMessage,
        _conn: &mut ControllerConnection,
    ) -> Result<Option<LoopOutcome>, LoopError> {
        Ok(None)
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        std::future::pending().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        _conn: &mut ControllerConnection,
    ) -> Result<Option<LoopOutcome>, LoopError> {
        match event {} // Infallible
    }

    async fn on_shutdown(
        &mut self,
        _conn: &mut ControllerConnection,
        _signal: Signal,
        _shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        LoopOutcome::Shutdown
    }
}

#[tokio::main]
async fn main() {
    let args = clap::Parser::parse();
    uptrakit_service_sdk::init_tracing("uptrakit_my_service=info");
    uptrakit_service_sdk::init_crypto();

    let mut handler = MyHandler;
    uptrakit_service_sdk::run_lifecycle_and_handle_errors(
        "uptrakit-my-service", &args.common, &mut handler,
    ).await;
}
```

## Signal handling

The SDK provides a cross-platform `SignalWatcher` that encapsulates `SIGINT`, `SIGTERM`, and `SIGHUP` handling:

```rust
pub enum Signal { Interrupt, Terminate, Hangup }

pub struct SignalWatcher { /* platform-specific internals */ }

impl SignalWatcher {
    pub fn new() -> std::io::Result<Self>;
    pub async fn recv(&mut self) -> Signal;
}
```

On Unix, all three signals are monitored. On non-Unix platforms, only `Ctrl+C` (mapped to `Signal::Interrupt`) is available; the other arms use
`std::future::pending()`.

## `CertificateRenewalHandler`

The SDK provides a `CertificateRenewalHandler` struct (in `cert_handler` module) that encapsulates the three certificate-lifecycle controller messages
shared by all services: `CaBundleUpdated`, `RequestCertRenewal`, and `Certificate`. The SDK event loop delegates to this handler automatically — services
do not need to handle these messages.

### Renewal timer helpers

The SDK provides shared helper functions for proactive certificate renewal timers:

| Function | Purpose |
| --- | --- |
| `create_renewal_sleep()` | Creates a pinned `Sleep` initialized to `FAR_FUTURE` (30 days). |
| `update_renewal_schedule(sleep, cert_not_after_ts, window_hours)` | Resets the timer based on certificate expiry and renewal window. |
| `compute_renewal_delay(cert_not_after_ts, window_hours)` | Computes the delay until the renewal window opens. |
| `handle_renewal_timer(identity, conn, renewal_sleep)` | Initiates renewal, sends CSR, and resets timer. |

All three service types (agent, SSH agent, MQTT) use these helpers for consistent renewal behavior.

### Internal state

The handler owns a single field — `pending_renewal_key: Option<String>` — that holds the private key PEM between the `RequestCertRenewal` →
`Certificate` pair (or between a timer-based `initiate_renewal` call and the subsequent `Certificate` response). When `handle_certificate` is called,
it takes the pending key, persists both the certificate and the key via `identity.save_certificate()` and `identity.save_private_key()`, and returns
`LoopOutcome::Reconnect`.

## Error handling at the trait boundary

The lifecycle works with `Report<EnrollmentError>`. Service callbacks return `Result<_, LoopError>`. The `LoopError` type bridges the gap with semantic
flags:

- `cert_expired` — triggers enrollment fallback.
- `receive_closed` — triggers reconnect with backoff.

A `From<Report<EnrollmentError>>` impl on `LoopError` automatically extracts these flags from the SDK's error type, enabling services to use `?` on
SDK connection operations (e.g. `conn.send(msg).await.map_err(LoopError::from)?`).

### `EnrollmentError` sub-enum structure

`EnrollmentError` is organized into domain sub-enums for clearer error categorization:

| Sub-enum | Domain | Example variants |
| --- | --- | --- |
| `TlsError` | TLS/certificate errors | `Config`, `Rustls`, `NoCertificates`, `CertificateParse`, `Pem`, `InvalidDnsName` |
| `IdentityError` | Identity/enrollment state | `KeypairGeneration`, `CsrGeneration`, `NotEnrolled`, `NotCertified` |
| `ProtocolError` | Wire protocol/enrollment flow | `Init`, `ReceiveClosed`, `UnexpectedMessage`, `Enrollment`, `EnrollmentRejected`, timeouts |
| `CaError` | CA certificate operations | `Fetch`, `CertFile` |

Top-level variants (`Io`, `Json`, `WebSocket`, `HttpUri`, `Directory`) remain directly on `EnrollmentError`. Services can match on categories (e.g.,
`EnrollmentError::Tls(_)`) instead of individual variants for coarse-grained error handling.

## Related documentation

- [Services and Operations](../api/services-operations.md) — shared startup flow and API operations
- [Wire Protocol](../api/wire-protocol.md) — WebSocket message taxonomy
- [Coding Standards](coding-standards.md) — error handling conventions
- [Security Architecture](../security/security-architecture.md) — mTLS and enrollment security model
- [TOFU and TLS](../security/tofu-tls.md) — CA bootstrap and TLS hardening
