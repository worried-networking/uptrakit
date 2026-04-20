# Unified Service Platform Design

## Summary

Uptrakit should move from a mixed architecture of standalone service binaries plus controller-specific embedded service implementations to a single
runtime implementation per service. The controller and the standalone binaries should become hosts for the same service runtimes rather than separate
implementations.

The target services are:

- `agent`
- `agent-ssh`
- `scheduler`
- `mqtt`

The new architecture keeps the existing service protocol model. Embedded services still behave like normal services with `service_id`, capabilities,
settings delivery, config delivery, workload claims, and the same controller message contracts. The difference between standalone and embedded
execution is transport and hosting, not service behavior.

This design intentionally ignores backwards compatibility.

## Goals

- Have one runtime implementation per service.
- Make embedded and standalone execution paths use the same runtime crate.
- Reduce service binaries to thin launchers.
- Reduce the controller to a thin embedded host plus controller-only dependency adapters.
- Keep one logical service protocol model in both standalone and embedded modes.
- Make service yielding a first-class platform feature with declarative policy.
- Make adding a new embeddable service require host wiring, not a second implementation.

## Non-Goals

- Preserve the current split between standalone SDK handlers and controller-only embedded loops.
- Keep `ServiceHandler` as the top-level long-term service abstraction.
- Maintain current feature-gating or crate boundaries where they conflict with the new runtime model.
- Redesign the wire protocol itself.
- Redesign the plugin system or the scheduler engine task model beyond what is required to host them through the unified runtime architecture.

## Current Problems

The current codebase already shares some lifecycle and event-loop plumbing through `uptrakit-service-sdk`, but the architecture is still inconsistent
at the product boundary.

Key problems:

- `agent`, `agent-ssh`, `scheduler`, and `mqtt` expose standalone runtime behavior through SDK-driven handlers, but embedded execution is only
  partially unified.
- The controller still owns custom embedded loops for `agent` and `agent-ssh`.
- The embedded scheduler path is controller-owned composition rather than a shared service runtime.
- `mqtt` has no equivalent embedded product shape even though the target architecture should make any service embeddable.
- Service yielding exists today, but the policy lives in controller-side custom logic instead of a reusable platform contract.
- The top-level abstraction is still too close to the SDK event loop. That is sufficient for callback wiring, but not strong enough to model a full
  service product with resources, startup phases, quiescence, and host integration.

## Architectural Direction

The system should standardize on a service platform architecture:

- A shared service platform defines the long-lived service model.
- Each service moves into its own runtime crate.
- Standalone binaries host those runtimes through a standalone host.
- The controller hosts those same runtimes through an embedded host.
- The existing SDK stays as standalone protocol plumbing, not as the primary architectural boundary.

In short:

- service behavior lives in runtime crates
- execution mechanics live in host/platform crates
- binaries and the controller only wire environments together

## Core Concepts

### Service Definition

Each service exposes static metadata that describes what it is and how it should be hosted.

Required fields:

- service kind
- app name
- capabilities
- scope kind: system or tenant
- yield policy
- runtime factory

This is the declarative identity of the service.

### Service Runtime

Each service has exactly one runtime implementation. A runtime owns:

- startup sequencing
- controller message handling
- service-local event handling
- background task orchestration
- shutdown and drain behavior
- yield quiescence and resume behavior

This runtime is product logic, not transport logic.

### Service Host

A host provides the environment in which a service runtime executes.

There are two concrete host types:

- standalone host
- embedded controller host

The host is responsible for:

- session transport wiring
- identity and service registration
- shutdown signals
- timer and task execution support
- yield detection
- controller-only or process-only dependency injection

The host is not responsible for service business logic.

### Service Context

The host builds a runtime context that contains the information and adapters the runtime needs. This should include:

- service ID
- tenant or system scope
- identity data where applicable
- agreed capabilities
- extension and config proxies
- task spawning helpers
- yield state accessor
- host-specific dependency adapters

### Service Session

Both standalone and embedded execution should expose the same logical session contract to the runtime:

- send service messages
- receive controller messages
- access negotiated capability state
- react to transport close or host shutdown

Standalone mode uses the websocket-backed SDK plumbing under this interface. Embedded mode uses in-process transport. The runtime does not know or
care which one it is using.

## Crate Layout

### New Shared Platform Crate

Add a new crate:

- `crates/shared/service-platform`

This crate owns:

- service definition types
- runtime traits
- host traits
- shared runtime context
- yield policy types
- generic hosting helpers
- common shutdown and quiescence contracts

### Existing SDK Crate

Keep:

- `crates/shared/service-sdk`

But narrow its role to:

- enrollment
- standalone connection management
- standalone lifecycle and reconnect behavior
- websocket protocol event-loop plumbing

It becomes a lower-level implementation detail used by the standalone host path.

### New Runtime Crates

Add one runtime crate per service:

- `crates/core/agent-runtime`
- `crates/core/agent-ssh-runtime`
- `crates/core/scheduler-runtime`
- `crates/core/mqtt-runtime`

These crates own the real service implementations.

### Existing Binary Crates

Keep the existing binaries:

- `crates/core/agent`
- `crates/core/agent-ssh`
- `crates/core/scheduler`
- `crates/core/mqtt`

But reduce them to thin launchers:

- parse CLI
- initialize process-level logging/bootstrap
- construct standalone host configuration
- run the service runtime

### Controller Crate

Keep the controller crate as the embedded host environment, but reduce its responsibilities to:

- embedded host lifecycle
- controller-owned service provisioning and routing
- controller-only dependency adapters
- startup policy for which built-in services are enabled

Controller modules should stop containing alternate service implementations.

## Startup And Lifecycle

All services should follow one host-neutral lifecycle.

### Phase 1: Host Construction

The binary or controller decides which host to construct and with which dependencies.

- standalone host resolves filesystem, enrollment, process signals, and websocket transport
- embedded host resolves controller state, service provisioning, in-process transport, and controller-managed shutdown

### Phase 2: Runtime Construction

The selected `ServiceDefinition` is used to create the runtime instance with a `ServiceContext`.

### Phase 3: Activation

The runtime performs startup in an ordered sequence:

- announce capabilities and registration metadata
- receive and apply service settings
- receive startup-only credentials or config where required
- start service-specific workers only when prerequisites are satisfied

Examples:

- `agent` should not emit its initial report until service settings are available
- `scheduler` should not start the engine until service credentials have been delivered
- `mqtt` should not start broker clients until config has been delivered and claims are known

### Phase 4: Steady State

The runtime processes:

- controller messages
- service-local events
- task completions
- extension/config interactions
- timers and polling where relevant

### Phase 5: Shutdown

All services should follow the same shutdown model:

- drain: stop accepting new work, allow in-flight work to finish if possible
- abort: enforce termination when drain exceeds the configured deadline

The host owns the deadline and enforcement. The runtime owns the logic for graceful quiescence, final status reporting, and resource cleanup.

## Service Yielding

Service yielding must be a first-class platform feature.

### Requirement

Built-in service instances inside the controller are fallback instances. They must disable themselves whenever an external instance of the same
service is running.

### Yield Policies

Each service definition declares a `YieldPolicy`:

- `SameServiceSameHost`
- `SameServiceAnywhere`
- `Never`

The services in scope use:

- `agent`: `SameServiceSameHost`
- `agent-ssh`: `SameServiceAnywhere`
- `scheduler`: `SameServiceAnywhere`
- `mqtt`: `SameServiceAnywhere`

### Detection Rules

For `agent`:

- yield only to an external `uptrakit-agent`
- only when that external instance reports the same host identity
- match is based on service identity plus `machine_id`

For `agent-ssh`, `scheduler`, and `mqtt`:

- yield whenever any external instance of the same service kind is connected

### Runtime Semantics

Yielding means quiescent but resumable, not destroyed.

When a runtime is yielded it must:

- stop accepting new controller work
- stop starting new side-effecting operations
- stop claiming or owning externally visible shared work
- stop periodic work that mutates external state
- keep only enough local state to resume cleanly

The host should notify the runtime explicitly of yield transitions through lifecycle hooks rather than exposing only a raw atomic flag.

Recommended runtime hooks:

- `on_yield_start()`
- `on_yield_stop()`
- `is_yielded()`

### Per-Service Yield Behavior

`agent`

- ignore new controller commands while yielded
- do not start local version checks, discovery, or updates

`agent-ssh`

- ignore controller commands while yielded
- stop periodic host reload scans
- stop starting SSH or PVE actions

`scheduler`

- enter full quiescence while yielded
- do not claim scheduled work while an external scheduler is active

`mqtt`

- stop applying new broker-facing work while yielded
- stop running built-in client management while an external MQTT service is active

## Service Mapping

### Agent Runtime

`agent-runtime` should own:

- local host info collection
- machine ID validation
- update execution state
- discovery/version-check task orchestration
- freeze and rate-limit logic
- interactive update behavior

Both the standalone agent binary and the controller embedded agent should host this same runtime.

### SSH Agent Runtime

`agent-ssh-runtime` should own:

- SSH pool lifecycle
- per-host update orchestration
- host reload and snapshot logic
- extension action handling
- infrastructure plugin integration
- tenant-aware remote-host behavior

Both the standalone SSH agent binary and the controller embedded SSH agent should host this same runtime.

### Scheduler Runtime

`scheduler-runtime` should own:

- service credential handling
- DB/NATS/master-key startup flow
- scheduler engine creation and lifecycle
- scheduler drain and abort semantics

Controller-only scheduler dependencies, such as controller-local notifier adapters or in-process CA rotation triggers, should be injected by the
embedded host instead of living in a second embedded scheduler implementation.

### MQTT Runtime

`mqtt-runtime` should own:

- tenant manager state
- service config delivery and update handling
- workload claim handling
- extension/config proxy flows
- MQTT client manager orchestration

This runtime should be hostable both by the standalone MQTT binary and by the controller embedded host.

## Controller-Specific Responsibilities

The controller should keep only responsibilities that are intrinsically controller-local:

- embedded service provisioning and registration
- service connection registry integration
- built-in service enablement policy
- controller-owned dependency adapters
- controller process startup and shutdown coordination

The controller should not keep separate embedded service business logic once the migration is complete.

## Standalone-Specific Responsibilities

Standalone binaries should keep only process-local responsibilities:

- CLI parsing
- process logging/bootstrap
- directory resolution
- enrollment/bootstrap
- OS signal integration

They should not keep service behavior once the migration is complete.

## Migration Plan

### Phase 1: Create the Platform Layer

Add the `service-platform` crate and define the long-term service abstractions:

- service definition
- service runtime
- service context
- host/session contracts
- yield policies
- shutdown and quiescence hooks

### Phase 2: Extract Runtime Crates

Create the four runtime crates and move service logic out of the current binaries and controller embedded modules.

Target crates:

- `agent-runtime`
- `agent-ssh-runtime`
- `scheduler-runtime`
- `mqtt-runtime`

### Phase 3: Rebuild Standalone Hosting

Refactor each binary crate to become a thin launcher over the new platform and runtime.

### Phase 4: Rebuild Embedded Hosting

Refactor the controller embedded-service infrastructure to host service definitions and service runtimes instead of service-specific controller
closures and custom loops.

### Phase 5: Normalize Yielding

Move all service yielding into the platform layer and express it entirely through declarative `YieldPolicy` plus runtime yield hooks.

### Phase 6: Remove Obsolete Top-Level Abstractions

Once all four services run through the new platform:

- delete the custom embedded `agent` loop
- delete the custom embedded `agent-ssh` loop
- collapse embedded scheduler composition into the unified runtime model
- add embedded `mqtt` hosting
- reduce `ServiceHandler` to an internal bridge or delete it if a cleaner platform-native runtime loop replaces it

## Key Decisions

- One runtime implementation per service is mandatory.
- Embedded services remain normal services in protocol terms.
- Hosts differ only in environment and transport, not in business logic.
- Service yielding is declarative and enforced by the hosting platform.
- Built-in services are fallback instances that yield to external instances according to explicit policy.
- Aggressive crate reshaping is acceptable and expected.

## Risks

- The scheduler and MQTT startup contracts are more phase-heavy than the agent runtimes, so forcing them into the same model must not erase important
  differences in credential/config timing.
- Some controller-local integrations, especially scheduler notifier adapters and extension/config bridges, may tempt the architecture back toward
  controller-owned service logic if not injected cleanly.
- Yield semantics must be explicit at runtime boundaries; otherwise the system will regress into ad hoc `is_yielded()` checks spread through service
  code.

## Acceptance Criteria

The design is complete when:

- each of the four services has one runtime crate that contains its actual behavior
- standalone binaries contain only process/bootstrap wiring
- controller embedded paths contain only host wiring and controller-only adapters
- embedded and standalone execution paths use the same runtime implementation per service
- yielding behavior is platform-defined and matches the required policies
- adding a future built-in service requires creating a runtime and host wiring, not a second service implementation
