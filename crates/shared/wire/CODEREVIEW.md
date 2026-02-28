# Code Review: uptrakit-internal-wire

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
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
`Other`/`Unknown` catch-all pattern is applied consistently to `Capability` and `CloseReason`.
Tests cover every message variant, the full sequence-counter state machine, all serde edge cases,
and spec-conformance against `asyncapi.yaml`.

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

**[CRITICAL]** `src/lib.rs:1028-1044` -- `ServiceEnvelope` and `ControllerEnvelope` carry no
`protocol_version` field. Rolling upgrades require a hard cut-over -- there is no mechanism
for the controller to detect that a connected agent is running an older protocol version.
This becomes critical when a new required field is added to an existing payload, a variant is
renamed/removed, or new capability negotiation semantics are introduced. Fix: add
`protocol_version: u32` with `#[serde(default)]` defaulting to 1.

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
- `#[non_exhaustive]` on `ServiceMessage` and `ControllerMessage` correctly forces caller code
  to handle future variants.
- `ServiceSettingsPayload.capabilities` uses `BTreeSet<Capability>` with `#[serde(default)]`.
- `asyncapi.yaml` is versioned alongside the Rust types. Spec-conformance tests prevent drift.

### Issues

**[MEDIUM]** `src/lib.rs:214-262` -- `ServiceMessage` and `ControllerMessage` mix agent and
MQTT concerns without structural separation. When a new `ServiceHandler` author writes a custom
service role, they face a flat enum with no type-level guidance about which variants are
relevant. This is a latent correctness hazard as new service roles are added.

**[LOW]** `src/lib.rs:160-177` -- `HookCommand` enum is not `#[non_exhaustive]`. Nested within
`ExecuteUpdatePayload`, which crosses the wire boundary. Older agents would fail to deserialize
new hook types.
