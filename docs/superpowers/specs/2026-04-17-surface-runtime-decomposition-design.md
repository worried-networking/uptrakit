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

- Finding 1: Split the `SurfaceProxy` transport/idempotency state machine.
- Finding 2: Decompose the SSH surface runtime module.
- Finding 8: Extract a generic batch-command template for package-manager plugins.

## Goals

- Separate runtime orchestration concerns into focused modules with smaller typed interfaces.
- Make local execution and proxied execution paths independently understandable in `SurfaceProxy`.
- Break the SSH surface runtime into modules that reflect domain responsibilities rather than file
  growth history.
- Reduce repeated batch-command orchestration code across package-manager plugins.

## Non-Goals

- No user-visible surface behavior change should be bundled into the decomposition itself.
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

This track owns the structural decomposition of `crates/ui/web-api/src/surface_proxy.rs`. Earlier
typing work may adapt signatures or boundary wiring there, but it should not also claim ownership
of the larger module split.

For APT specifically, the scope includes both the file that owns batch detect orchestration and the
file that owns batch fetch orchestration today. The point of the shared extraction is to cover both
shapes where they are currently implemented, not only one half of the pipeline.

This track deliberately scopes the first shared extraction around the Linux-oriented package-manager
path represented by APT, DNF, pkg, and snap. Other crates such as Homebrew or npm are out of scope
for the first implementation slice unless implementation planning shows they still carry the same
orchestration skeleton and can adopt the helper without distorting the design.

This design addresses Finding 8 by defining a shared orchestration shape that already covers the
two dominant batch patterns in scope here: names-only commands and version-embedded commands. The
first implementation slice proves that shape across APT, DNF, pkg, and snap. Any remaining
package-manager crate is either out of scope because its flow is materially different or a direct
follow-on adoption of the same helper, not a redesign of the helper.

The implementation plan for this track owns the explicit go/no-go decision on any later Homebrew or
npm adoption once the first-slice helper exists.

For DNF, the current scope stays concentrated in `crates/plugins/package-managers/dnf/src/plugin.rs`
because that file owns both batch detect and batch fetch orchestration today.

## File Map

Primary files expected in scope:

- `crates/ui/web-api/src/surface_proxy.rs`
- `crates/core/agent-ssh/src/surface_runtime.rs`
- `crates/plugins/package-managers/dnf/src/plugin.rs`
- `crates/plugins/package-managers/apt/src/detection.rs`
- `crates/plugins/package-managers/apt/src/releases.rs`
- `crates/plugins/package-managers/pkg/src/plugin.rs`
- `crates/plugins/package-managers/snap/src/detection.rs`
- `crates/plugins/package-managers/snap/src/releases.rs`
- `crates/plugins/infrastructure/core/src/helpers.rs`

Likely decomposition targets:

- new submodules under `crates/core/agent-ssh/src/`
- new helper modules under `crates/ui/web-api/src/`
- expanded shared package-manager orchestration helpers

## Acceptance Criteria

- `SurfaceProxy` no longer concentrates transport/state-machine logic in one primary method; the
  implementation is split into dedicated modules, with no single inline method still owning all of
  request resolution, schema and sensitive-field validation, rollout/permission/idempotency gating,
  controller-local execution, provider-proxied execution, and timeout/failure bookkeeping.
- The SSH surface runtime is split into modules that separate registration/builders, dispatch,
  bootstrap, sync, parameter parsing, and controller-proxy helpers, or an equivalent structure with
  the same responsibility boundaries.
- Bootstrap and sync runtime paths use named typed request/argument structs at their internal
  boundaries instead of passing loosely coupled JSON maps end-to-end.
- A shared batch-command orchestration helper exists and is adopted by the APT, DNF, pkg, and snap
  implementations in scope, while package-specific parsing semantics remain local.
- The extracted shared batch-command helper has direct tests covering names-only orchestration,
  version-embedded orchestration, parser-result fan-out, and representative failure mapping.
- The extracted batch-command helper is shaped to cover both names-only and version-embedded
  command flows so that later adoption by remaining matching package-manager crates is an extension
  of the same design rather than a second redesign.
- The decomposition preserves existing surface behavior; any intended behavior change must ship as a
  separate, explicit follow-on change rather than being bundled into the refactor, and existing
  targeted runtime tests in `crates/ui/web-api/src/surface_proxy.rs`,
  `crates/core/agent-ssh/src/surface_runtime.rs`, and the package-manager modules touched in this
  track continue to cover caller-origin and schema/idempotency behavior in `SurfaceProxy`,
  dispatch/bootstrap/sync behavior in the SSH runtime, and per-item result fan-out behavior in the
  package-manager flows after files are split, even if the tests move to new modules.

## Recommended Sequencing

This should be the last of the four tracks. It starts after the plugin API typing track lands and
after the shared-surfaces and typed-config tracks have landed for any interfaces this runtime work
depends on, so the refactor can target clearer boundaries instead of preserving today’s weakest
interfaces.
