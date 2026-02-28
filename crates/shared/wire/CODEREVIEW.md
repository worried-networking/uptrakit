# CODEREVIEW — uptrakit-internal-wire

## Summary

`uptrakit-internal-wire` is the shared wire-protocol library used by every
service in the workspace that communicates over WebSocket: agents, MQTT bridges,
SSH agents, and the controller itself. It defines all message types
(`ServiceMessage`, `ControllerMessage`), the application-level replay-protection
envelope (`ServiceEnvelope`, `ControllerEnvelope`), the capability negotiation
model, the close-reason type, and a suite of serde helpers. The crate ships with
`asyncapi.yaml`, which is the machine-readable source of truth for the protocol.

This is the most carefully engineered crate in the workspace. It is the only
crate with `warnings = "deny"` and `clippy::all = "deny"` enforced at the
`[lints]` level — a standard the rest of the workspace has not yet adopted.
Every public type that callers may match exhaustively carries `#[non_exhaustive]`
and the forward-compatibility `Other`/`Unknown` catch-all pattern is applied
consistently to `Capability` and `CloseReason`. Tests cover every message
variant, the full sequence-counter state machine, all serde edge cases, and
spec-conformance against `asyncapi.yaml`.

One issue requires attention before the next protocol version increment:
`ServiceMessage` and `ControllerMessage` mix agent and MQTT
concerns in a single enum without documented segregation guidance for
`ServiceHandler` implementors. The former `EnrollPayload.service_type` field
has been removed; service identity is now determined entirely by the
`BTreeSet<Capability>` advertised during enrollment. `PluginType` now
correctly uses `Other(String)` for forward-compatibility, matching the pattern
established by `Capability` and `CloseReason`.

---

## Architecture

### Strengths

- **Single-responsibility scope.** The crate contains only wire types, serde
  helpers, sequence counters, and protocol documentation. It imports no
  application logic, no database layer, and no HTTP framework. Its dependency
  tree is minimal: `serde`, `serde_json`, `strum`, `thiserror`, `time`, `uuid`,
  and the workspace's own `uptrakit-shared-types`. `sea-orm`, `tokio`, `axum`,
  and every other heavy dependency are absent.

- **Lint enforcement is the workspace reference implementation.** `Cargo.toml`
  is the only file in all 24 crates that sets `[lints.rust] warnings = "deny"`
  and `[lints.clippy] all = "deny"`. Because this crate's types flow through
  every other crate, the zero-warning discipline here provides a meaningful
  quality floor at the protocol boundary. This configuration should be promoted
  to `[workspace.lints]`.

- **`#[non_exhaustive]` applied correctly throughout.** `Capability`,
  `EnrollmentStatus`, `ErrorCode`, `UpdateFinalStatus`, `DisconnectReason`,
  `ServiceMessage`, and `ControllerMessage` all carry `#[non_exhaustive]`. This
  prevents downstream code from writing exhaustive matches that would silently
  break when new variants are added across agent/controller version skew.

- **`Capability::Other(String)` forward-compatibility pattern.**
  `Capability::from_str` maps unknown strings to `Other(s)` rather than
  returning an error. `is_known()` guards the intersection logic so that
  `Other` variants are never treated as agreed capabilities. A newer peer can
  advertise capabilities an older build does not recognise without triggering a
  connection failure or a silent behavioural change.
  (`src/lib.rs:71`, `src/lib.rs:87–93`, `src/lib.rs:118–131`)

- **`CloseReason::Unknown(String)` mirrors the same pattern.**
  `CloseReason::from_str` in `close_reason.rs` is infallible: unknown close
  strings from a newer controller are preserved and logged rather than
  discarded. The `ParseCloseReasonError` type satisfies the `FromStr` contract
  but is never instantiated in practice, and this is documented explicitly.
  (`src/close_reason.rs:87–105`)

- **Sequence envelope provides application-level replay protection.**
  `ServiceEnvelope` and `ControllerEnvelope` wrap every message with a
  monotonically increasing `seq: u64`. `OutgoingSeq` assigns and increments;
  `IncomingSeq` validates and advances. `SeqError` is typed with `expected` and
  `received` fields, enabling the controller to emit a structured
  `ErrorCode::SequenceError` response.
  (`src/lib.rs:841–941`)

- **`asyncapi.yaml` as machine-readable protocol source of truth.**
  The spec ships inside the crate (`include_str!("../asyncapi.yaml")`). Tests
  parse it at runtime and validate that every serialised message satisfies its
  required-field list, const discriminators, and enum constraints. This closes
  the loop between the Rust types and the documented protocol, preventing silent
  schema drift.

- **`ExecuteUpdate` heap-boxed to limit enum discriminant size.**
  `ControllerMessage::ExecuteUpdate(Box<ExecuteUpdatePayload>)` prevents one
  large payload from inflating the size of every `ControllerMessage` value. The
  struct carries up to two `Vec<HookCommand>` fields plus a `serde_json::Value`,
  so boxing is appropriate and correctly applied.
  (`src/lib.rs:254`)

- **`SecretString` on all credential fields in wire payloads.**
  `EnrollPayload.enrollment_token`, `EnrolledPayload.enrollment_secret`,
  `MqttTenantConfig.username`, `.password`, and `.ca_pem` all use
  `SecretString`. This prevents the values from appearing in `Debug` output or
  log lines and aligns with the rest of the workspace's secret-handling policy.

- **Serde helpers are self-contained and well-documented.**
  `utc_datetime_millis` (private) and `duration_seconds` (public, `pub mod`)
  are small, documented, and use `map_err(serde::ser::Error::custom)` for
  correct error plumbing rather than `unwrap()`. The `duration_seconds` module
  documents its `u32` choice and practical upper bound.

### Issues

**[SEVERITY: Critical]** `src/lib.rs:1028–1044` — `ServiceEnvelope` and `ControllerEnvelope`
carry no protocol-version field; rolling upgrades require a hard cut-over

Neither `ServiceEnvelope` nor `ControllerEnvelope` includes a `protocol_version` field.
When a breaking wire-format change is needed, all agents and the controller must be updated
atomically — there is no mechanism for the controller to detect that a connected agent is
running an older protocol version and handle its messages differently.

This becomes critical in the following scenarios:
- A new required field is added to an existing payload (old agents silently ignore it or
  fail to deserialise)
- A variant is renamed or removed from `ServiceMessage` (old agents produce an
  `ErrorCode::BadRequest` that the new controller cannot contextualise)
- New capability negotiation semantics are introduced

Fix: add a `protocol_version: u32` field to both envelope types (default 1 for backward
compatibility via `#[serde(default)]`). The controller can then reject connections from
agents using incompatible versions with a structured `CloseReason` and a diagnostic log
message that helps operators identify which agents need updating.

**[SEVERITY: Medium]** `src/lib.rs:214–262` — `ServiceMessage` and
`ControllerMessage` mix agent and MQTT concerns in a single monolithic enum

Both enums contain inline comment sections labelling variants as
`// -- Agent-specific --` and `// -- MQTT-specific --`, but the enum
declaration itself carries no structural enforcement. An agent `ServiceHandler`
implementor receives a `ServiceMessage` that can deserialise as `Register`,
`ReleaseTenants`, or `MqttClientStatus` — messages it must never act on. The
controller side has the same problem: `ServiceMessage::DiscoveryResults`
deserialises on an MQTT connection. The handler code in `service_ws.rs` must
mentally classify variants and discard unexpected ones, which is easy to get
wrong silently. The `#[non_exhaustive]` attribute mitigates compile-time
exhaustion but does not communicate to a `ServiceHandler` author which variants
are relevant to their capability set.

Recommended direction: introduce marker types or a sub-enum split
(`AgentServiceMessage`, `MqttServiceMessage`, `SharedServiceMessage`) that can
be re-exported from this crate. The monolithic enum can remain as the wire
representation for backward compatibility, with a `classify()` method returning
the typed variant. This is a design-level change that affects `service_ws.rs`
and `service-sdk`; it warrants a tracked issue before the next protocol version.

---

## Security & Safety

### Strengths

- **Zero `unsafe` blocks in the entire crate.** Neither `lib.rs` nor
  `close_reason.rs` contains any `unsafe` code.

- **No `unwrap()` in production code paths.** The one occurrence of
  `unwrap_or` in the `Deserialize` implementation for `Capability`
  (`lib.rs:142`) is intentional and correct: `s.parse().unwrap_or(Capability::Other(s))`
  is semantically `parse().unwrap_or_else` on an infallible parser, used to
  move `s` into the fallback. This is the idiomatic implementation for this
  pattern.

- **`EnrolledPayload.enrollment_secret` uses `SecretString`.** The enrollment
  secret is the only credential that crosses the unauthenticated WebSocket
  before the service acquires a client certificate. Wrapping it in `SecretString`
  ensures it is redacted in `Debug` formatting and zeroed on drop.

- **Private key material never appears in wire types.** `CertificatePayload`
  contains only `cert_pem`, not a private key. The protocol design requires
  the service to generate its own keypair locally; the test at `lib.rs:1465`
  (`assert!(!json.contains("key_pem"))`) guards against accidental addition of
  a key field.

- **`MqttTenantConfig` credentials transmitted as `SecretString`.** MQTT
  broker username, password, and custom CA PEM are all `Option<SecretString>`
  and are omitted from serialisation when `None`
  (`#[serde(skip_serializing_if = "Option::is_none")]`). This limits credential
  exposure to connections that actually require them.

### Issues

No security issues are directly attributable to this crate. The mTLS
`allow_unauthenticated()` issue identified in the Phase 1 security review
originates in `crates/core/controller/src/pki.rs`, not in wire types, and is
outside this crate's scope.

---

## Code Quality

### Strengths

- **Every public type has a doc comment.** Enums document their wire-format
  string, deprecation intent, and intersection semantics. Payload structs
  document each field. Serde helper modules document their numeric type choice
  and range. The `asyncapi.yaml` lifecycle description at the top of the file is
  the clearest protocol narrative in the workspace.

- **`ParseCapabilityError` is honest about its infallibility.**
  `ParseCapabilityError(std::convert::Infallible)` is defined but
  `Capability::from_str` declares `type Err = std::convert::Infallible`
  directly. The dead struct exists solely to satisfy an older API surface and is
  documented as such. The naming is slightly misleading — callers see
  `Infallible` in the trait signature, not the named error — but the comment
  at `lib.rs:102–106` explains this correctly.

- **`now_millis()` is a simple, self-contained helper.** It uses
  `time::UtcDateTime` consistently with the rest of the workspace's clock
  source, returns `i64` via the public `Timestamp` type alias, and is covered
  by a test that checks a lower bound against 2024-01-01.

- **Backward-compatibility serde tests are explicit and targeted.** Tests such
  as `service_settings_backward_compat_missing_shutdown_timeout`,
  `host_info_deserializes_without_new_fields`, and
  `version_check_result_backward_compat_no_latest_version` document the
  additive-field pattern at the test level, not just as comments. Any future
  field removal will break these tests before it breaks production.

### Issues

**[SEVERITY: Low]** `src/lib.rs:102–116` — Dead `ParseCapabilityError` struct
is not annotated `#[allow(dead_code)]` or removed

`ParseCapabilityError` wraps `std::convert::Infallible` but is never
constructed and never used as an error type — `Capability::from_str` declares
`type Err = std::convert::Infallible` directly. The struct and its `Display`
and `Error` impls are compilation dead code. Because `warnings = "deny"` is in
effect for this crate, this code either passes today because the struct happens
to be `pub` (suppressing the dead-code lint), or it is being retained
intentionally for a future API. Either way, a brief comment explaining why the
type is kept rather than removed would eliminate the ambiguity.
(`src/lib.rs:107–116`)

---

## Tests

### Strengths

- **Comprehensive coverage: every message variant has at least one roundtrip
  test.** Both `ServiceMessage` and `ControllerMessage` have serialisation
  roundtrips, wire-string assertions, and `None`-field omission checks for every
  variant. The test module in `lib.rs` exceeds 2,000 lines and the module in
  `close_reason.rs` adds 96 lines of coverage.

- **Spec-conformance tests against the live `asyncapi.yaml`.** The
  `AsyncApiSpec` helper parses the bundled YAML at test time and validates
  required fields, `const` discriminators, and enum constraints for every
  message shape. At the time of this review, 26+ spec-conformance tests are
  present, one for each message type. This is a strong regression guard for
  protocol schema drift.

- **Sequence-counter state machine fully tested.** `incoming_seq_accepts_sequential`,
  `incoming_seq_rejects_replay`, `incoming_seq_rejects_skip`, and
  `incoming_seq_rejects_zero` form a complete state-machine test for
  `IncomingSeq`. `OutgoingSeq` is tested for monotonic increment across both
  `wrap_service` and `wrap_controller`. `Default` impls for both are tested
  separately.

- **Edge cases in `utc_datetime_millis` serde helper are all covered.**
  Tests for practical range (2024-01-28), Unix epoch, far future (year 9999),
  and pre-epoch negative timestamps ensure the millisecond conversion is correct
  across the full representable range of `UtcDateTime`.

- **`CloseReason` test coverage is exhaustive and structured.** All 11 known
  variants are declared in a `KNOWN_VARIANTS` constant and exercised in four
  tests: `display_produces_wire_strings`, `as_str_matches_display`,
  `from_str_roundtrip_known_variants`, and separate tests for
  `Unknown` passthrough, empty string, equality, and `Clone`.

- **No `unwrap()` in test panics — `expect()` with context used throughout.**
  Every `serde_json::from_str` in tests uses `.unwrap()` in the standard
  test-only pattern, which is explicitly approved by project conventions.

### Issues

**[SEVERITY: Low]** `src/lib.rs:2397–2484` — `AsyncApiSpec::validate` does not
check field types or object/array shapes

The validator checks required fields, `const` discriminators, and `enum` member
constraints, but it does not validate numeric types, string formats, array item
schemas, or nested object shapes. A field present with the wrong JSON type
(e.g. `"seq": "one"` instead of `"seq": 1`) would pass the validator. This is
acceptable at the current complexity level but should be noted as the
validation scope grows. The spec-conformance tests would benefit from
mentioning this scope boundary in a comment at the top of the
`AsyncApiSpec` struct definition.

---

## High Availability

### Strengths

- **`IncomingSeq` provides per-connection replay protection without shared
  state.** Each `IncomingSeq` instance is per-connection and stack-allocated.
  There is no global counter, no lock, and no database access in the hot
  path. A sequence violation immediately surfaces as `SeqError` for the caller
  to handle.

- **`OutgoingSeq` is infallible and non-blocking.** `wrap_service` and
  `wrap_controller` are synchronous, take no locks, and cannot fail. The only
  failure mode (u64 overflow at 2^64 messages) is not a realistic concern.

- **Serde errors on unknown message types are localised to the deserialization
  call site.** `#[non_exhaustive]` on `ServiceMessage` and `ControllerMessage`
  means the controller's handler can reject unknown types with an
  `ErrorCode::BadRequest` response rather than panicking or silently
  ignoring them.

---

## Database

### Strengths

Not applicable. This crate has no database dependency.

### Issues

Not applicable.

---

## Coding Standards

### Strengths

- **Only crate in the workspace enforcing `warnings = "deny"` and
  `clippy::all = "deny"` at the `[lints]` table level.** This means no
  `#[allow(...)]` suppressions are present and the crate compiles cleanly under
  maximum lint scrutiny. It serves as the reference configuration for the
  workspace `[workspace.lints]` table that AGENTS.md calls for.
  (`Cargo.toml:21–25`)

- **`strum::Display` aligns serde wire strings with `Display` output.**
  `EnrollmentStatus`, `ErrorCode`, and `DisconnectReason` all derive both
  `strum::Display` and `serde(rename_all = "snake_case")`. Tests assert
  `code.to_string() == serde_json::to_value(code).unwrap().as_str()` for every
  variant, preventing the two representations from diverging.
  (`src/lib.rs:1959–1988`)

- **`serde_yaml_ng` is a dev-dependency only.** The YAML parser is needed
  exclusively for the `AsyncApiSpec` test helper. It is correctly declared in
  `[dev-dependencies]` and does not increase the production binary's link time
  or dependency surface.
  (`Cargo.toml:19`)

- **No `as_u16()`, no raw status codes, no string error types.** The crate
  uses typed enums (`ErrorCode`, `EnrollmentStatus`) throughout. `String` is
  used only where the wire format is inherently opaque (machine IDs, CSR PEM,
  human-readable messages).

### Issues

**[RESOLVED]** ~~`EnrollPayload.service_type` deprecation is documented in a comment but is not enforced by the compiler~~

The `service_type` field has been removed from `EnrollPayload`. Service identity is now determined entirely by the `BTreeSet<Capability>` advertised during enrollment. The controller infers the service role from the agreed capability set. Enrollment uses a single `register()` call and a single enrollment token.

---

## Extensibility

### Strengths

- **`Capability::Other(String)` with `is_known()` guard is the correct pattern
  for additive protocol extension.** New capabilities can be deployed to the
  controller before all agents are updated. Older agents preserve the unknown
  capability in the set but never include it in the agreed intersection, so
  they simply do not participate in the new capability's behaviour. No
  connection failures, no silent changes.

- **`CloseReason::Unknown(String)` provides forward compatibility in the
  reverse direction.** Older service builds can receive a close frame from a
  newer controller with a reason they do not recognise and handle it gracefully
  (log the string, disconnect cleanly) rather than panicking or ignoring it.

- **`#[non_exhaustive]` on `ServiceMessage` and `ControllerMessage` correctly
  forces caller code to handle future variants.** Adding a new message type to
  either enum is a non-breaking change from the compiler's perspective as long
  as downstream matches use a wildcard arm, which `#[non_exhaustive]` requires.
  The wildcard arms in `service_ws.rs` that send `ErrorCode::BadRequest` on
  unknown message types are the correct handling pattern.

- **`ServiceSettingsPayload.capabilities` uses `BTreeSet<Capability>` with
  `#[serde(default)]`.** Older services that do not send `capabilities` in
  `ReportHosts` receive an empty set by default. Older controllers that do not
  send `capabilities` in `ServiceSettings` produce an empty set by default.
  Both sides of the negotiation degrade gracefully to zero agreed capabilities,
  which is the correct safe default.

- **`asyncapi.yaml` is versioned alongside the Rust types.** Protocol changes
  require updating both the Rust serde annotations and the YAML spec. The
  spec-conformance tests will fail if one is updated without the other, keeping
  the two representations in sync.

#### 2026-02-24 Review

#### Strengths

- **`DisconnectReason` is `#[non_exhaustive]` with documented variants.** `src/lib.rs:637-641` — Correctly omits `Other(String)` because the service controls what it sends.

#### Issues

**[SEVERITY: Low]** `src/lib.rs:160-177` — `HookCommand` enum is not `#[non_exhaustive]`

Nested within `ExecuteUpdatePayload`, which crosses the wire boundary. Older agents would fail to deserialize new hook types.

### Issues

**[SEVERITY: Medium]** `src/lib.rs:214–262` — `ServiceMessage` and
`ControllerMessage` mix agent and MQTT concerns without structural separation,
creating an incomplete trait surface for `ServiceHandler` implementors

(Repeated from Architecture/Issues for emphasis in the extensibility context.)

When a new service role is implemented as a `ServiceHandler`, the developer
faces a flat enum of `ServiceMessage` variants and `ControllerMessage`
variants, with no type-level guidance about which are relevant to a given
capability set. The agent handles `ReportHosts`, `VersionCheckResults`,
`UpdateStarted`, `UpdateOutput`, `UpdateResult`, and `DiscoveryResults`. The
MQTT service handles `Register`, `ReleaseTenants`, and `MqttClientStatus`.
The shared set is `Ping`, `Enroll`, `RequestCertificate`,
`RenewCertificate`, and `Disconnecting`.

A new `ServiceHandler` author writing a custom service role will receive all
variants at their dispatch point and must manually identify which to handle
and which to reject. There is no trait method signature or associated type
that communicates the expected message surface. This is a latent correctness
hazard as new service roles are added.

**[RESOLVED]** ~~`EnrollPayload.service_type` deprecation undocumented to the compiler~~

The `service_type` field has been removed from `EnrollPayload`. Service identity is now determined by the `BTreeSet<Capability>` advertised during enrollment, eliminating the need for the deprecated field.
