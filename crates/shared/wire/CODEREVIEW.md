# Code Review: `uptrakit-internal-wire`

Reviewed: `src/lib.rs` (2188 lines), `src/close_reason.rs` (210 lines), `Cargo.toml`.

## Summary

The wire crate is well-structured with strong serialization test coverage and
backward compatibility.

## Role in the Architecture

This crate defines the service-controller wire protocol: message enums,
payload structs, sequence management, and close reasons. It sits between
`shared-types` (foundation) and the service/controller binaries.

## Findings

### High — Extensibility

#### E1: `ServiceMessage` and `ControllerMessage` enums are closed (ACCEPTED)

Accepted as a deliberate design tradeoff. The wire protocol
is versioned (`PROTOCOL_VERSION = 1`) and all services are compiled together.
Closed enums provide exhaustive match checking and compile-time safety. Adding
a new message type requires modifying the wire crate, which is acceptable
given the current architecture where all service types are first-party.

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
