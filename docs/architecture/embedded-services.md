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
| `YieldOnSameAppName` (default) | Yield when an external service with the same `service_app_name` connects. Matches by binary identity, not capability set, so shared capabilities like `GracefulShutdown` never cause false yields. |
| `Custom(f)` | Custom closure — use when additional context (e.g. `machine_id`) is needed beyond `service_app_name`. |
| `NeverYield` | Never yield — always coexist with external services. |

The `service_app_name` used for `YieldOnSameAppName` comparisons is read from
`ServiceConnectionRegistry` when the external service connects. This ensures the value is
set exactly once — in `register()` — for both embedded and external services.

The `EmbeddedServiceNotifier` trait (defined in `web-api`) provides the callback interface:

- `on_external_connected()` -- called when an external service completes WebSocket
  authentication. Sets the `yielded` flag on matching embedded services.
- `on_external_disconnected()` -- called on disconnect. Clears the `yielded` flag.
- `on_machine_id_reported()` -- reserved for custom policies that use `machine_id` matching.
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
    "Embedded Scheduler",                    // label
    "uptrakit-scheduler",                    // app_name (DB lookup key + yield comparison)
    scheduler_caps,                          // BTreeSet<Capability>
    true,                                    // is_system_service
    None,                                    // tenant_id (None for system services)
    CoexistencePolicy::YieldOnSameAppName,   // coexistence policy
    move |transport, tokens| { /* async service closure */ },
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
| Scheduler | `embedded-scheduler` | `YieldOnSameAppName` | Yields when an external `uptrakit-scheduler` connects; internal tasks always run regardless. |
| Agent | `embedded-agent` | `Custom` | Yields when an external `uptrakit-agent` on the same host (matching `machine_id`) connects. Tenant service, not system. |

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

The embedded agent uses `CoexistencePolicy::Custom` with a closure that checks both
`service_app_name == "uptrakit-agent"` and `machine_id`. The embedded agent yields **only**
when an external `uptrakit-agent` on the same physical host connects — requiring both the
correct binary name and a matching machine ID. This allows external agents on other hosts to
coexist without affecting the embedded agent.

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
    types.rs      -- CoexistencePolicy, YieldCheckFn, EmbeddedTransport, ExternalServiceInfo
    bridge.rs     -- run_response_forwarder() (push_rx -> ctrl_tx bridge)
    provision.rs  -- provision_embedded_system_service()

crates/core/controller/src/agent/
    mod.rs        -- (cfg: embedded-agent) Embedded agent using uptrakit-agent-core
```

## API Response Fields

Both `ServiceResponse` and `SystemServiceResponse` expose the embedded state:

| Field | Type | Description |
| --- | --- | --- |
| `is_embedded` | `bool` | `true` for controller-embedded services. |
| `yielded_to` | `Uuid[]?` | External service IDs causing this embedded service to yield. `null` when not yielded or not embedded. Refreshed from `embedded_service_runtime_states` on a 30-second interval. |

### Constraints enforced by the API

- `DELETE` (deactivation) returns `409 CONFLICT` for embedded services.
- `POST .../merge` returns `409 CONFLICT` when either side is embedded.
- Batch deactivate/delete includes embedded services in the `failed` array.

## Related Documentation

- [Scheduler Architecture](scheduler.md) -- scheduler deployment modes and HA mechanism
- [Scheduler Engine (Development)](../development/scheduler-engine.md) -- engine internals
- [System Services](system-services.md) -- system service tier and enrollment
- [Security Architecture](../security/security-architecture.md) -- defense-in-depth
