# Service Lifecycle

The `uptrakit-service-sdk` crate provides a `ServiceHandler` trait and `run_service_lifecycle()` function
that encapsulate the entire bootstrap-enrollment-reconnect flow shared by all Uptrakit services (agent,
MQTT, and any future service types).

## Overview

Building a new Uptrakit service requires implementing three trait methods. The SDK handles all common
plumbing: CLI argument parsing, directory resolution, identity management, CA bootstrap, enrollment with
backoff, certificate expiry detection, and reconnection with exponential backoff.

## The `ServiceHandler` trait

```rust
pub trait ServiceHandler {
    fn config(&self) -> ServiceConfig;
    fn enrollment_info(&self) -> ServiceEnrollmentInfo;
    fn run_authenticated_loop<'a>(
        &'a mut self,
        ctx: AuthenticatedContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LoopOutcome>> + Send + 'a>>;
}
```

### `config()`

Returns static configuration for the service:

- `dir_name` — directory name used for platform-specific directory resolution (e.g. `"agent"`, `"mqtt"`).
- `service_label` — human-readable label for log messages (e.g. `"uptrakit-agent service"`).

### `enrollment_info()`

Returns enrollment-time parameters:

- `service_type` — `ServiceType::Agent` or `ServiceType::Mqtt`.
- `host_info` — optional `HostInfo` struct. The agent collects host metadata; MQTT sends `None`.

### `run_authenticated_loop()`

Runs the service-specific authenticated event loop. Receives an `AuthenticatedContext` containing:

- `host` / `port` — controller address.
- `tls_connector` — pre-built mTLS connector (rebuilt on each reconnect iteration since certificates may have rotated).
- `ca_pem` — raw CA PEM bytes if a pinned CA is in use.
- `identity` — the loaded `ServiceIdentityState` (certified).
- `base_url` — controller base URL (e.g. `https://host:8443`).
- `pki_addr` — optional PKI address.

Returns a `LoopOutcome`:

| Variant | Meaning | SDK behavior |
|---|---|---|
| `Shutdown` | SIGINT/SIGTERM received | Exit the lifecycle cleanly |
| `Reconnect` | Certificate rotated | Reconnect immediately (reset backoff) |
| `Disconnected` | Connection closed | Reconnect with exponential backoff |
| `Restart` | Service-specific restart (e.g. agent SIGHUP) | Exit the lifecycle |

The return type uses a boxed future (`Pin<Box<dyn Future + Send + 'a>>`) to avoid higher-ranked lifetime
issues that arise with `impl Future` in trait methods when the implementation captures references with
complex lifetime relationships (e.g. streaming iterators with `buffer_unordered`).

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
2. Resolve application directories and create them with secure permissions (0o700).
3. Load identity state from disk.
4. Handle `--force-enroll` by clearing existing enrollment state.
5. Bootstrap CA certificate (cached, file, PKI endpoint, TOFU, or system trust).
6. If already certified: check expiry, try authenticated loop, fall back to enrollment on `CertificateExpired`.
7. Build TLS connector for enrollment (server-auth only, no client cert).
8. Run enrollment with exponential backoff on disconnects.
9. Enter the authenticated loop with reconnection (backoff for `Disconnected`, immediate reconnect for `Reconnect`, exit for `Shutdown`/`Restart`).

The mTLS connector is rebuilt on each reconnect iteration inside the loop, because certificates may have
been rotated since the last connection.

## Example: minimal service

```rust
use uptrakit_service_sdk::{
    AuthenticatedContext, ControllerConnection, LoopOutcome,
    ServiceConfig, ServiceEnrollmentInfo, ServiceHandler,
};

struct MyHandler;

impl ServiceHandler for MyHandler {
    fn config(&self) -> ServiceConfig {
        ServiceConfig {
            dir_name: "my-service",
            service_label: "uptrakit-my-service",
        }
    }

    fn enrollment_info(&self) -> ServiceEnrollmentInfo {
        ServiceEnrollmentInfo {
            service_type: uptrakit_internal_wire::ServiceType::Agent,
            host_info: None,
        }
    }

    fn run_authenticated_loop<'a>(
        &'a mut self,
        ctx: AuthenticatedContext<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = uptrakit_service_sdk::Result<LoopOutcome>> + Send + 'a>,
    > {
        Box::pin(async move {
            let mut conn = ControllerConnection::connect(
                ctx.host, ctx.port, &ctx.tls_connector, None,
            ).await?;

            // Service-specific message loop here...

            let _ = conn.close().await;
            Ok(LoopOutcome::Shutdown)
        })
    }
}

#[tokio::main]
async fn main() {
    // ... tracing, crypto provider setup, CLI parsing ...
    let mut handler = MyHandler;
    if let Err(e) = uptrakit_service_sdk::run_service_lifecycle(&args.common, &mut handler).await {
        tracing::error!(error = %e, "service failed");
        std::process::exit(1);
    }
}
```

## Error handling at the trait boundary

The lifecycle works with `Report<EnrollmentError>`. The handler's `run_authenticated_loop` returns
`Result<LoopOutcome>` using the SDK's `Result` type. Each service converts its internal error type
to `EnrollmentError` at the boundary. The lifecycle only needs two semantic checks:

- `is_cert_expired()` — triggers enrollment fallback.
- `is_receive_closed()` — triggers reconnect with backoff.

All other errors propagate up and terminate the lifecycle.

## Related documentation

- [Services and Operations](../api/services-operations.md) — shared startup flow and API operations
- [Wire Protocol](../api/wire-protocol.md) — WebSocket message taxonomy
- [Coding Standards](coding-standards.md) — error handling conventions
- [Security Architecture](../security/security-architecture.md) — mTLS and enrollment security model
- [TOFU and TLS](../security/tofu-tls.md) — CA bootstrap and TLS hardening
