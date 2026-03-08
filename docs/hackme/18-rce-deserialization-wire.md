# ATK-18: RCE via Wire Protocol Deserialization

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | Wire protocol / serde |
| Prerequisites | Ability to send WebSocket messages to the controller (mTLS or anonymous) |
| STRIDE | Tampering, Denial of Service |

## Attack description

### Deserialization-based attacks

1. The attacker establishes a WebSocket connection to the controller (anonymous for
   enrollment, or mTLS for authenticated sessions).
2. The attacker sends crafted JSON messages designed to exploit the deserialization
   layer:
   - **Memory exhaustion.** A message close to the 1 MB limit containing deeply
     nested JSON objects (up to serde_json's 128-level recursion limit) or a single
     field with a near-1 MB string allocation.
   - **CPU exhaustion.** Messages with many small fields that trigger O(N^2)
     allocation patterns in serde's flattened struct deserialization.
   - **Duplicate key confusion.** JSON with duplicate `"type"` keys where
     `serde_json` uses last-value-wins semantics, potentially causing a valid-looking
     message to be parsed as `Unknown` while its payload fields were already
     processed.
   - **Large `Vec` allocations.** Fields like `ReportHostsPayload.hosts`
     (`Vec<HostInfo>`) or `CheckVersionsPayload.assignments`
     (`Vec<VersionCheckAssignment>`) with no count limits, filling the 1 MB budget
     with thousands of small entries that trigger per-element allocations.

### Type confusion via `#[serde(flatten)]`

1. The attacker sends a JSON message with fields that exploit the interaction between
   `#[serde(tag = "type")]` and `#[serde(flatten)]` on envelope structs.
2. In some serde_json versions, the order of keys in flattened tagged enums can affect
   which variant is selected. A carefully positioned duplicate `"type"` key could
   cause the envelope to parse differently than expected.

### Forward compatibility abuse

1. The attacker sends messages with unrecognized `"type"` values that deserialize to
   `ServiceMessage::Unknown` via `#[serde(other)]`.
2. The `seq` counter is still consumed for `Unknown` messages, keeping the connection
   alive. The attacker floods `Unknown` messages to consume server-side resources
   (memory for connection state, CPU for JSON parsing) without triggering error
   handling.

## Worst-case impact

- **Denial of service.** Memory exhaustion or CPU saturation on the controller from
  crafted messages, potentially affecting all connected agents.
- **Connection state corruption.** Type confusion or duplicate-key exploitation could
  cause the controller to misinterpret a message, potentially processing it as a
  different variant than intended.
- **Resource exhaustion via anonymous connections.** An attacker without mTLS can
  establish anonymous WebSocket connections (for enrollment) and send crafted messages
  during the 30-second anonymous timeout window.

Note: **Remote code execution via deserialization is extremely unlikely.** Rust's
serde framework does not support arbitrary code execution through deserialization
(unlike Java's `ObjectInputStream` or Python's `pickle`). There are no known serde
gadget chains that lead to code execution. The risk is primarily denial of service
and data integrity.

## Current mitigations

- **JSON-only serialization.** The wire protocol uses `serde_json` exclusively. JSON
  deserialization in Rust does not have the gadget chain vulnerabilities present in
  binary serialization formats like Java's `ObjectInputStream`.
- **1 MB WebSocket message limit.** `MAX_WS_MESSAGE_SIZE = 1_048_576` is enforced at
  the transport layer before any JSON parsing, bounding the maximum memory allocation
  from a single message.
- **serde_json recursion limit.** The default recursion depth limit of 128 levels
  prevents stack overflow from deeply nested JSON.
- **Message rate limiting.** 50 messages per second per connection, 30 connections per
  60 seconds per IP, and 10 auth failures per 300 seconds per IP.
- **30-second anonymous timeout.** Anonymous connections that do not send `Enroll`
  within 30 seconds are automatically closed.
- **Sequence validation.** Every message validates the `seq` field. Out-of-order or
  replayed sequences close the connection immediately.
- **Protocol version validation.** Messages with unexpected `protocol_version` trigger
  connection termination before full deserialization.
- **Two-pass deserialization.** The controller first parses the `EnvelopeHeader`
  (protocol version + seq) before attempting full message deserialization. This
  allows early rejection of malformed messages without parsing the payload.
- **Strongly typed structs.** All message payloads are deserialized into fixed Rust
  structs with known field types. There are no `serde_json::Value` fields in the
  message envelope layer (though `PluginAssignment.config` uses `serde_json::Value`).
- **`Unknown` variant handling.** Unrecognized message types are deserialized to
  `Unknown` (no payload materialized), logged at `warn` level, and the event loop
  continues without processing.
- **Per-field and per-collection size limits (`WireValidate`).** *(Implemented)*
  All `Vec<T>` and `String` fields in wire protocol payloads are validated after
  deserialization via the `WireValidate` trait. Collection limits (e.g.,
  `MAX_REPORT_HOSTS = 500`, `MAX_VERSION_CHECK_ASSIGNMENTS = 2,000`) and string
  length limits (e.g., `MAX_SHORT_STRING_LEN = 1,024`,
  `MAX_OUTPUT_STRING_LEN = 1,048,576`) are enforced. Payloads exceeding any limit
  are rejected as deserialization failures (hard fail, connection close). See
  `crates/shared/wire/src/limits.rs` and `crates/shared/wire/src/wire_validate_impls.rs`.
- **Sliding-window rate limiter.** *(Implemented)* The message rate limiter uses a
  sliding-window-counter algorithm instead of a fixed window. The effective estimate
  is `prev_count * (1 - elapsed_fraction) + curr_count`, preventing boundary burst
  attacks where 2× the limit could be processed in rapid succession. See
  `crates/ui/web-api/src/routes/service_ws/protocol.rs`.
- **Consecutive unknown message counter.** *(Implemented)* The authenticated
  message loop tracks consecutive `Unknown` messages and closes the connection
  after `MAX_CONSECUTIVE_UNKNOWN_MESSAGES` (10) consecutive unknowns. The counter
  resets on any known message. Prevents fuzzing or probing clients from keeping
  connections alive indefinitely. See
  `crates/ui/web-api/src/routes/service_ws/handler/mod.rs`.

## Residual risk

- **`serde_json::Value` in plugin configs.** `PluginAssignment.config` is
  `serde_json::Value`, allowing arbitrary JSON structure within the message. While
  this is intentional for plugin flexibility, it means plugin config payloads are
  not type-checked at the wire protocol level.
- **No `deny_unknown_fields`.** Extra fields in incoming JSON are silently ignored.
  This is a forward-compatibility design choice but means field confusion attacks
  (sending fields intended for a different struct) are not detected.

## Recommended improvements

- ~~Add per-collection size limits~~ — **Implemented** via `WireValidate` trait.
- ~~Switch to sliding-window rate limiter~~ — **Implemented.**
- ~~Add per-field string length limits~~ — **Implemented** via `WireValidate` trait.
- ~~Monitor anonymous connections sending Unknown messages~~ — **Implemented** via
  consecutive unknown message counter (closes after 10).
- Consider adding `#[serde(deny_unknown_fields)]` to critical message payloads (with
  a version-negotiated opt-in) to detect field confusion attacks.

## References

- [Wire Protocol](../api/wire-protocol.md)
- [Wire Protocol — Connection Limits](../api/wire-protocol.md#connection-limits)
- `crates/shared/wire/src/messages.rs` — `ServiceMessage`, `ControllerMessage`
- `crates/shared/wire/src/envelope.rs` — `EnvelopeHeader`
- `crates/shared/wire/src/limits.rs` — `WireValidate` trait, limit constants,
  `WireValidationError`
- `crates/shared/wire/src/wire_validate_impls.rs` — per-struct validation
  implementations
- `crates/ui/web-api/src/routes/service_ws/protocol.rs` —
  `deserialize_service_msg()`, sliding-window `MessageRateLimiter`
- `crates/ui/web-api/src/routes/service_ws/handler/mod.rs` —
  `MAX_CONSECUTIVE_UNKNOWN_MESSAGES`, consecutive unknown counter
- `crates/ui/web-api/src/routes/service_ws/connection.rs` — `MAX_WS_MESSAGE_SIZE`
