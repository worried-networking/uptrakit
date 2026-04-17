# Plugin Extension API Typing — Design

## Problem

The plugin extension boundary is currently weaker than the rest of the Rust codebase.

Controller-facing plugin contexts expose the database through `dyn Any`, which forces plugin code
to downcast at runtime and hides what capabilities the host is intentionally offering. At the same
time, reusable extension traits and config validation hooks still lean on `Result<_, String>`,
which erases structure from failures and encourages ad hoc formatting instead of stable, typed
error evolution.

Those two design choices make the extension layer harder to understand, test, and change safely.
The plugin boundary should be one of the strongest typed seams in the system, not one of the
loosest.

## Covered Findings

- Remove `dyn Any` from plugin-facing controller contexts.
- Replace `Result<_, String>` in reusable plugin APIs.

## Goals

- Replace runtime downcasts with explicit typed host capabilities at the plugin boundary.
- Introduce typed error contracts for reusable plugin APIs.
- Preserve plugin extensibility without exposing internal controller implementation details.
- Keep the wire/UI layer free to render user-facing error strings at the last possible boundary.

## Non-Goals

- No removal of all dynamic dispatch from the plugin system.
- No forced migration of every plugin implementation in one unstructured sweep.
- No exposure of raw SeaORM internals as the permanent plugin abstraction.
- No redesign of the plugin catalog’s high-level responsibilities.

## Design

### 1. Replace `dyn Any` with narrow controller-side capability traits

Plugin surface actions and protection hooks should receive typed controller-facing context objects.
The context should expose only the capabilities that plugins are allowed to use, for example:

- tenant/user identity
- controller-side persistence operations required by that plugin boundary
- config/report emission hooks
- audit/event helpers where applicable

The design should avoid simply swapping `dyn Any` for “one giant controller trait”. The target is
a small set of capability-focused traits or typed adapters that make permissions and dependencies
obvious.

This keeps the controller in control of its boundary while letting plugin code compile against an
explicit contract.

### 2. Replace stringly reusable errors with typed enums

Reusable traits such as plugin config validation and plugin surface action handling should stop
using `String` as their primary error type. The design target is typed error enums for the reusable
layers, with `Display` output reserved for the final presentation boundary.

The exact split can vary, but the design should distinguish at least:

- configuration validation failures
- action input/contract failures
- controller integration failures
- plugin-internal unexpected failures

The goal is not to eliminate human-readable messages. It is to stop using free-form strings as the
only contract between reusable layers.

### 3. Define a migration shape that preserves momentum

This track should include a migration strategy that allows the catalog and plugin implementations to
move incrementally:

- add typed abstractions first
- adapt catalog/controller call sites
- migrate plugin implementations in batches
- keep string conversion only at the outer boundary until all call paths are typed

That sequencing matters because these APIs are shared by many crates.

## File Map

Primary files expected in scope:

- `crates/plugins/infrastructure/core/src/descriptor.rs`
- `crates/plugins/infrastructure/core/src/roles.rs`
- `crates/plugins/infrastructure/core/src/plugin_config.rs`
- `crates/plugins/infrastructure/core/src/plugin_ops.rs`
- `crates/ui/web-api/src/surface_proxy.rs`

Likely downstream touch points:

- controller-side plugin action/protection call sites
- plugin crates implementing controller-side surface or protection hooks

## Acceptance Criteria

- Plugin-facing controller contexts no longer require runtime downcasts from `dyn Any` for normal
  operation.
- Reusable plugin APIs expose typed error contracts instead of `Result<_, String>`.
- The presentation layer still has a clear place to convert typed failures into user-facing text.
- The migration path is incremental and identifies how existing plugins adopt the new boundary.
- The new abstractions make plugin dependencies more explicit than the current design.

## Recommended Sequencing

This is the first track that should land. It defines the typed boundary shape that the config and
runtime tracks should target rather than work around.
