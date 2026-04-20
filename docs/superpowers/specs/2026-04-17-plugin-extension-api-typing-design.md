# Plugin Extension API Typing — Design

## Problem

The plugin extension boundary is currently weaker than the rest of the Rust codebase.

Controller-facing plugin contexts expose the database through `dyn Any`, which forces plugin code to downcast at runtime and hides what capabilities
the host is intentionally offering. At the same time, reusable extension traits and config validation hooks still lean on `Result<_, String>`, which
erases structure from failures and encourages ad hoc formatting instead of stable, typed error evolution.

Those two design choices make the extension layer harder to understand, test, and change safely. The plugin boundary should be one of the strongest
typed seams in the system, not one of the loosest.

## Covered Findings

- Finding 4: Remove `dyn Any` from plugin-facing controller contexts.
- Finding 5: Replace `Result<_, String>` in reusable plugin APIs.

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

Plugin surface actions and protection hooks should receive typed controller-facing context objects. The context should expose only the capabilities
that plugins are allowed to use, for example:

- tenant/user identity
- controller-side persistence operations required by that plugin boundary
- config/report emission hooks
- audit/event helpers where applicable

The design should avoid simply swapping `dyn Any` for “one giant controller trait”. The target is a small set of capability-focused traits or typed
adapters that make permissions and dependencies obvious.

The V1 controller-side boundary should cover, at minimum:

- tenant and caller identity
- persistence operations required by surface-action and protection hooks
- config/report emission operations already exposed through the plugin boundary
- audit/event emission hooks where a plugin-facing action already needs them

When a future plugin needs more controller capabilities, the extension path should be additive: introduce a new narrow trait or adapter and compose it
into the boundary, rather than widening one general-purpose context object.

This keeps the controller in control of its boundary while letting plugin code compile against an explicit contract.

### 2. Replace stringly reusable errors with typed enums

Reusable traits such as plugin config validation and plugin surface action handling should stop using `String` as their primary error type. The design
target is typed error enums for the reusable layers, with `Display` output reserved for the final presentation boundary.

For this track, the reusable plugin APIs in scope are:

- `PluginConfig` and its validation/config-schema-facing error surface
- controller-side plugin action/protection boundaries in `roles.rs`
- controller-side plugin operation wiring in `plugin_ops.rs`

Agent-only role internals and plugin-private helper functions are out of scope unless they are surfaced through one of those reusable boundaries.

The exact split can vary, but the design should distinguish at least:

- configuration validation failures
- action input/contract failures
- controller integration failures
- plugin-internal unexpected failures

The goal is not to eliminate human-readable messages. It is to stop using free-form strings as the only contract between reusable layers.

### 3. Define a migration shape that preserves momentum

This track should include a migration strategy that allows the catalog and plugin implementations to move incrementally:

- add typed abstractions first
- adapt catalog/controller call sites
- migrate plugin implementations in batches
- keep string conversion only at the outer boundary until all call paths are typed

That sequencing matters because these APIs are shared by many crates.

The first explicit plugin migration wave should be named in the spec rather than left generic:

- `uptrakit-plugin-infrastructure-proxmox` for controller-side surface actions and controller update protection
- `uptrakit-notification-plugin-email` for controller-side notification surface actions
- `uptrakit-notification-plugin-telegram` for controller-side notification surface actions
- `uptrakit-notification-plugin-webhook` for controller-side notification surface actions
- `uptrakit-plugin-releases-docker` for controller-side surface actions

Controller-side fetch-only plugins that do not currently exercise the same surface/protection boundary shape, such as GitHub releases, are outside
this first migration wave unless implementation planning shows that they depend on the same reusable typed boundary.

## File Map

Primary files expected in scope:

- `crates/plugins/infrastructure/core/src/descriptor.rs`
- `crates/plugins/infrastructure/core/src/roles.rs`
- `crates/plugins/infrastructure/core/src/plugin_config.rs`
- `crates/plugins/infrastructure/core/src/plugin_ops.rs`

Likely downstream touch points:

- `crates/ui/web-api/src/surface_proxy.rs`
- `crates/plugins/notifications/email/src/surfaces.rs`
- controller-side plugin action/protection call sites
- plugin crates implementing controller-side surface or protection hooks

This track may require adapting `surface_proxy.rs` call signatures or boundary wiring, but it does not own the structural decomposition of that file.
The later runtime-decomposition track owns the module split and state-machine refactor there.

The first migration wave, including the Proxmox plugin, should not wait on or assume that later `surface_proxy.rs` decomposition work has already
happened. This track owns typed boundary adoption; the later runtime track owns structural runtime churn.

For `crates/plugins/notifications/email/src/surfaces.rs`, this track owns typed controller-context and reusable error adoption. The typed-config track
may touch the same file later for settings snapshot/config-wrapper cleanup, but only after this boundary work has landed.

## Acceptance Criteria

- Plugin-facing controller contexts in the named core boundary files no longer require runtime downcasts from `dyn Any`.
- Reusable plugin APIs in scope, including `PluginConfig` validation paths and the controller-side plugin action/protection boundaries in `roles.rs`
  and `plugin_ops.rs`, expose typed error contracts instead of `Result<_, String>`.
- Finding 5 completion for this track is explicitly limited to `PluginConfig`, its validation/config-schema-facing error surface, the controller-side
  plugin action/protection boundaries in `roles.rs`, and the controller-side operation wiring in `plugin_ops.rs`; agent-only role internals and
  plugin-private helpers are excluded unless they surface through those boundaries.
- Reusable plugin boundaries return typed failures, and conversion into user-facing text happens only at the outer controller/web-api error-mapping
  layer rather than inside the reusable plugin traits themselves.
- No single catch-all controller trait replaces `dyn Any`; the exported boundary is composed from narrow capability traits or typed adapters.
- The first migration wave (`uptrakit-plugin-infrastructure-proxmox`, `uptrakit-notification-plugin-email`, `uptrakit-notification-plugin-telegram`,
  `uptrakit-notification-plugin-webhook`, and `uptrakit-plugin-releases-docker`) compiles against the typed context/error boundary without relying on
  `dyn Any` or `String` as the reusable contract.
- Exported plugin-facing controller boundary signatures in the named core crates describe required capabilities through typed contexts or narrow
  traits rather than erased controller objects.

## Recommended Sequencing

This is the first track that should land. It defines the typed boundary shape that the config and runtime tracks should target rather than work
around.
