# Typed Dynamic Config Boundaries — Design

## Problem

The codebase currently allows dynamic JSON to leak too far inward.

At the public contract layer, several request/response models expose raw `serde_json::Value` even
when the surrounding API already knows more about the payload shape than “some JSON”. Internally,
some settings paths deserialize database values into `HashMap<String, serde_json::Value>` and then
reconstruct typed domain state by hand with custom getters.

This pattern keeps flexibility, but it weakens Rust’s ability to express invariants and forces the
same shape validation, defaulting, and string conversion work to be repeated manually.

## Covered Findings

- Finding 6: Reduce raw `serde_json::Value` in public request/response contracts.
- Finding 7: Stop hand-parsing settings maps into domain structs.

## Goals

- Push untyped JSON to the outermost boundary only.
- Replace unconstrained raw JSON in public APIs with typed wrappers or tagged/typed payloads where
  the domain already knows the shape.
- Replace internal `HashMap<String, Value>` reconstruction logic with serde-driven typed snapshots.
- Preserve the necessary flexibility for plugin-defined config where the shape is truly dynamic.

## Non-Goals

- No attempt to make all plugin configuration globally static at compile time.
- No breaking redesign of every API endpoint that carries plugin-defined config in one pass.
- No removal of JSON storage from the database where JSON remains the right persistence format.

## Design

### 1. Distinguish “dynamic object” from “arbitrary JSON”

Where APIs genuinely need a plugin- or channel-defined config payload, the boundary should still be
tighter than raw `serde_json::Value`. The design target is a validated wrapper that expresses the
actual contract, such as “JSON object only”, instead of allowing any scalar, array, or null and
re-validating that fact everywhere else.

Where the domain has a finite set of known variants, the design should prefer tagged enums or typed
structs over JSON wrappers.

### 2. Deserialize settings snapshots into typed structs

Internal settings consumers should stop rebuilding domain structs through manual getter families.
The design target is:

1. load raw persisted values
2. map them once into a typed serde-driven snapshot
3. run domain validation/default handling on the typed snapshot
4. pass typed data through the rest of the code

This is especially important for notification and SMTP settings, where defaults, nullability,
secret handling, and enum-like fields are currently reconstructed manually.

Within `crates/plugins/notifications/email/src/surfaces.rs`, this track owns typed settings
snapshots and config wrappers. The earlier plugin API typing track owns typed controller contexts
and typed reusable errors in the same module; this track should layer on top of that boundary
rather than redefine it.

The same typed-snapshot rule applies to the supporting settings plumbing in
`crates/ui/web-api-auth/src/settings_store.rs`, `crates/shared/db/src/raw_settings.rs`, and the
notification dispatcher path that currently reconstructs typed state from dynamic maps.

### 3. Keep dynamic boundaries explicit

This track should define where raw JSON is still acceptable:

- persistence/storage boundaries
- plugin/catalog extension points whose shape is intentionally open
- wire formats that must remain plugin-extensible

Everywhere else should converge toward a typed wrapper or typed model. The design should make that
boundary explicit in crate/module documentation for the affected settings and API layers so future
work does not slide back toward `Value`-everywhere.

### 4. Preserve compatibility intentionally

This track should not assume that every typed-boundary improvement can ship as a silent contract
change. For each touched public API model, the design should choose one explicit compatibility path:

- preserve the existing wire shape while tightening the Rust-side wrapper type
- add a typed replacement alongside the old shape with a deprecation window
- or document a deliberate breaking change if the contract is internal-only and the blast radius is
  understood

The important part is that compatibility is decided per boundary rather than left implicit.

For external REST contracts in scope, the default rule in this first phase is to preserve the
existing wire shape while tightening the Rust-side type. Additive replacement should be used only
when the wire contract itself must grow a new typed form. Deliberate breaking changes are reserved
for internal-only boundaries with understood blast radius, not the named external contracts below.

The first named public contracts in scope should be handled explicitly in the spec rather than left
implicit:

- `CreateNotificationChannelRequest.config`: preserve the existing JSON-object wire shape while
  tightening the Rust-side request wrapper/type around that object boundary.
- `UpdateNotificationChannelRequest.config`: preserve the existing optional JSON-object wire shape
  while tightening the Rust-side update wrapper/type around that object boundary.
- `NotificationChannelResponse.config`: preserve the existing JSON-object response shape in the
  first phase while moving response construction onto a typed Rust-side model.
- `HostPluginRoleAssignment.config_override`: preserve the current override wire shape in the first
  phase while introducing a typed Rust-side wrapper for override semantics.
- `UpdateHostAssignmentRequest.config_override`: preserve the current object-or-null update wire
  shape while introducing a typed Rust-side wrapper for override semantics.
- `HostPluginRoleSummary.config_override`: preserve the current response wire shape in the first
  phase while moving response construction onto a typed Rust-side model.
- `UpdateSoftwareItemRequest.icon_url`: replace the raw JSON patch field with a typed patch wrapper
  while preserving the current absent/null/string wire semantics.
- `SoftwareItemResponse.latest_release_metadata` and
  `SoftwareItemHostSummary.latest_release_metadata`: remain intentionally dynamic in the first phase
  because the payload shape is plugin-defined and not finite at the REST contract level.

## File Map

Primary files expected in scope:

- `crates/shared/web-api-types/src/notifications/channels.rs`
- `crates/shared/web-api-types/src/software_items.rs`
- `crates/plugins/notifications/email/src/surfaces.rs`
- `crates/ui/web-api/src/notifications/dispatcher.rs`
- `crates/ui/web-api-auth/src/settings_store.rs`
- `crates/shared/db/src/raw_settings.rs`

Likely supporting areas:

- shared DB/settings helpers
- controller/web-api query layers that translate between storage and API contracts

## Acceptance Criteria

- Public API models that only require JSON objects stop exposing unconstrained `serde_json::Value`.
- Finite-domain config or patch fields use typed models where the API already knows the shape.
- `UpdateSoftwareItemRequest.icon_url` adopts the typed patch wrapper described in this spec while
  preserving the current absent/null/string wire semantics.
- The named notification and software-item contracts in scope either adopt the first-phase type
  tightening described in this spec or are explicitly documented as intentionally dynamic for the
  current REST boundary.
- Internal settings consumers build typed snapshots through serde-driven deserialization rather than
  manual `HashMap<String, Value>` getter chains.
- The design records which boundaries remain intentionally dynamic and why in crate/module
  documentation for the touched settings and API layers; any cross-cutting summary in
  `docs/development/rust-idioms.md` is optional supporting guidance rather than the sole source of
  truth.
- For each named public contract in scope, the compatibility path is explicit rather than implied:
  preserved wire shape, additive replacement with deprecation, or an explicit internal-only
  breaking change.
- Existing or new validation/serialization coverage proves that the named preserved-shape external
  contracts (`CreateNotificationChannelRequest.config`, `UpdateNotificationChannelRequest.config`,
  `NotificationChannelResponse.config`, `HostPluginRoleAssignment.config_override`,
  `UpdateHostAssignmentRequest.config_override`, `HostPluginRoleSummary.config_override`, and
  `UpdateSoftwareItemRequest.icon_url`) keep their current wire semantics while the Rust-side type
  is tightened.
- `crates/plugins/notifications/email/src/surfaces.rs`,
  `crates/ui/web-api/src/notifications/dispatcher.rs`,
  `crates/ui/web-api-auth/src/settings_store.rs`, and `crates/shared/db/src/raw_settings.rs`
  stop relying on per-field getter families to rebuild typed state from `HashMap<String, Value>`
  maps.

## Recommended Sequencing

This track should land after the plugin API typing track, because the plugin boundary should define
what stays dynamic for extensibility. It can run in parallel with the shared surfaces hardening
track after that first boundary-typing work lands, but it should complete before the broad runtime
decomposition work consumes the new boundary shapes.
