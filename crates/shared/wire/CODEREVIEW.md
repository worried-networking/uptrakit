# Code Review: `uptrakit-internal-wire`

Reviewed: `src/lib.rs` (2188 lines), `src/close_reason.rs` (210 lines), `Cargo.toml`.

## Summary

The wire crate is well-structured with strong serialization test coverage and
backward compatibility. Key issues are inconsistent use of `thiserror`,
missing `#[non_exhaustive]` on several public enums, and the closed nature
of the message enums limiting protocol extensibility.

## Role in the Architecture

This crate defines the service-controller wire protocol: message enums,
payload structs, sequence management, and close reasons. It sits between
`shared-types` (foundation) and the service/controller binaries.

## Findings

### High — Extensibility

#### E1: `ServiceMessage` and `ControllerMessage` enums are closed

**Location:** `src/lib.rs`

`ServiceMessage` (13 variants) and `ControllerMessage` (17 variants) are
exhaustive enums. Adding a new message type (e.g., for a new service type's
custom RPC) requires modifying this crate, which triggers recompilation of
every crate that depends on it.

**Impact:** External service developers cannot introduce custom message
types without modifying the wire crate. This is the most significant
barrier to creating new service types.

**Recommendation:** Consider an extension mechanism for future protocol
evolution:

```rust
// Add a catch-all variant for forward compatibility
Custom {
    message_type: String,
    payload: serde_json::Value,
}
```

This would allow experimental or external message types without modifying
the wire crate. The `PROTOCOL_VERSION` constant (currently `1`) could gate
when custom messages are supported.

### Medium

#### ~~M1: `SeqError` uses manual `Display`/`Error` despite `thiserror` dependency~~ (FIXED)

**Resolution:** Converted `SeqError` to use `#[derive(thiserror::Error)]` with
`#[error("sequence error: expected {expected}, received {received}")]`, removing
the manual `Display` and `Error` implementations.

#### ~~M2: `ErrorCode` lacks `#[non_exhaustive]`~~ (FIXED)

**Resolution:** Added `#[non_exhaustive]` to `ErrorCode`.

#### ~~M3: `EnrollmentStatus` lacks `#[non_exhaustive]`~~ (FIXED)

**Resolution:** Added `#[non_exhaustive]` to `EnrollmentStatus`.

#### ~~M4: `UpdateFinalStatus` lacks `#[non_exhaustive]`~~ (FIXED)

**Resolution:** Added `#[non_exhaustive]` to `UpdateFinalStatus`. External consumers
(e.g., `agent_ws.rs`) now include wildcard match arms.

#### ~~M5: `DisconnectReason` lacks `#[non_exhaustive]`~~ (FIXED)

**Resolution:** Added `#[non_exhaustive]` to `DisconnectReason`.

### Low

#### L1: `ErrorCode` and `EnrollmentStatus` use manual `Display` impls

**File:** `src/lib.rs:29-36` (EnrollmentStatus), `src/lib.rs:294-307` (ErrorCode)

Both enums have manual `Display` impls that produce `snake_case` output
matching their serde `rename_all`. These could be derived with `strum::Display`
(`#[strum(serialize_all = "snake_case")]`) or `thiserror` to reduce boilerplate
and ensure consistency with the serde representation.

**Recommendation:** Consider deriving `Display` via `strum` or keeping the
manual impl if the explicit mapping is preferred for wire-format stability.
Document the choice.

#### L2: `HookCommand::Display` does not round-trip with serde

**File:** `src/lib.rs:61-81`

The `Display` impl for `HookCommand::Shell` only outputs the command string
(line 64), losing the `shell` field. This is fine for logging but could be
confusing if used for comparison or reconstruction.

**Recommendation:** Document that `Display` is for human-readable logging only,
not for round-trip serialization.

### Info

#### I1: Re-exports from `shared-types` are appropriate

**File:** `src/lib.rs:14-19`

The wire crate re-exports `HookShell`, `MqttClientConnectionStatus`,
`MqttTransport`, `OutputStreamType`, `ProviderType`, `ReleaseAsset`,
`ReleaseInfo`, `ServiceType`, and `SecretString` from `shared-types`. This
is appropriate -- the wire crate acts as an abstraction layer so downstream
consumers don't need to depend on `shared-types` directly for wire-related
types.

#### I2: Excellent backward compatibility testing

**File:** `src/lib.rs:956-976`, `src/lib.rs:1310-1355`

Multiple tests verify that messages from older protocol versions (missing
fields like `latest_version`, `ca_bundle_hash`, `shutdown_timeout_seconds`)
deserialize correctly via `#[serde(default)]`. This is exemplary for a wire
protocol crate.

#### I3: `Box<ExecuteUpdatePayload>` reduces enum size

**File:** `src/lib.rs:133`

`ExecuteUpdatePayload` is the largest variant and is properly boxed to keep
`ControllerMessage` at a reasonable size. Good practice.

#### I4: `close_reason.rs` is well-designed

**File:** `src/close_reason.rs`

The `CloseReason` enum with `Unknown(String)` fallback, `FromStr`/`Display`
parity, and `ParseCloseReasonError` following naming conventions is a model
implementation. Thorough test coverage (lines 112-209) covers all known
variants, unknown passthrough, empty strings, equality, and clone.

#### I5: Protocol is versioned

`PROTOCOL_VERSION = 1` enables future protocol evolution with negotiation.

#### I6: Test coverage is comprehensive

~450 lines of tests covering serialization roundtrips for every message type,
all enum variants, backward compatibility, edge cases (empty vectors omitted,
None fields omitted, default values), and sequence number validation.
