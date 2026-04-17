# Surface Runtime Decomposition — Design

## Problem

Two runtime-heavy areas have grown into orchestration hubs:

- `SurfaceProxy::invoke_inner()` in `web-api`
- `surface_runtime.rs` in `agent-ssh`

Both now mix multiple concerns that should evolve independently: lookup, validation, permission and
rollout checks, idempotency, local/proxied transport handling, timeout behavior, JSON parameter
parsing, background task spawning, and workflow execution.

In parallel, the package-manager crates repeat the same batch-command orchestration pattern over and
over. That duplication increases maintenance cost whenever runtime behavior needs to change
consistently across plugins.

## Covered Findings

- Split the `SurfaceProxy` transport/idempotency state machine.
- Decompose the SSH surface runtime module.
- Extract a generic batch-command template for package-manager plugins.

## Goals

- Separate runtime orchestration concerns into focused modules with smaller typed interfaces.
- Make local execution and proxied execution paths independently understandable in `SurfaceProxy`.
- Break the SSH surface runtime into modules that reflect domain responsibilities rather than file
  growth history.
- Reduce repeated batch-command orchestration code across package-manager plugins.

## Non-Goals

- No user-visible surface behavior change as part of the decomposition itself.
- No redesign of the shared surface contract model; that belongs to the shared surfaces track.
- No attempt to unify every runtime module in the repository under one abstraction.

## Design

### 1. Decompose `SurfaceProxy` by responsibility

The design target for `SurfaceProxy` is a coordinator that delegates to smaller units instead of
implementing the entire state machine inline. The important boundaries are:

- request resolution and caller-origin normalization
- schema and sensitive-field validation
- rollout/permission/idempotency gating
- local controller-owned execution
- provider-proxied execution
- timeout/failure bookkeeping and cached response handling

The end state should make it obvious which logic is shared between both execution modes and which
logic is transport-specific.

### 2. Decompose the SSH surface runtime into domain modules

`agent-ssh` surface runtime code should be reorganized around stable responsibilities such as:

- registration/builders
- request dispatch
- parameter parsing and typed request models
- bootstrap workflow
- sync workflow
- controller-proxy helpers

The design should replace “JSON map passed everywhere” with smaller typed request/argument structs
inside the runtime modules where practical.

### 3. Extract shared batch-command orchestration

The APT/DNF/pkg/snap-style batch patterns should move toward a reusable template that separates:

- identifier/version validation
- argument building
- single-command execution
- parser-specific output interpretation
- per-item result fan-out

This should not force different package managers into an unnatural common parser. The shared layer
should own orchestration, not package-specific semantics.

## File Map

Primary files expected in scope:

- `crates/ui/web-api/src/surface_proxy.rs`
- `crates/core/agent-ssh/src/surface_runtime.rs`
- `crates/plugins/package-managers/dnf/src/plugin.rs`
- `crates/plugins/package-managers/apt/src/detection.rs`
- `crates/plugins/package-managers/apt/src/releases.rs`
- corresponding shared helper modules in plugin infrastructure core

Likely decomposition targets:

- new submodules under `crates/core/agent-ssh/src/`
- new helper modules under `crates/ui/web-api/src/`
- expanded shared package-manager orchestration helpers

## Acceptance Criteria

- `SurfaceProxy` no longer concentrates all transport/state-machine logic in one primary method.
- The SSH surface runtime is split into modules that each have one clear job.
- Internal runtime code uses more typed request/argument structs and fewer loosely coupled JSON maps.
- Package-manager batch orchestration is shared where the control flow is the same, while package
  specifics remain local.
- The decomposition reduces future change risk without changing surface behavior.

## Recommended Sequencing

This should be the last of the four tracks. It benefits from the earlier typing and contract work
so that the runtime refactor can target clearer boundaries instead of preserving today’s weakest
interfaces.
