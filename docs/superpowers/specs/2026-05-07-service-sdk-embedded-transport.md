# service-sdk Embedded Transport Abstraction

**Date:** 2026-05-07
**Status:** Approved — pending implementation

---

## Problem

`ServiceHandler` trait methods take `&mut ControllerConnection` — a concrete WebSocket type from
`uptrakit-service-sdk`. This makes it impossible to use the same `ServiceHandler` implementation
for both standalone mode (WebSocket to controller) and embedded mode (in-process
`EmbeddedTransport` channels). Current embedded services (`run_embedded_ssh_agent`,
`run_embedded_mqtt`) bypass `ServiceHandler` entirely and maintain bespoke event loops in
`controller-runtime`, each duplicating service lifecycle logic.

The `service-sdk` prerequisite for the service binary/runtime boundary refactor:
`AgentSshHandler`, `MqttHandler`, and `SchedulerHandler` must each have a single implementation
that the controller passes to `run_embedded_service` without any bespoke wrapper.

---

## Established Pattern

`agent-runtime` and `mqtt-runtime` already use `&mut dyn ServiceTransport` throughout their
public methods. The thin `ServiceHandler` wrappers in binary crates bridge from
`&mut ControllerConnection` to `&mut dyn ServiceTransport` today only because
`ControllerConnection: ServiceTransport`. After this refactor the bridge coercion moves to the
SDK call sites, and binary-crate handlers delegate directly.

---

## Invariants After This Refactor

**Transport independence:** `ServiceHandler` implementations have **no dependency on
`ControllerConnection`**. The trait compiles against `uptrakit-wire` types only
(`ServiceTransport`, `ControllerMessage`, `ServiceMessage`, `Capability`). A handler impl that
compiles is transport-agnostic by construction.

**Embedded identity injection:** `run_embedded_service` does **not** call `on_connected`.
Embedded `ServiceHandler` implementations must **not** rely on `on_connected` for identity
material, credentials, or any initialization that the handler needs before `on_settings` fires.
Service identifiers, private key material, and any pre-connection state must be injected via the
handler constructor or a dedicated initialization method before `run_embedded_service` is called.
The first handler callback in the embedded path is `on_settings` — handler logic must be correct
at that point without having received an `on_connected` call. Any handler that assumes
`on_connected` always precedes `on_settings` will malfunction in embedded mode.

---

## Work Stream 1 — `ServiceHandler` Trait Changes

### 1a. Connection parameter

Every method signature changes from `conn: &mut ControllerConnection` to
`conn: &mut dyn ServiceTransport`:

```rust
// Before
async fn on_connected(&mut self, conn: &mut ControllerConnection, identity: &ServiceIdentityState)
    -> LoopResult<()>;

// After
async fn on_connected(&mut self, conn: &mut dyn ServiceTransport, identity: &ServiceIdentityState)
    -> LoopResult<()>;
```

Applies to: `on_connected`, `on_message`, `on_settings`, `on_service_event`,
`on_surface_action_request`, `on_shutdown`.

`on_surface_action_request` has a default implementation that calls `conn.send(...)`. That
implementation is updated to call `conn.transport_send(...)` and map `TransportError` to
`LoopError::Other(...)`.

### 1b. `on_settings` — agreed capabilities parameter

```rust
// Before
async fn on_settings(&mut self, settings: &ServiceSettingsPayload, conn: &mut ControllerConnection)

// After
async fn on_settings(
    &mut self,
    settings: &ServiceSettingsPayload,
    conn: &mut dyn ServiceTransport,
    agreed_capabilities: &BTreeSet<Capability>,
)
```

`conn.agreed_capabilities()` is not on `ServiceTransport`. The SDK already computes
agreed capabilities before calling `on_settings`; pass them directly. Handlers that
previously read `conn.agreed_capabilities()` read the parameter instead.

### 1c. New method — `on_yield_change`

```rust
async fn on_yield_change(&mut self, _is_yielded: bool, _conn: &mut dyn ServiceTransport) {}
```

Default: no-op (appropriate for most services — messages are simply dropped when yielded).
MQTT overrides to call `runtime.handle_yield_change()` for its reconnect-storm logic.
Called by `run_embedded_service` on yield state transitions.

### 1e. `ServiceTransport::yield_change_notifier` (in `uptrakit-wire`)

`run_embedded_service` needs to await yield state changes. `EmbeddedTransport::yield_change_notifier()`
is currently `pub(crate)` — not accessible via `impl ServiceTransport`. Add a default method to
the `ServiceTransport` trait in `uptrakit-wire`:

```rust
fn yield_change_notifier(&self) -> Option<Arc<tokio::sync::Notify>> {
    None
}
```

`EmbeddedTransport` overrides to return `Some(Arc::clone(&self.yield_state_changed))`.
`ControllerConnection` uses the default `None`. In `run_embedded_service`, this is called once
before the loop to obtain an optional notifier handle.

**Cargo.toml change required:** `uptrakit-wire/Cargo.toml` must add `"sync"` to its tokio
features (`tokio::sync::Notify` is gated behind that feature). This is the only new feature
flag — no new crate dependencies.

**`Option<Arc<Notify>>` in `select!`:** `select!` cannot poll on an `Option<Future>` directly
without either `unwrap` (denied) or a conditional future. The implementation must use
`futures::future::OptionFuture` (or an equivalent) to conditionally await the notifier:

```rust
let yield_notifier = transport.yield_change_notifier();
// In select! arm:
() = async {
    if let Some(n) = &yield_notifier { n.notified().await }
    else { std::future::pending().await }
} => { /* yield change */ }
```

`futures` is already a workspace dependency. This avoids any `unwrap`/`expect` invocation.

### 1d. New variant — `ShutdownCause::EmbeddedDrain`

`ShutdownCause` is a public enum without `#[non_exhaustive]`. Adding `EmbeddedDrain` requires
adding `#[non_exhaustive]` in the same commit (per project standards for extensible public
enums). All exhaustive match sites must then add a wildcard arm with `tracing::warn!` and a
safe fallback. Known sites: `default_resolve_shutdown` in `lifecycle.rs`,
`StandaloneSchedulerHandler::on_shutdown` in `scheduler-runtime/src/standalone.rs` (which has
an exhaustive `ShutdownCause` pattern for selecting `SchedulerStopMode`), and all handler
`on_shutdown` implementations.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShutdownCause {
    Signal(Signal),
    ServerRestarting,
    EmbeddedDrain,
}
```

`default_resolve_shutdown` in `lifecycle.rs` maps `EmbeddedDrain` to
`(DisconnectReason::Shutdown, LoopOutcome::Shutdown)` — same as `Signal(_)`.

`ShutdownCause` remains `Copy`; `EmbeddedDrain` carries no data.

---

## Work Stream 2 — Standalone Event Loop Call Sites

`run_event_loop_connected` keeps `conn: &mut ControllerConnection` internally. The standalone
path still needs typed `recv()` (returns `Result<Option<ControllerMessage>, Report<EnrollmentError>>`),
`set_agreed_capabilities`, `set_report_page_limits`, `close()`, and `close_reason()` — none of
which are on `ServiceTransport`.

At every point where the event loop calls a `ServiceHandler` method, it passes
`conn as &mut dyn ServiceTransport`:

```rust
// event_loop.rs — before
handler.on_connected(conn, identity).await?;

// after
handler.on_connected(conn as &mut dyn ServiceTransport, identity).await?;
```

`process_service_settings` passes agreed capabilities as a new argument:

```rust
handler.on_settings(settings, conn as &mut dyn ServiceTransport, &agreed).await;
```

No other changes to the standalone reconnect, cert renewal, ping, or CA bundle logic.

---

## Work Stream 3 — `run_embedded_service`

New public entry point in `crates/shared/service-sdk/src/embedded.rs`:

```rust
pub async fn run_embedded_service<H: ServiceHandler>(
    mut handler: H,
    mut transport: impl ServiceTransport,
    drain: CancellationToken,
    abort: CancellationToken,
)
```

`CancellationToken` is already a `tokio_util` dep of `service-sdk` (`shutdown.rs` uses it
for `TokenShutdown`). No new crate dependencies.

### Startup sequence

1. **Wait for `ServiceSettings`** — the controller sends this over the embedded channel
   immediately after `EmbeddedServiceHost::add()` provisions the service. The wait is bounded:

   ```rust
   const EMBEDDED_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

   let first_msg = tokio::select! {
       biased;
       () = abort.cancelled() => return,
       result = tokio::time::timeout(EMBEDDED_STARTUP_TIMEOUT, transport.transport_recv()) => {
           match result {
               Err(_elapsed) => {
                   tracing::error!("embedded service did not receive ServiceSettings within 10s; aborting");
                   return;
               }
               Ok(None) => {
                   tracing::error!("embedded transport closed before ServiceSettings arrived");
                   return;
               }
               Ok(Some(msg)) => msg,
           }
       }
   };
   ```

   If the first message is not `ServiceSettings`, log a warning and continue as if no settings
   were received (the controller is misbehaving; return early). If `abort` fires before the
   message arrives, return immediately without calling any handler callback.

2. **Compute agreed capabilities** — intersection of `handler.capabilities()` with
   `settings.capabilities`, filtered to typed (known) variants. Same logic as the standalone
   path in `process_service_settings`.

3. **Call `on_settings(settings, &mut transport, &agreed)`** — first handler callback.
   Handler performs all capability-aware initialization here (surface registration, initial
   state reports, etc.). No `on_connected` call — embedded services are pre-provisioned.

4. **Initial yield notification** — call `handler.on_yield_change(transport.is_yielded(), &mut transport)`.

5. **Enter event loop.**

### Event loop — two-phase select

Phase 1 selects on the cheap arms. Phase 2 runs `handle_event` inside its own select with
drain/abort guards. This prevents a long-running `on_service_event` (e.g. MQTT reconnect storm)
from blocking shutdown signals.

```text
'outer: loop {
    // Phase 1: resolve the next event. poll_service_event arm resolves to a
    // stored `event` local; does NOT call on_service_event here.
    let event: Option<H::ServiceEvent> = select! {
        biased;
        () = abort.cancelled()    => break,          // immediate exit
        () = drain.cancelled()    => { on_shutdown(EmbeddedDrain, timeout).await; break }
        () = yield_arm            => {               // yield_arm = OptionFuture from §1e
            let is_yielded = transport.is_yielded(); // &self read before &mut borrow
            handler.on_yield_change(is_yielded, &mut transport).await;
            continue
        }
        event = handler.poll_service_event() => Some(event),
        msg = transport.transport_recv() => {
            None => break,                           // transport closed
            Some(msg) if transport.is_yielded() => { continue } // drop silently
            Some(msg) => {
                // dispatch: mirrors handle_controller_message routing (see below)
                if let Some(outcome) = dispatch(msg, &mut handler, &mut transport).await {
                    break
                }
                continue
            }
        }
    };

    // Phase 2: run on_service_event with drain/abort guards.
    // event is Some(_) only when the poll_service_event arm fired in Phase 1.
    if let Some(event) = event {
        select! {
            biased;
            () = abort.cancelled()  => break,
            () = drain.cancelled()  => { on_shutdown(EmbeddedDrain, timeout).await; break }
            outcome = handler.on_service_event(event, &mut transport) => {
                if outcome.is_some() { break }
            }
        }
    }
}
```

**Two-phase control flow note:** Phase 1 resolves exactly one event and stores it (or handles
it inline for transport messages and yield changes). Phase 2 only executes when a service event
was resolved in Phase 1. The `event` local bridges the two selects — no borrow-check conflict
because `poll_service_event` completes and releases its `&mut handler` borrow before Phase 2
re-borrows `handler` for `on_service_event`.

**`on_shutdown` during drain:** When the drain token fires, `run_embedded_service` calls
`handler.on_shutdown(conn, ShutdownCause::EmbeddedDrain, timeout)` before returning. Handlers
may attempt to send final messages via `conn.transport_send()` during shutdown. These sends may
fail with `TransportError::Closed` if the controller-side response forwarder has already exited.
Handlers must treat `TransportError::Closed` from `transport_send()` as a silent no-op during
shutdown (not an error). The existing `on_shutdown` convention already discards transport errors
after the first close signal — no new handling is required at call sites; the note is for
handler authors.

**`shutdown_timeout`** starts at `DEFAULT_SHUTDOWN_TIMEOUT` (same constant as the standalone
loop). When a `ServiceSettings` message arrives (initial or re-negotiation), `shutdown_timeout`
is updated from `settings.shutdown_timeout_secs` before calling `on_settings`.

**`large_futures` suppression:** `run_embedded_service` holds `handler: H`, `transport: T`,
two `CancellationToken`s, an `Option<Arc<Notify>>`, and `shutdown_timeout: Duration` across
await points. This state machine will trigger `clippy::large_futures` (workspace-denied). The
function must carry:

```rust
#[expect(clippy::large_futures, reason = "embedded service state machine; per-service allocation is acceptable")]
pub async fn run_embedded_service<H: ServiceHandler>(...) { ... }
```

Per project standards, `#[allow]` is forbidden — `#[expect]` with `reason` is mandatory.

**Message dispatch** mirrors all of `handle_controller_message`'s callback routing — not just
`on_message`. The embedded dispatch must call the same named callbacks as the standalone path:

| Message variant         | Dispatch                                                   |
| ----------------------- | ---------------------------------------------------------- |
| `ServiceSettings`       | Re-negotiate agreed caps, `on_settings`, `on_yield_change` |
| `SurfaceActionRequest`  | `on_surface_action_request`                                |
| `SurfaceActionResponse` | `on_surface_action_response`                               |
| `ServiceConfigAck`      | `on_service_config_ack`                                    |
| `ServerRestarting`      | `on_shutdown(ServerRestarting, timeout)`                   |
| `Unknown`               | `tracing::warn!`, continue                                 |
| Everything else         | `on_message`                                               |

Cert/CA/ping variants (`Certificate`, `CaBundleUpdated`, `Pong`, `RequestCertRenewal`) are no-ops
for embedded — log at `tracing::debug!` and continue. Do not call cert handler logic.

**No cert renewal, no ping timer, no CA bundle handling** — embedded services share the
controller's CA and have no per-service certificates.

**No OS signal handling** — drain/abort tokens replace signals for embedded services.

---

## Work Stream 4 — Controller-Runtime: `ServiceSettings` at Startup

`EmbeddedServiceHost::add()` gains a `service_settings: ServiceSettingsPayload` parameter (or
the relevant fields). After the response forwarder is spawned (step 4 in the existing flow),
before the service closure is called, the controller sends `ServiceSettings` to the embedded
service via the `ctrl_tx` sender:

```rust
ctrl_tx.send(ControllerMessage::ServiceSettings(service_settings)).await?;
```

The `ServiceSettingsPayload` is constructed by the controller with:

- `capabilities`: the full controller-advertised capability set (handler's `capabilities()`
  intersects this when computing agreed caps)
- `tenant_id`: the tenant the embedded service belongs to
- `ping_interval`: use a non-zero sentinel such as `Duration::from_secs(60)` — `Duration::ZERO`
  panics in `tokio::time::interval`. The embedded loop ignores this value; it never sets up a
  ping timer.
- `renewal_window_hours`, `report_page_limits`: zero / default values. Embedded loop ignores
  cert renewal and enforces no page size limits.

---

## Work Stream 5 — Replace Bespoke Embedded Loops

### `ssh_agent/mod.rs`

Delete `run_embedded_ssh_agent`. After the service binary/runtime boundary spec delivers
`AgentSshHandler` in `agent-ssh-runtime`, the controller wires:

```rust
let handler = AgentSshHandler::new(shared_db, ssh_state_dir, false);
run_embedded_service(handler, transport, tokens.drain, tokens.abort).await;
```

The transitional path (before `AgentSshHandler` exists) keeps `run_embedded_ssh_agent` intact;
this work stream is completed as part of the binary/runtime boundary spec, not this one.

### `mqtt/mod.rs`

Delete `run_embedded_mqtt`. `MqttHandler` (from `mqtt-runtime` after its own boundary cleanup)
implements `ServiceHandler` with `on_yield_change` overriding `MqttRuntime::handle_yield_change`.

```rust
let handler = MqttHandler::new();
run_embedded_service(handler, transport, tokens.drain, tokens.abort).await;
```

### Scheduler

Scheduler has no embedded loop today. The scheduler boundary spec delivers
`SchedulerHandler`; when embedded scheduler mode ships it uses `run_embedded_service` directly.

---

## All `ServiceHandler` Implementors — Migration

Every impl updates connection parameter types and `on_settings` signature. The table below
tracks all known sites:

| Crate                                 | Type                         | Notes                                                                                                        |
| ------------------------------------- | ---------------------------- | ------------------------------------------------------------------------------------------------------------ |
| `agent/src/main.rs`                   | `AgentHandler`               | `on_connected` already delegates to `AgentRuntime::on_connected(&mut dyn ST)` — coercion gone, pass directly |
| `mqtt/src/main.rs`                    | `StandaloneMqttHandler`      | Same; add `on_yield_change` override delegating to `runtime.handle_yield_change()`                           |
| `scheduler-runtime/src/standalone.rs` | `StandaloneSchedulerHandler` | Currently uses `ControllerConnection` directly — migrate all methods (see subsection below)                  |
| `agent-ssh/src/main.rs`               | `SshAgentHandler`            | Migrated as part of binary/runtime boundary spec (`AgentSshHandler` in runtime)                              |

### `StandaloneSchedulerHandler` — `conn.send()` migration detail

`StandaloneSchedulerHandler` calls `conn.send(...)` directly in two places. These return
`Result<(), Report<EnrollmentError>>`. After migration, the connection parameter is
`conn: &mut dyn ServiceTransport` and `send` becomes `conn.transport_send(...)`, which returns
`Result<(), TransportError>`. The error type differs; map explicitly:

```rust
// Before
conn.send(ServiceMessage::SomeVariant(...))?;

// After
conn.transport_send(ServiceMessage::SomeVariant(...).into())
    .map_err(|e| LoopError::Other(rootcause::report!(e)))?;
```

Known call sites in `standalone.rs`:

1. **`on_connected`** — sends initial state report or subscription setup message
2. **`drain_service_events`** (called from `on_shutdown`) — drains pending events and sends
   final acknowledgements; may call `conn.send()` inside the drain loop

Both must be updated to `transport_send` with the `LoopError::Other` mapping above. Sending
errors during `on_shutdown` should be logged at `warn!` rather than propagated as errors, since
the connection may already be closing (`TransportError::Closed` is expected).

---

## Changed Files Summary

| File                                                   | Change                                                                                                                                    |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `crates/shared/wire/Cargo.toml`                        | Add `"sync"` to tokio features                                                                                                            |
| `crates/shared/wire/src/transport.rs`                  | Add `yield_change_notifier` default method                                                                                                |
| `crates/shared/service-sdk/src/shared_types.rs`        | `ServiceHandler` trait: all conn params, `on_settings` signature, `on_yield_change`, `ShutdownCause::EmbeddedDrain` + `#[non_exhaustive]` |
| `crates/shared/service-sdk/src/lifecycle.rs`           | `default_resolve_shutdown`: add `EmbeddedDrain` arm                                                                                       |
| `crates/shared/service-sdk/src/event_loop.rs`          | Call sites: `conn as &mut dyn ServiceTransport`                                                                                           |
| `crates/shared/service-sdk/src/lib.rs`                 | Export `run_embedded_service`                                                                                                             |
| `crates/shared/service-sdk/src/embedded.rs`            | New file: `run_embedded_service`                                                                                                          |
| `crates/core/controller-runtime/src/embedded/types.rs` | `EmbeddedTransport`: override `yield_change_notifier`                                                                                     |
| `crates/core/controller-runtime/src/embedded/mod.rs`   | Send `ServiceSettings` to embedded service after forwarder spawn                                                                          |
| `crates/core/agent/src/main.rs`                        | `AgentHandler`: update conn params                                                                                                        |
| `crates/core/mqtt/src/main.rs`                         | `StandaloneMqttHandler`: update conn params, add `on_yield_change` override                                                               |
| `crates/core/scheduler-runtime/src/standalone.rs`      | `StandaloneSchedulerHandler`: update all methods; add `EmbeddedDrain` arm to `ShutdownCause` match                                        |

## Documentation Deliverables

| Deliverable                                              | Action                                                                                                                                                  |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docs/adr/0004-service-handler-transport-abstraction.md` | New ADR: why `dyn ServiceTransport` not `ControllerConnection`; established pattern from `agent-runtime`/`mqtt-runtime`; embedded unification rationale |
| `docs/development/coding-standards.md`                   | Add to "Service binary/runtime boundary" section: `ServiceHandler` implementations must not import `ControllerConnection`; use `dyn ServiceTransport`   |

---

## Quality Gates

- `cargo check --all-features` passes with no `ControllerConnection` in any `ServiceHandler` impl
- `cargo clippy --all-targets --all-features -- -D warnings` clean
- `cargo test --all-features` passes including standalone event loop tests
- New `run_embedded_service` has unit tests covering: settings timeout (transport closes before
  ServiceSettings arrives), normal startup → event loop → drain shutdown, abort during startup,
  yield change dispatch, message drop when yielded
- `StandaloneSchedulerHandler` migration tested via existing scheduler integration tests

---

## Non-Goals

- Changing `ControllerConnection`'s internal implementation
- Making `run_event_loop_connected` generic over transport (internal bookkeeping stays typed)
- Cert renewal, ping, or CA bundle logic for embedded services
- Moving `EmbeddedTransport` or `EmbeddedShutdownTokens` out of `controller-runtime`
- Applying this pattern to plugin crates
