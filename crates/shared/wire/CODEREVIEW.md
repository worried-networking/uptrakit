# Code Review: uptrakit-internal-wire

Extensibility-focused review of the wire protocol crate.

## Role in the Architecture

This crate defines the service-controller wire protocol: message enums, payload structs, sequence
management, and close reasons. It sits between `shared-types` (foundation) and the service/
controller binaries.

## Findings

### Significant: ServiceMessage and ControllerMessage enums are closed

**Location:** `src/lib.rs`

`ServiceMessage` (13 variants) and `ControllerMessage` (17 variants) are exhaustive enums. Adding
a new message type (e.g., for a new service type's custom RPC) requires modifying this crate,
which triggers recompilation of every crate that depends on it.

**Impact:** External service developers cannot introduce custom message types without modifying
the wire crate. This is the most significant barrier to creating new service types.

**Recommendation:** Consider an extension mechanism for future protocol evolution:

```rust
// Add a catch-all variant for forward compatibility
Custom {
    message_type: String,
    payload: serde_json::Value,
}
```

This would allow experimental or external message types without modifying the wire crate. The
`PROTOCOL_VERSION` constant (currently `1`) could gate when custom messages are supported.

### Minor: re-exports from shared-types are appropriate

**Location:** `src/lib.rs:14-19`

The wire crate re-exports `HookShell`, `MqttClientConnectionStatus`, `MqttTransport`,
`OutputStreamType`, `ProviderType`, `ReleaseAsset`, `ReleaseInfo`, `ServiceType`, and
`SecretString` from `shared-types`. This is appropriate -- the wire crate acts as an abstraction
layer so downstream consumers don't need to depend on `shared-types` directly for wire-related
types.

## Positive Observations

- **Protocol is versioned** (`PROTOCOL_VERSION = 1`) -- enables future protocol evolution with
  negotiation.
- **`CloseReason` enum** with WebSocket close codes provides structured disconnect signaling.
- **`EnrollmentStatus`** is wire-specific (only `Pending` and `Approved`) rather than reusing
  the broader `ServiceStatus` -- appropriate narrowing for the enrollment context.
- **`HookCommand` enum** (`Shell` vs `Exec`) cleanly separates shell-interpreted from
  argument-list commands.
- **Sequence management** (`OutgoingSeq`, `IncomingSeq`) provides ordered message delivery
  guarantees.
- Only depends on `shared-types` -- minimal and clean.
