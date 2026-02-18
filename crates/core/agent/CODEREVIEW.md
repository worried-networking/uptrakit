# Code Review: `uptrakit-agent`

Reviewed: `src/main.rs` (120 lines), `src/client.rs` (649 lines),
`src/error.rs` (122 lines), `src/host_info.rs`, `src/update.rs`,
`src/version_check.rs`, `src/cli.rs`, `Cargo.toml`.

## Summary

The agent crate is well-structured with proper error delegation, graceful
shutdown handling, and bounded output buffers. Key issues are the transitive
provider-registry dependency, manual error conversion at the `ServiceHandler`
boundary, and missing platform portability guards.

## Dependency Analysis

| Dependency | Purpose | Concern |
| --- | --- | --- |
| `uptrakit-provider-registry` | Instantiate providers from JSON config | Pulls in **all** provider implementation crates transitively |
| `uptrakit-service-sdk` | Service lifecycle, enrollment, TLS | Clean; appropriate for an agent binary |
| `uptrakit-internal-wire` | Wire protocol messages | Clean; required for controller communication |
| `uptrakit-command` | Local command execution | Clean; minimal dependency |
| `uptrakit-shared-types` | Shared enums and value types | Clean; leaf crate |

## Findings

### High

#### H1: Agent depends on `provider-registry` (extensibility)

**Location:** `Cargo.toml:28`

The agent binary depends on `uptrakit-provider-registry`, which transitively
pulls in every concrete provider crate (GitHub, Docker Registry, Proxmox
Helper Scripts, Homebrew). The agent needs the registry to instantiate
providers from JSON config for local version detection and update execution.
This is **architecturally justified** -- the agent must be able to run any
provider locally -- but it creates a large transitive dependency footprint
that couples the agent binary to all provider implementations at compile
time.

**Impact:** Adding a new provider crate increases the agent's compile time
and binary size, even if a given agent deployment never uses that provider.

**Recommendation:** Introduce a `ProviderFactory` trait (in `provider-core`
or a new `provider-factory` crate) that the registry implements:

```rust
pub trait ProviderFactory: Send + Sync {
    fn create_provider(
        &self,
        provider_type: ProviderType,
        config: &serde_json::Value,
        executor: Arc<dyn CommandExecutor>,
    ) -> Result<Box<dyn Provider>>;
}
```

The agent would depend on the factory trait rather than the concrete
registry. Alternatively, compile provider selection behind **feature flags**
so deployments can opt into only the providers they need.

#### H2: Unix-only signal handling without `#[cfg(unix)]` guard

**File:** `src/client.rs:85-88`

The SIGTERM and SIGHUP signal handlers use `tokio::signal::unix::signal()`
without any `#[cfg(unix)]` guard. This will fail to compile on non-Unix
platforms (e.g., Windows).

```rust
let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    .context_to::<Error>()?;
let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
    .context_to::<Error>()?;
```

The MQTT crate (`crates/core/mqtt/src/main.rs:180-186, 287-312`) handles
this correctly with `#[cfg(unix)]` and `#[cfg(not(unix))]`
`futures_util::future::pending()` fallbacks.

**Recommendation:** Add `#[cfg(unix)]` / `#[cfg(not(unix))]` guards matching
the pattern used in the MQTT crate.

### Medium

#### M1: Manual error conversion reconstructs `EnrollmentError` variants

**File:** `src/main.rs:58-76`

The `ServiceHandler::run_authenticated_loop` implementation manually
reconstructs `EnrollmentError` variants from the agent's internal `Error`
type. This is fragile because it duplicates the error classification logic
(is_cert_expired, is_receive_closed) and creates new error instances that
lose the original error chain.

```rust
if ctx.is_cert_expired() {
    Err(report!(uptrakit_service_sdk::EnrollmentError::Tls(
        uptrakit_service_sdk::TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateExpired,
        ))
    )))
```

**Recommendation:** Consider a conversion function on the SDK side that
accepts the semantic flags (cert expired, receive closed) and an error
message, or implement a `From` conversion that preserves the error context.
The agent-ssh crate (`src/main.rs`) likely has the same pattern.

#### M2: In-flight update task panic handled but not propagated

**File:** `src/client.rs:460-472`

When the update task panics (caught by `JoinError`), the agent sends a
failure result to the controller and logs an error, but the panic message
from `JoinError` is not included in the error sent to the controller.

```rust
Err(e) => {
    tracing::error!(error = %e, "update task panicked");
    conn.send_best_effort(ServiceMessage::UpdateResult(UpdateResultPayload {
        ...
        error: Some("Update task panicked".to_string()),
    })).await;
}
```

**Recommendation:** Include the panic message in the error sent to the
controller: `Some(format!("Update task panicked: {e}"))`. This aids
debugging.

### Low

#### L1: `DEFAULT_SHUTDOWN_TIMEOUT` and `PING_INTERVAL` are not configurable

**File:** `src/client.rs:63-64`

Both constants are hardcoded. `shutdown_timeout_seconds` is later updated
from `ServiceSettings` (line 212), but `PING_INTERVAL` (300s) has no
override mechanism.

**Recommendation:** Consider making `PING_INTERVAL` configurable via CLI
args if different deployment scenarios require different ping frequencies.

#### L2: `compute_local_ca_hash` returns empty string on error

**File:** `src/client.rs:557-567`

When the CA file cannot be read, `compute_local_ca_hash` returns an empty
string. This means a missing CA file will always trigger a CA bundle fetch
on every `ServiceSettings` message with a non-empty hash.

**Recommendation:** This behavior is acceptable (self-healing), but worth
documenting with a brief comment explaining the intent.

### Info

#### I1: Bounded output buffer prevents OOM

**File:** `src/update.rs`

The update execution uses a bounded output buffer (10 MB) to prevent
runaway processes from exhausting memory. This is a good defensive practice.

#### I2: Concurrent version checks with `buffer_unordered(8)`

**File:** `src/client.rs:282-308`

Version checks are run concurrently with a concurrency limit of 8. The
pre-refresh step deduplicates by provider type to avoid redundant index
refreshes. Good design.

#### I3: Graceful shutdown drains in-flight updates

**File:** `src/client.rs:479-554`

The `handle_graceful_shutdown` function properly waits for in-flight updates
with a configurable timeout, drains remaining output messages, and sends the
`Disconnecting` message to the controller. Edge cases (timeout reached,
remaining output) are handled.

#### I4: Error classification tests are thorough

**File:** `src/error.rs:59-121`

Tests cover `is_receive_closed` and `is_cert_expired` across all error
variant paths, including negative cases (e.g., `UpdateExecution` containing
"CertificateExpired" in the string should not match).

#### I5: Clean SDK usage

Uses `service-sdk` for lifecycle management, enrollment, and TLS -- clean
separation. Command execution is abstracted via `CommandExecutor` trait
injection. No direct database dependency; the agent is stateless beyond its
service identity.
