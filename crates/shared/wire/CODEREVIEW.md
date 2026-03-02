# Code Review: uptrakit-internal-wire

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-internal-wire` is the shared wire-protocol library used by every service that communicates
over WebSocket: agents, MQTT bridges, SSH agents, and the controller. It defines all message types
(`ServiceMessage`, `ControllerMessage`), the application-level replay-protection envelope
(`ServiceEnvelope`, `ControllerEnvelope`), the capability negotiation model, the close-reason type,
and a suite of serde helpers. The crate ships with `asyncapi.yaml`, the machine-readable source of
truth for the protocol.

This is the most carefully engineered crate in the workspace. It is the only crate with
`warnings = "deny"` and `clippy::all = "deny"` enforced at the `[lints]` level. Every public type
that callers may match exhaustively carries `#[non_exhaustive]`, and the forward-compatibility
`Other`/`Unknown` catch-all pattern is applied consistently to `Capability`, `CloseReason`,
`ServiceMessage`, and `ControllerMessage`. The `#[serde(other)] Unknown` variant on both message
enums ensures serde never returns a hard error on an unrecognised `"type"` tag, keeping WebSocket
connections alive across rolling upgrades. Tests cover every message variant, the full
sequence-counter state machine, all serde edge cases (including `Unknown` round-trips), and
spec-conformance against `asyncapi.yaml`.

## Architecture

### Strengths

- `src/lib.rs` -- Single-responsibility scope. Contains only wire types, serde helpers, sequence
  counters, and protocol documentation. No application logic, no database layer, no HTTP
  framework. Minimal dependency tree: `serde`, `serde_json`, `strum`, `thiserror`, `time`,
  `uuid`, and `uptrakit-shared-types`.
- `Cargo.toml:21-25` -- Only crate enforcing `warnings = "deny"` and `clippy::all = "deny"` at
  the `[lints]` table level. Serves as reference configuration for `[workspace.lints]`.
- `src/lib.rs` -- `#[non_exhaustive]` applied throughout: `Capability`, `EnrollmentStatus`,
  `ErrorCode`, `UpdateFinalStatus`, `DisconnectReason`, `ServiceMessage`, `ControllerMessage`.
- `src/lib.rs:71,87-93,118-131` -- `Capability::Other(String)` forward-compatibility pattern.
  `is_known()` guards intersection logic so `Other` variants never treated as agreed
  capabilities.
- `src/close_reason.rs:87-105` -- `CloseReason::Unknown(String)` mirrors the same pattern.
  Infallible `from_str`.
- `src/lib.rs:841-941` -- `ServiceEnvelope` and `ControllerEnvelope` wrap every message with
  monotonically increasing `seq: u64` for application-level replay protection. `SeqError` with
  `expected` and `received` fields.
- `asyncapi.yaml` -- Machine-readable protocol source of truth, shipped inside the crate via
  `include_str!`. Tests parse it at runtime and validate required-field lists, const
  discriminators, and enum constraints.
- `src/lib.rs:254` -- `ExecuteUpdate` heap-boxed to limit enum discriminant size.
- `SecretString` on all credential fields in wire payloads: `EnrollPayload.enrollment_token`,
  `EnrolledPayload.enrollment_secret`, `MqttTenantConfig.username`, `.password`, `.ca_pem`.
- Serde helpers (`utc_datetime_millis`, `duration_seconds`) are self-contained and documented.

### Issues

**[MEDIUM]** `src/lib.rs:214-262` -- `ServiceMessage` and `ControllerMessage` mix agent and MQTT
concerns in a single monolithic enum. The inline comment sections (`// -- Agent-specific --`,
`// -- MQTT-specific --`) carry no structural enforcement. An agent handler receives variants
it must never act on (`Register`, `ReleaseTenants`, `MqttClientStatus`). The controller side
has the same problem. Consider a sub-enum split (`AgentServiceMessage`,
`MqttServiceMessage`, `SharedServiceMessage`) with a `classify()` method.

## Security and Safety

### Strengths

- Zero `unsafe` blocks.
- No `unwrap()` in production code paths. The one `unwrap_or` in `Capability` deserialize
  (`lib.rs:142`) is intentional and correct.
- `EnrolledPayload.enrollment_secret` uses `SecretString`.
- `CertificatePayload` contains only `cert_pem`, never private key. Test at `lib.rs:1465`
  guards against accidental addition of a key field.
- `MqttTenantConfig` credentials are `Option<SecretString>` with
  `#[serde(skip_serializing_if = "Option::is_none")]`.

### Issues

No security issues found.

## Code Quality

### Strengths

- Every public type has a doc comment documenting wire-format string, deprecation intent, and
  intersection semantics where applicable.
- `ParseCapabilityError(std::convert::Infallible)` is defined but `Capability::from_str`
  declares `type Err = std::convert::Infallible` directly. Comment at `lib.rs:102-106`
  explains this.
- `now_millis()` uses `time::UtcDateTime` consistently, returns `i64` via the `Timestamp`
  type alias.
- Backward-compatibility serde tests are explicit:
  `service_settings_backward_compat_missing_shutdown_timeout`,
  `host_info_deserializes_without_new_fields`,
  `version_check_result_backward_compat_no_latest_version`.
- Comprehensive test coverage: every message variant has at least one roundtrip test. Test module
  exceeds 2,000 lines. 26+ spec-conformance tests against `asyncapi.yaml`.
- Sequence-counter state machine fully tested: `incoming_seq_accepts_sequential`,
  `incoming_seq_rejects_replay`, `incoming_seq_rejects_skip`, `incoming_seq_rejects_zero`.
- Edge cases in `utc_datetime_millis` serde helper covered: practical range, Unix epoch, far
  future (year 9999), pre-epoch negative timestamps.
- `CloseReason` test coverage exhaustive and structured: all 11 known variants in
  `KNOWN_VARIANTS` constant, four tests per variant.

### Issues

**[LOW]** `src/lib.rs:102-116` -- Dead `ParseCapabilityError` struct is not annotated
`#[allow(dead_code)]` or removed. The struct wraps `std::convert::Infallible` but is never
constructed. Passes compilation because it is `pub`. Add a comment explaining why the type is
retained.

**[LOW]** `src/lib.rs:2397-2484` -- `AsyncApiSpec::validate` does not check field types or
object/array shapes. A field present with the wrong JSON type (e.g., `"seq": "one"` instead of
`"seq": 1`) would pass the validator. Acceptable at current complexity but should be noted.

## High Availability

### Strengths

- `IncomingSeq` provides per-connection replay protection without shared state. Stack-allocated,
  no global counter, no lock, no database access in the hot path.
- `OutgoingSeq` is infallible and non-blocking. Synchronous, takes no locks, cannot fail.
- Serde errors on unknown message types are localized to the deserialization call site.
  `#[non_exhaustive]` means the controller's handler can reject unknown types with
  `ErrorCode::BadRequest`.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Only crate enforcing `warnings = "deny"` and `clippy::all = "deny"` at the `[lints]` table
  level. Zero `#[allow(...)]` suppressions.
- `strum::Display` aligns serde wire strings with `Display` output. Tests assert
  `code.to_string() == serde_json::to_value(code).unwrap().as_str()` for every variant.
- `serde_yaml_ng` is a dev-dependency only.
- No `as_u16()`, no raw status codes, no string error types.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `Capability::Other(String)` with `is_known()` guard is the correct pattern for additive
  protocol extension. New capabilities deployed to controller before all agents are updated.
- `CloseReason::Unknown(String)` provides forward compatibility in the reverse direction.
- `#[non_exhaustive]` on `ServiceMessage` and `ControllerMessage` forces caller code to handle
  future variants at compile time; `#[serde(other)] Unknown` ensures serde handles unknown `"type"`
  tags at runtime without hard errors, enabling safe rolling upgrades.
- `ServiceSettingsPayload.capabilities` uses `BTreeSet<Capability>` with `#[serde(default)]`.
- `asyncapi.yaml` is versioned alongside the Rust types. Spec-conformance tests prevent drift.

### Issues

**[MEDIUM]** `src/lib.rs:214-262` -- `ServiceMessage` and `ControllerMessage` mix agent and
MQTT concerns without structural separation. When a new `ServiceHandler` author writes a custom
service role, they face a flat enum with no type-level guidance about which variants are
relevant. This is a latent correctness hazard as new service roles are added.

**[LOW]** `src/lib.rs` -- `ErrorCode` has `#[non_exhaustive]` but no `Other` variant. Unknown
error codes from a newer controller cause the entire error message to fall through to
`ControllerMessage::Unknown`, losing the error payload. An `Other(String)` variant would
preserve the error semantics even when the specific code is unrecognised.

**[LOW]** `src/lib.rs` -- `EnrollmentStatus` has the same concern -- no `Other` variant.
A newer controller returning a new enrollment status value will cause the message containing
it to deserialize as `ControllerMessage::Unknown`, silently discarding the enrollment
response.

## Tests

### Strengths

- `src/lib.rs:1287-1808+` -- The test module exceeds 2,000 lines with 50+ tests covering
  every `ServiceMessage` and `ControllerMessage` variant for JSON round-trip serialisation,
  `#[serde(other)] Unknown` handling, backward-compatibility deserialization (missing new
  fields, extra ignored fields), and the serde helper utilities (`utc_datetime_millis`,
  `duration_seconds`) at boundary values (Unix epoch, year 9999, pre-epoch).
- `src/lib.rs` -- `IncomingSeq` / `OutgoingSeq` state machine fully tested: sequential
  acceptance, replay rejection, skip rejection, and zero-seq rejection.
- `src/close_reason.rs:131-205` -- `CloseReason` exhaustive coverage: all 11 known variants
  in the `KNOWN_VARIANTS` constant, four tests per variant (serialise, deserialise, round-trip,
  `Unknown` fallback).
- 26+ spec-conformance tests against `asyncapi.yaml`: required-field lists, const
  discriminators, enum constraints, and field-type checks.
- `CertificatePayload` guard test at `lib.rs:1465` verifies that the struct contains only
  `cert_pem` (no private key field), acting as a permanent regression guard against accidental
  key material exposure.

### Issues

**[LOW]** `src/lib.rs:2397-2484` -- `AsyncApiSpec::validate` does not check field types or
object/array shapes. A field present with the wrong JSON type (e.g., `"seq": "one"` instead of
`"seq": 1`) would pass the spec-conformance validator. The existing validation detects missing
required fields and unknown field names, but type-level constraints are not enforced. This
limits the spec-conformance tests' ability to catch schema regressions in payload structure.

## Consistency

### Strengths

- All duration fields that cross the wire use one of two consistent encoding strategies: a
  bare `u32` with an explicit `_seconds` suffix in the field name, or a `std::time::Duration`
  with `#[serde(with = "duration_seconds")]`. Both strategies produce the same JSON
  representation (an integer number of seconds), and the two serde helpers (`duration_seconds`,
  `utc_datetime_millis`) are shared and documented at `src/lib.rs:559-577`.
- `#[non_exhaustive]` is applied consistently to every extensible enum:
  `Capability`, `EnrollmentStatus`, `ErrorCode`, `UpdateFinalStatus`, `DisconnectReason`,
  `ServiceMessage`, `ControllerMessage`. No public enum in the crate is accidentally exhaustive.
  `#[serde(other)] Unknown` is consistently paired with `#[non_exhaustive]` on both message
  enums, ensuring forward compatibility at both compile time and runtime.
- All credential-bearing fields across every payload type use `SecretString`:
  `EnrollPayload.enrollment_token`, `EnrolledPayload.enrollment_secret`,
  `MqttTenantConfig.username`, `.password`, `.ca_pem`. No credential field is a bare `String`.
  This is enforced uniformly; there is no payload that carries a credential without
  `SecretString`.
- `#[serde(skip_serializing_if = "Option::is_none")]` is consistently applied to all optional
  fields across all payloads. There are no optional fields that serialize as `null` when
  absent; omission is the uniform convention. This is verified by the backward-compatibility
  deserialization tests which confirm that older receivers ignoring unknown fields continue to
  work.

### Issues

**[MEDIUM]** `src/lib.rs:596` (`ServiceSettingsPayload.shutdown_timeout_seconds`) and
`src/lib.rs:748` (`ExecuteUpdatePayload.timeout_seconds`) vs `src/lib.rs:600`
(`ServiceSettingsPayload.ping_interval`) -- Duration fields are encoded inconsistently across
`ServiceSettingsPayload`. `shutdown_timeout_seconds` is a bare `Option<u32>` with the
`_seconds` suffix encoded directly in the field name. `timeout_seconds` in
`ExecuteUpdatePayload` is a bare `u32` with the same suffix convention. By contrast,
`ping_interval` is typed as `std::time::Duration` and relies on `#[serde(with =
"duration_seconds")]` for serialization — with no `_seconds` suffix in the field name. The
on-wire representation is identical (integer seconds), but the Rust API surface is not: callers
of `ping_interval` use `Duration::from_secs(n)` while callers of `shutdown_timeout_seconds`
use a raw `u32`. Both conventions appear within the same struct, creating an inconsistent
authoring experience when adding new duration fields. Preferred pattern: adopt one convention
throughout — either all durations as `u32` with `_seconds` suffix (simpler, no serde helper
needed), or all as `std::time::Duration` with `#[serde(with = "duration_seconds")]` (more
type-safe). The `duration_seconds` module is already well-tested and available; the bare-u32
fields are legacy.

**[LOW]** `src/lib.rs:396-403` (`PingPayload`, `PongPayload`) -- `PingPayload` and
`PongPayload` are empty structs (`pub struct PingPayload {}`, `pub struct PongPayload {}`),
while all other payload types that carry no data are omitted from the enum entirely (the
tag-only variant is handled by `#[serde(tag = "type")]` with an empty payload struct). The
empty structs themselves are fine, but they are not annotated with `#[non_exhaustive]` unlike
every other named payload type that could evolve. If a future protocol version adds a
`PingPayload.timestamp` for round-trip latency measurement, adding that field without
`#[non_exhaustive]` on `PingPayload` would be a breaking API change for any code constructing
the struct with `PingPayload {}`. Preferred fix: add `#[non_exhaustive]` to `PingPayload` and
`PongPayload` consistent with the project's stated policy for extensible public types.

## Maintainability

### Strengths

- Every public type in the production section (lines 1–1266) has a doc comment. Enum variants
  carry their wire-format string, deprecation intent, and intersection semantics. This makes the
  file self-documenting as a protocol reference.
- The test module (lines 1267–3790) is larger than the production code (~2,524 vs ~1,266 lines),
  demonstrating thorough investment in test coverage. The asymmetry is deliberate and appropriate
  for a wire-protocol library where correctness is critical.
- Section headers (`// =====`, `// MQTT Service Specific`, `// Infrastructure Credential`) divide
  the production code into named domains that mirror the `asyncapi.yaml` spec structure.
- `asyncapi.yaml` is versioned alongside the Rust types with spec-conformance tests that fail
  when the spec drifts from the implementation, providing automated documentation correctness.

### Issues

**[HIGH]** `src/lib.rs:1-3790` -- The entire wire protocol library — production types, serde
helpers, sequence counters, and 2,524 lines of tests — lives in a single file. While the
section headers divide it logically, navigating between a payload type definition (e.g.,
`ExecuteUpdatePayload` at line ~720) and its tests (scattered through the 1267–3790 range)
requires manual searching. Splitting into modules (`src/messages.rs`, `src/envelopes.rs`,
`src/serde_helpers.rs`, `src/sequence.rs`) with a `src/lib.rs` that only re-exports public
items would make each concern independently navigable. The tests could move to
`src/messages/tests.rs` etc. This is a pure refactor with no behavioral change.

**[MEDIUM]** `src/lib.rs:266-371` -- `ServiceMessage` and `ControllerMessage` are documented
with inline section comments (`// -- Agent-specific --`, `// -- MQTT-specific --`) but those
comments carry no structural enforcement. A new `ServiceHandler` implementation — say, an
HTTP-bridge service — will see a flat enum containing MQTT variants (`Register`,
`ReleaseTenants`, `MqttClientStatus`) with no compile-time guidance that those variants are
irrelevant to it. As the number of service roles grows, the lack of variant grouping becomes a
maintenance hazard. Already noted in Architecture and Extensibility; included here because the
maintenance cost compounds with each new service role added.

**[LOW]** `src/lib.rs:102-116` -- `ParseCapabilityError(std::convert::Infallible)` is a dead
struct: it wraps `Infallible` and is never constructed, but because it is `pub` it compiles
without a lint warning. The comment at lines 102–106 explains that `Capability::from_str`
declares `type Err = std::convert::Infallible` directly, making the struct redundant. The fix
is either to remove the struct or to annotate it `#[deprecated = "Use std::convert::Infallible
directly"]` to signal intent to future readers.

**[LOW]** `src/lib.rs:978` -- `RequestCrlRenewalPayload {}` is an empty struct with no fields
and no doc comment beyond the type-level doc. Empty payload structs are correct (future fields
can be added without breaking changes due to `#[non_exhaustive]`) but `RequestCrlRenewalPayload`
does not carry `#[non_exhaustive]`. If a future protocol version adds a `reason: String` field
to this payload, callers constructing `RequestCrlRenewalPayload {}` will have a compile error.
Apply `#[non_exhaustive]` for consistency with the stated policy.
