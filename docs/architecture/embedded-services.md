# Embedded Services

The controller supports running services inside its own process via in-process mpsc channels.
This enables single-binary deployments (homelab scenarios) where the controller, scheduler,
MQTT bridge, and agents all run in one process.

## Overview

The `EmbeddedServiceHost` (in `crates/core/controller/src/embedded/`) orchestrates embedded
service lifecycle:

1. **Auto-provisioning** -- creates a `system_services` DB record for each embedded service
   on startup (idempotent, keyed by `service_app_name`).
2. **Registry integration** -- registers the embedded service in `ServiceConnectionRegistry`
   with its capability set, making it indistinguishable from an external WebSocket-connected
   service for routing purposes.
3. **In-process transport** -- bidirectional mpsc channels replace the WebSocket framing.
   A response forwarder task bridges the registry push channel to the service's receive
   channel.
4. **Coexistence** -- each embedded service declares a `CoexistencePolicy` that controls
   whether it yields to an external service with overlapping capabilities.

## Architecture

```text
Controller Process
 +------------------------------------------------------------+
 |  ServiceConnectionRegistry                                  |
 |    [external WS] [embedded scheduler] [embedded mqtt] ...  |
 +--------+----------------+------------------+----------------+
          |                |                  |
    WS handler      Response forwarder  Response forwarder
          |           (push_rx -> ctrl_tx)                     |
          |                |                  |
    MessageProcessor  EmbeddedTransport  EmbeddedTransport
          |           (service closure)  (service closure)
 +------------------------------------------------------------+
```

## Coexistence Policy

Each embedded service declares a `CoexistencePolicy`:

| Policy | Behaviour |
| --- | --- |
| `YieldAlways` | Yield when an external service with overlapping capabilities connects. |
| `NeverYield` | Never yield -- coexist with external services. |

An optional custom `yield_check` closure can override the policy for fine-grained control
(e.g., yield only when the external service runs on the same host).

The `EmbeddedServiceNotifier` trait (defined in `web-api`) provides the callback interface:

- `on_external_connected()` -- called when an external service completes WebSocket
  authentication. Sets the `yielded` flag on matching embedded services.
- `on_external_disconnected()` -- called on disconnect. Clears the `yielded` flag.
- `on_machine_id_reported()` -- reserved for future `YieldOnSameHost` policies.
- `is_capability_yielded()` -- queried by embedded services to check their yield state.

The controller stores the host as `Arc<dyn EmbeddedServiceNotifier>` in `AppState`, avoiding
a circular dependency between `web-api` and the controller crate.

## Auto-Provisioning

`provision_embedded_system_service()` creates a `system_services` row with:

- `service_app_name` as the lookup key (idempotent -- reuses existing records).
- `status = Approved` (embedded services are trusted by definition).
- A synthetic `enrollment_secret_hash` (`embedded:{service_id}`) that cannot collide
  with real Argon2id hashes from external enrollments.

## `EmbeddedServiceHost::add()`

The `add()` method takes decomposed parameters (no shared trait required):

```rust
embedded_host.add(
    "Embedded Scheduler",     // label
    "uptrakit-scheduler",     // app_name (DB lookup key)
    scheduler_caps,           // BTreeSet<Capability>
    true,                     // is_system_service
    CoexistencePolicy::YieldAlways,
    None,                     // optional custom yield_check closure
    move |transport, cancel| { /* async service closure */ },
    &app_state,
    &mut bg,
).await?;
```

The method:

1. Provisions the DB record.
2. Registers in `ServiceConnectionRegistry`.
3. Creates bidirectional mpsc channels.
4. Spawns a response forwarder task.
5. Spawns the service closure with an `EmbeddedTransport` handle.
6. Tracks all task handles in `BackgroundTasks`.

## Current Embedded Services

| Service | Feature flag | Coexistence | Notes |
| --- | --- | --- | --- |
| Scheduler | `embedded-scheduler` | `YieldAlways` | Defers external tasks when an external scheduler connects; internal tasks always run. |

## Module Structure

```text
crates/core/controller/src/embedded/
    mod.rs        -- EmbeddedServiceHost, EmbeddedServiceHandle, impl EmbeddedServiceNotifier
    types.rs      -- CoexistencePolicy, EmbeddedTransport, ExternalServiceInfo
    bridge.rs     -- run_response_forwarder() (push_rx -> ctrl_tx bridge)
    provision.rs  -- provision_embedded_system_service()
```

## Related Documentation

- [Scheduler Architecture](scheduler.md) -- scheduler deployment modes and HA mechanism
- [Scheduler Engine (Development)](../development/scheduler-engine.md) -- engine internals
- [System Services](system-services.md) -- system service tier and enrollment
- [Security Architecture](../security/security-architecture.md) -- defense-in-depth
