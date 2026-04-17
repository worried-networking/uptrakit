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

- Reduce raw `serde_json::Value` in public request/response contracts.
- Stop hand-parsing settings maps into domain structs.

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

### 3. Keep dynamic boundaries explicit

This track should define where raw JSON is still acceptable:

- persistence/storage boundaries
- plugin/catalog extension points whose shape is intentionally open
- wire formats that must remain plugin-extensible

Everywhere else should converge toward a typed wrapper or typed model. The design should make that
boundary explicit so future work does not slide back toward `Value`-everywhere.

## File Map

Primary files expected in scope:

- `crates/shared/web-api-types/src/notifications/channels.rs`
- `crates/shared/web-api-types/src/software_items.rs`
- `crates/plugins/notifications/email/src/surfaces.rs`
- related notification/settings types that currently expose or rebuild raw JSON

Likely supporting areas:

- shared DB/settings helpers
- controller/web-api query layers that translate between storage and API contracts

## Acceptance Criteria

- Public API models that only require JSON objects stop exposing unconstrained `serde_json::Value`.
- Finite-domain config or patch fields use typed models where the API already knows the shape.
- Internal settings consumers build typed snapshots through serde-driven deserialization rather than
  manual `HashMap<String, Value>` getter chains.
- The design explicitly records which boundaries remain intentionally dynamic and why.
- Notification/settings logic becomes easier to validate and extend without duplicating parsing code.

## Recommended Sequencing

This track should land after the plugin API typing track, because the plugin boundary should define
what stays dynamic for extensibility. It can proceed in parallel with parts of the shared surfaces
track, but should complete before the broad runtime decomposition work consumes the new boundary
shapes.
