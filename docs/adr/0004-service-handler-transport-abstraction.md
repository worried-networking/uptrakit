# 0004 — `ServiceHandler` Transport Abstraction

Date: 2026-05-07

## Status

Accepted

## Context

`ServiceHandler` trait methods originally accepted `&mut ControllerConnection` — a concrete
WebSocket-specific type from `uptrakit-service-sdk`. This made it impossible to use the same
`ServiceHandler` implementation in embedded mode (in-process `EmbeddedTransport` channels).
Controller-runtime worked around this by maintaining bespoke event loops
(`run_embedded_ssh_agent`, `run_embedded_mqtt`) that bypassed `ServiceHandler` entirely,
duplicating service lifecycle logic.

## Decision

All `ServiceHandler` method signatures accept `&mut dyn ServiceTransport` instead of
`&mut ControllerConnection`. `ControllerConnection` continues to be used internally by
`run_event_loop_connected` (the standalone path), which passes it as `conn as &mut dyn
ServiceTransport` at each handler call site. A new `run_embedded_service` entry point in
`uptrakit-service-sdk` accepts any `impl ServiceTransport`, enabling both WebSocket and
in-process transports to share the same handler.

`agreed_capabilities` (previously read from `conn.agreed_capabilities()`, a method not on
`ServiceTransport`) is passed directly as a `&BTreeSet<Capability>` parameter to `on_settings`.

## Established Pattern

`agent-runtime` and `mqtt-runtime` already used `&mut dyn ServiceTransport` throughout their
public methods. This ADR formalises the same constraint at the `ServiceHandler` trait level,
making transport-agnosticism enforceable by the compiler: a handler that compiles does not
import `ControllerConnection`.

## Consequences

- Handler implementations gain no dependency on `ControllerConnection` or WebSocket internals.
- `run_embedded_service` enables the service binary/runtime boundary refactor: a single
  `AgentSshHandler`, `MqttHandler`, or `SchedulerHandler` type runs in both standalone and
  embedded modes without bespoke wrappers.
- `StandaloneSchedulerHandler::on_connected` and related methods must map `TransportError` to
  `LoopError::Other` (not via the old `context_to::<LoopError>()` chain that consumed
  `Report<EnrollmentError>`).
- Embedded handlers must inject identity and credentials via their constructors — `on_connected`
  is not called by `run_embedded_service`.
