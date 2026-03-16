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
| Agent | `embedded-agent` | `YieldAlways` + custom `yield_check` | Yields only when an external agent with the same `machine_id` connects. Tenant service, not system. |

## Embedded Agent

When the `embedded-agent` feature is enabled, the controller runs a local agent inside its own
process. This eliminates the need for a separate `uptrakit-agent` binary in single-tenant
deployments (homelab, appliance).

### Provisioning

Unlike the embedded scheduler (which is a system service), the embedded agent is provisioned as
a **tenant service** in the `services` table under `AppState.default_tenant_id`. This is because
agent operations (discovery, updates) are tenant-scoped. The feature requires single-tenant mode.

### Transport and message flow

The embedded agent communicates with the controller through `EmbeddedTransport` (in-process mpsc
channels). Messages flow through the same `MessageProcessor` pipeline as WebSocket-connected
services, so the controller applies identical validation, routing, and side effects. The
`ServiceTransport` trait (defined in `uptrakit-internal-wire`) abstracts the transport layer,
allowing `uptrakit-agent-core` to operate identically over both WebSocket and in-process channels.

### Yield behaviour (same-host coexistence)

The embedded agent uses `CoexistencePolicy::YieldAlways` with a custom `yield_check` closure.
The closure compares the external service's `machine_id` (reported via
`on_machine_id_reported()`) against the embedded agent's own `machine_id`. The embedded agent
yields **only** when an external agent on the same physical host connects. This allows external
agents on other hosts to coexist without affecting the embedded agent.

While yielded, the embedded agent stops processing inbound commands (discovery requests, update
execution) and defers to the external agent.

### Update safety

- **Freeze file**: `<state_dir>/embedded-agent/update-freeze` prevents updates when present.
- **Rate limiting**: A 5-second cooldown between accepted `ExecuteUpdate` / `ExecuteBatchUpdate`
  messages prevents runaway update loops.
- **Machine ID validation**: All inbound messages are validated against the embedded agent's
  `machine_id` to prevent cross-service message routing errors.

### Interactive updates

When the controller is compiled with both `embedded-agent` and `interactive`, the embedded agent
supports PTY-based interactive update sessions. The `interactive` feature propagates to
`uptrakit-agent-core` via `uptrakit-agent-core?/interactive`.

### Module location

The embedded agent module lives at `crates/core/controller/src/agent/mod.rs` and reuses all
business logic from `uptrakit-agent-core` (the same crate used by the standalone agent binary).

## Module Structure

```text
crates/core/controller/src/embedded/
    mod.rs        -- EmbeddedServiceHost, EmbeddedServiceHandle, impl EmbeddedServiceNotifier
    types.rs      -- CoexistencePolicy, EmbeddedTransport, ExternalServiceInfo
    bridge.rs     -- run_response_forwarder() (push_rx -> ctrl_tx bridge)
    provision.rs  -- provision_embedded_system_service()

crates/core/controller/src/agent/
    mod.rs        -- (cfg: embedded-agent) Embedded agent using uptrakit-agent-core
```

## Related Documentation

- [Scheduler Architecture](scheduler.md) -- scheduler deployment modes and HA mechanism
- [Scheduler Engine (Development)](../development/scheduler-engine.md) -- engine internals
- [System Services](system-services.md) -- system service tier and enrollment
- [Security Architecture](../security/security-architecture.md) -- defense-in-depth
