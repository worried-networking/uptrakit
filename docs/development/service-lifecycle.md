# Service Lifecycle

The `uptrakit-service-sdk` crate provides a `ServiceHandler` trait and `run_service_lifecycle()` function that encapsulate the entire
bootstrap-enrollment-reconnect flow shared by all Uptrakit services (agent, SSH agent, MQTT, and any future capability combinations).

## Overview

Building a new Uptrakit service requires implementing a set of callbacks on the `ServiceHandler` trait plus two associated constants and a
`capabilities()` method. The SDK owns the entire event loop (`tokio::select!`) and handles all common plumbing: CLI argument parsing, directory
resolution, identity management, CA bootstrap, enrollment with backoff, certificate renewal, ping/pong keepalive, signal handling, CA staleness
checks, and reconnection with exponential backoff.

## The `ServiceHandler` trait

```rust
#[async_trait]
pub trait ServiceHandler: Send {
    const DIR_NAME: &'static str;
    const SERVICE_LABEL: &'static str;

    type ServiceEvent: Send;

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::new()
    }

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()>;

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>>;

    async fn on_settings(&mut self, _settings: &ServiceSettingsPayload) {}

    async fn poll_service_event(&mut self) -> Self::ServiceEvent;

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>>;

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome;
}
```

### Associated constants

| Constant | Purpose |
| --- | --- |
| `DIR_NAME` | Directory name for platform-specific resolution (e.g. `"agent"`, `"mqtt"`). |
| `SERVICE_LABEL` | Human-readable label for log messages (e.g. `"uptrakit-agent service"`). |

### `capabilities()` method

Returns the `BTreeSet<Capability>` that this service advertises during enrollment and in `ReportHosts`. The
SDK intersects this set with the controller's advertised capabilities (from `ServiceSettings`) to compute
the agreed capability set. Only typed (known) variants participate in the intersection.

The default implementation returns an empty set. Services should override this to advertise their actual
capabilities. For example, the local agent returns `{GracefulShutdown, SoftwareDiscovery, UpdateHooks}`.

On the controller side, the persisted capability set is used to derive a `ServiceProfile` (Agent,
MqttBridge, or Unknown) which drives behavioral defaults such as ping interval, shutdown timeout, and
human-readable `service_label`. See [ServiceProfile derivation](#serviceprofile-derivation) below.

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

Called after the SDK processes the shared `ServiceSettings` fields (capability negotiation, renewal
schedule, shutdown timeout, CA staleness). Override for service-specific settings processing.
Default is a no-op.

#### `poll_service_event`

Poll for service-specific events (additional `select!` arm). Return `std::future::pending()` if the
service has no custom events. The returned future is dropped when another `select!` arm fires,
releasing the `&mut self` borrow.

#### `on_service_event`

Handle a resolved service event from `poll_service_event`. Return `Ok(Some(outcome))` to break the loop, `Ok(None)` to continue.

#### `on_shutdown`

Graceful shutdown handler. Called when an OS signal or `ServerRestarting` message is received. Send `Disconnecting` and drain in-flight work. The
`cause` parameter is a `ShutdownCause` enum that distinguishes OS signals (`Signal::Hangup` for restart, `Signal::Interrupt`/`Signal::Terminate` for
shutdown) from controller-initiated restarts. `shutdown_timeout_seconds` comes from the latest `ServiceSettings`.

### `LoopOutcome`

| Variant | Meaning | SDK behavior |
| --- | --- | --- |
| `Shutdown` | SIGINT/SIGTERM received | Exit the lifecycle cleanly |
| `Reconnect` | Certificate rotated | Reconnect immediately (reset backoff) |
| `Disconnected` | Connection closed | Reconnect with exponential backoff |
| `Restart` | Graceful restart via SIGHUP | Exit the lifecycle |

### `LoopError`

`LoopError` is a `thiserror`-backed enum with four variants:

| Variant | Meaning | SDK behavior |
| --- | --- | --- |
| `CertExpired` | TLS handshake rejected (certificate expired) | Re-enroll |
| `ReceiveClosed` | WebSocket cleanly closed by controller | Reconnect with backoff |
| `TransientNetwork(String)` | Transient network error (broken pipe, connection reset, DNS failure, send timeout) | Reconnect with backoff |
| `Other(String)` | Any other event-loop error | Propagate |

Callbacks return `LoopResult<T>` (alias for `Result<T, Report<LoopError>>`), following the project-wide
`Report<T>` convention. An `impl_report_conversion!(EnrollmentError => LoopError, ...)` closure-based
conversion enables `.context_to::<LoopError>()?` on SDK connection operations inside callbacks.

### Why `#[async_trait]`

All async trait methods use the `#[async_trait]` macro, which desugars `async fn` into `Pin<Box<dyn Future + Send + '_>>` return types. This matches the
established pattern used across the codebase (Plugin, CommandExecutor, TaskExecutor, CertSigner, etc.) and eliminates the manual `Pin<Box<...>>` /
`Box::pin(async move { ... })` boilerplate that was previously required in the trait definition and all implementations.

## SDK-managed event loop

The SDK owns the `tokio::select!` loop in `event_loop::run_event_loop()`. Services no longer write their own select loop. The event loop handles (in
biased priority order):

1. **Service events** (`handler.poll_service_event()`) — highest priority
1. **Ping timer** — sends `Ping` messages at the controller-provided `ping_interval` (conditional; see below)
1. **Renewal timer** — proactive certificate renewal
1. **Controller messages** (`conn.recv()`) — dispatched to SDK handlers or `handler.on_message()`
1. **OS signals** (`signals.recv()`) — `handler.on_shutdown()`

After the event loop exits, the SDK attempts a clean WebSocket close with a **5-second timeout**
(`CLOSE_TIMEOUT`). If the controller is unresponsive, the close times out and the connection is
dropped without waiting for the TCP stack to give up (which can take minutes).

### Conditional ping timer

The ping timer starts as `None` (no pings are sent). When a `ServiceSettings` message arrives, the SDK
reads the `ping_interval` field (a `Duration` set by the controller per-service) and creates a
`tokio::time::Interval` with that duration. The first immediate tick is consumed during setup so the
first actual ping fires after one full interval. If a subsequent `ServiceSettings` message arrives with a
different interval, the timer is replaced.

This design means the ping interval is fully controller-managed. The `ServiceHandler` trait no longer
exposes a `ping_interval()` method. The controller derives the interval from a per-service database
override (`services.ping_interval_seconds`) or falls back to `ServiceProfile`-based defaults (300s for
Agent profile, 15s for MqttBridge profile).

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
| `init_crypto()` | Install the `aws-lc-rs` rustls crypto provider. |
| `print_build_info(name, version, features)` | Print build metadata for `--version`. |
| `run_lifecycle_and_handle_errors(name, args, handler)` | Run the lifecycle and handle errors (log + exit code). |

> **Note:** Tracing subscriber initialization (`tracing_subscriber::fmt().init()`) is intentionally **not** provided
> by the SDK. Libraries must never configure the global tracing dispatcher. Each binary is responsible for
> initializing tracing in its own `main()` before calling SDK functions.

## Example: minimal service

```rust
use std::collections::BTreeSet;
use async_trait::async_trait;
use uptrakit_internal_wire::{Capability, ControllerMessage};
use uptrakit_service_sdk::{
    ControllerConnection, LoopOutcome, LoopResult, ServiceHandler,
    ServiceIdentityState, ShutdownCause,
};

struct MyHandler;

#[async_trait]
impl ServiceHandler for MyHandler {
    const DIR_NAME: &'static str = "my-service";
    const SERVICE_LABEL: &'static str = "uptrakit-my-service";
    const SERVICE_APP_NAME: &'static str = env!("CARGO_PKG_NAME");

    type ServiceEvent = std::convert::Infallible;

    fn capabilities(&self) -> BTreeSet<Capability> {
        [Capability::GracefulShutdown, Capability::SoftwareDiscovery]
            .into_iter()
            .collect()
    }

    async fn on_connected(
        &mut self,
        _conn: &mut ControllerConnection,
        _identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        Ok(())
    }

    async fn on_message(
        &mut self,
        _msg: ControllerMessage,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(None)
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        std::future::pending().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {} // Infallible
    }

    async fn on_shutdown(
        &mut self,
        _conn: &mut ControllerConnection,
        _cause: ShutdownCause,
        _shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        LoopOutcome::Shutdown
    }
}

#[tokio::main]
async fn main() {
    let args = clap::Parser::parse();

    // Each binary initializes its own tracing subscriber.
    // The SDK does not provide init_tracing() — libraries must not configure
    // the global dispatcher.
    let mut filter = tracing_subscriber::EnvFilter::from_default_env();
    if let Ok(directive) = "uptrakit_my_service=info".parse() {
        filter = filter.add_directive(directive);
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();

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

All services (agent, SSH agent, MQTT) use these helpers for consistent renewal behavior.

### Internal state

The handler owns a single field — `pending_renewal_key: Option<String>` — that holds the private key PEM between the `RequestCertRenewal` →
`Certificate` pair (or between a timer-based `initiate_renewal` call and the subsequent `Certificate` response). When `handle_certificate` is called,
it takes the pending key, persists both the certificate and the key via `identity.save_certificate()` and `identity.save_private_key()`, and returns
`LoopOutcome::Reconnect`.

## Error handling at the trait boundary

The lifecycle works with `Report<EnrollmentError>`. Service callbacks return `LoopResult<T>` (i.e.
`Result<T, Report<LoopError>>`). `LoopError` is a `thiserror` enum with four variants (`CertExpired`,
`ReceiveClosed`, `TransientNetwork`, `Other`), and a closure-based `impl_report_conversion!` maps
`EnrollmentError` to the appropriate variant. This enables services to use
`.context_to::<LoopError>()?` on SDK connection operations (e.g.
`conn.send(msg).await.context_to::<LoopError>()?`).

The event loop intercepts transient network errors (broken pipe, connection reset, etc.) from
`conn.recv()` and breaks with `LoopOutcome::Disconnected` instead of propagating them as fatal
errors. As defense-in-depth, `run_authenticated_with_reconnect` also catches `TransientNetwork`
and `ReceiveClosed` errors from handler callbacks and treats them as disconnections (reconnect
with backoff) rather than fatal exits.

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

## Capability-based enrollment

Services no longer carry a `ServiceType` enum. Instead, each service advertises a `BTreeSet<Capability>`
during enrollment (in `EnrollPayload.capabilities`) and on every authenticated connect (in
`ReportHostsPayload.capabilities`). The controller persists these capabilities in the
`services.capabilities` column as a JSON array of snake_case strings (e.g.
`["graceful_shutdown","software_discovery","update_hooks"]`).

A single enrollment token (`service_enrollment.token_hash` in the settings table) is shared across all
service kinds. The previous per-type tokens (`agent_enrollment.token_hash`,
`mqtt_enrollment.token_hash`, `ssh_agent_enrollment.token_hash`) have been consolidated into this
single key.

The connection registry exposes a unified `register()` method that accepts a `BTreeSet<Capability>`
parameter. The previous type-specific methods (`register_agent()`, `register_mqtt()`,
`register_ssh_agent()`) have been removed.

## ServiceProfile derivation

`ServiceProfile` is a controller-side enum that is **never persisted** in the database. It is always
derived from the service's persisted capability set via `ServiceProfile::from_capabilities()`.

| Profile | Key capability | Example services |
| --- | --- | --- |
| `MqttBridge` | `Capability::MqttBridge` | MQTT service |
| `Agent` | `Capability::SoftwareDiscovery` | Local agent, SSH agent |
| `Unknown` | (fallback) | Unrecognized combinations |

`MqttBridge` takes precedence if both `MqttBridge` and `SoftwareDiscovery` are present.

The profile drives behavioral defaults:

| Default | MqttBridge | Agent | Unknown |
| --- | --- | --- | --- |
| `default_ping_interval_secs` | 15 | 300 | 300 |
| `shutdown_timeout_secs` | None | Some(120) | Some(120) |
| `service_label(false)` | "MQTT Bridge" | "Agent" | "Unknown" |
| `service_label(true)` | "MQTT Bridge" | "SSH Agent" | "Unknown" |

The `service_label` column in API responses (`ServiceResponse.service_label`) is derived at query time
from the profile and the presence of `Capability::SshRemote`. It is not stored in the database.

## Identity state persistence

`ServiceIdentityState` manages the `service.json` file in the state directory. The file contains:

| Field | Type | Description |
| --- | --- | --- |
| `service_id` | UUID | Assigned by the controller during enrollment |
| `enrollment_secret` | String | Bearer token for pre-certificate auth (cleared after certificate issuance) |
| `tenant_id` | UUID (optional) | Received from the controller via `ServiceSettings`; persisted so CLI commands can use it offline |

The `tenant_id` field uses `#[serde(default, skip_serializing_if = "Option::is_none")]` for backward
compatibility with existing `service.json` files that predate the field. Services persist the
tenant_id by calling `identity.save_tenant_id(tid)` when they receive it in `on_settings()`. CLI
commands (e.g. the SSH agent's `host sync` and `host bootstrap`) load the persisted tenant_id from
the identity state instead of requiring a live controller connection.

## Related documentation

- [Services and Operations](../api/services-operations.md) — shared startup flow and API operations
- [Wire Protocol](../api/wire-protocol.md) — WebSocket message taxonomy
- [Coding Standards](coding-standards.md) — error handling conventions
- [Security Architecture](../security/security-architecture.md) — mTLS and enrollment security model
- [TOFU and TLS](../security/tofu-tls.md) — CA bootstrap and TLS hardening
