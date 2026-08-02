# 0019 — Typed Dynamic Config Boundaries

Date: 2026-06-03

## Status

Accepted

## Context

Raw `serde_json::Value` leaked too far inward across three surfaces:

1. **Public REST DTOs** — `CreateNotificationChannelRequest.config`,
   `HostPluginRoleAssignment.config_override`, and `UpdateSoftwareItemRequest.icon_url` accepted
   arbitrary JSON, forcing repeated shape validation downstream.
2. **Settings consumers** — `settings_map_to_snapshot` in the email plugin used a 12-argument
   hand-rolled getter chain to extract typed fields from a flat `HashMap`, silently weakening Rust's
   invariants at a boundary the codebase already understood well.

Both patterns deferred validation to runtime and offered no compiler guidance on shape.

## Decision

### Public REST DTOs

Object-only config fields (`CreateNotificationChannelRequest.config`,
`HostPluginRoleAssignment.config_override`) use typed wrappers — `JsonObjectMap` (owned) and
`JsonObjectInput` (request-side) — that enforce object-shape at the serde boundary. Two distinct
newtypes share a private `parse_json_object` parser; each preserves its `ValidationError.field`
independently so the API wire contract is unchanged. `#[non_exhaustive]` is applied to both wrapper
structs as they are extensible. Finite-state patch fields (`icon_url`, plugin config overrides) use
typed enums (`IconUrlPatch`, `JsonObjectMapPatch`) with custom `Deserialize` that covers the
absent / null / value wire semantics. These enums are explicitly NOT `#[non_exhaustive]` because
external crates construct their `Set(...)` tuple variant and must exhaustively match them.

### Settings consumers

`decode_prefixed_settings` (in `uptrakit-shared-db`) is the canonical way to convert a
prefix-scoped settings map into a typed Rust snapshot via serde. The email plugin's
`settings_map_to_snapshot` getter chain is replaced by a `decode_prefixed_settings` call onto
`SmtpNonSecretSnapshot`, followed by a separate decryption step for `password` (which cannot cross
the serde boundary as `SecretString` ciphertext).

### Intentionally dynamic remainder

`latest_release_metadata` and the Telegram dispatcher bag remain `serde_json::Value` — their
shapes are plugin-defined and cannot be typed at the shared-settings layer.

## Alternatives Considered

### 1. Single `JsonObject` newtype for all object fields

A single newtype would eliminate the two-type split but lose per-call-site `ValidationError.field`
discrimination. Rejected: `ValidationError.field` is part of the observable API contract; merging
call sites would require a follow-up wire change.

### 2. Typed snapshot at the dispatcher layer

`build_settings_bag` could emit typed snapshots instead of a flat keyed bag. Rejected for now
because `smtp_from_settings_map` in the email plugin reads via `format!("{prefix}{suffix}")` key
lookups; changing that cross-crate contract is scoped to a follow-up (see Explicit Deferral below).

## Consequences

- Stronger Rust invariants at three previously weakened surfaces; eliminates the 12-arg getter
  family in the email plugin.
- `decode_prefixed_settings` is now the canonical pattern for prefix-scoped settings snapshots;
  plugin crates should not hand-roll field-by-field getters.
- `#[non_exhaustive]` applied only to extensible wrapper structs (`JsonObjectMap`,
  `JsonObjectInput`); explicitly omitted from closed sum types (`IconUrlPatch`,
  `JsonObjectMapPatch`) so external crates can construct and exhaustively match them.
- Per-call-site `ValidationError.field` preserved; no observable wire change for API clients.

## Explicit Deferral

`notification_settings.rs::build_settings_bag` still produces a flat `smtp.*`-keyed JSON bag
because `smtp_from_settings_map` in the email plugin reads via `format!("{prefix}{suffix}")`
lookups. The spec acceptance criterion naming the dispatcher path as a typed-snapshot target is
therefore only partially satisfied: the email plugin's internal getter chain is removed, but the
cross-crate dispatcher → plugin bag stays flat. Follow-up track: **typed dispatcher bag** — change
`smtp_from_settings_map` to consume `SmtpSettingsSnapshot` directly, allowing `build_settings_bag`
to emit typed snapshots.

## References

- Spec: `docs/superpowers/specs/2026-04-17-typed-dynamic-config-boundaries-design.md`
- Plan: `docs/superpowers/plans/2026-04-17-typed-dynamic-config-boundaries.md`
- Related: `0018-plugin-extension-typed-boundary.md` — typed plugin extension boundary this builds on
