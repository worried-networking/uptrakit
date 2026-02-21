# Code Review: `uptrakit-service-sdk`

Reviewed: `src/lib.rs`, `src/error.rs` (218 lines), `src/lifecycle.rs` (325
lines), `src/connection.rs` (171 lines), `src/tls.rs` (216 lines),
`src/ca.rs`, `src/backoff.rs`, `src/cert_handler.rs`, `src/cli.rs`,
`src/ws.rs`, `src/identity.rs`, `Cargo.toml`.

## Summary

The service SDK is well-organized with clean trait boundaries and thorough
error categorization via domain sub-enums. Key issues are the `ServiceType`
enum constraining new service types at the enrollment boundary and the
inherent TOFU security trade-off (documented by design).

## Role in the Architecture

This crate provides the lifecycle management framework for services (agents,
MQTT service) that connect to the controller. External developers creating
new service types would use this crate as their primary dependency.

## Findings

### High — Extensibility

#### E1: `ServiceType` enum constrains new service types (ACCEPTED)

Accepted as a deliberate design tradeoff. All service types
are first-party and compiled together. The centralized `ServiceType` enum
provides exhaustive matching and compile-time safety across the enrollment
boundary. Adding a new service type requires modifying the `ServiceType`
enum in `shared-types`, which is acceptable given the current architecture.

### Low

#### L1: `build_tofu_client_config` security trade-off is well-documented

**File:** `src/tls.rs:112-185`

The `TofuVerifier` accepts any server certificate during CA bootstrap but
still validates TLS signatures (lines 136-170). The doc comments clearly
explain the security model: trust relies on post-download fingerprint
verification, not on certificate chain validation.

This is a reasonable TOFU trade-off. The signature verification prevents
trivial MITM attacks. The comment at line 131-132 documents the intent.

**Recommendation:** No change needed. Consider adding a log warning when
TOFU mode is active (if not already present in `ca.rs`).

### Info

#### I1: Clean `ServiceHandler` trait design

**File:** `src/lifecycle.rs:74-89`

The trait uses `Pin<Box<dyn Future>>` instead of `impl Future` to avoid
higher-ranked lifetime issues. This is well-documented (lines 70-73) and is
the correct choice for trait methods that capture references with complex
lifetime relationships. `run_service_lifecycle` orchestrates the full
lifecycle -- enrollment, certificate management, reconnection with backoff,
and graceful shutdown -- so service developers only implement business logic.

#### I2: Domain sub-enum structure in `EnrollmentError`

**File:** `src/error.rs:5-87`

The error enum is organized into four domain sub-enums (`TlsError`,
`IdentityError`, `ProtocolError`, `CaError`) with clear separation of
concerns. The `is_cert_expired()` method (lines 118-130) properly handles
three different wrapping layers (direct rustls, WebSocket-wrapped IO, plain
IO).

#### I3: Comprehensive `ReportConversion` coverage

**File:** `src/error.rs:134-151`

All foreign error types have `impl_report_conversion!` entries, including
indirect paths (e.g., `rustls::pki_types::pem::Error` -> `TlsError::Pem` ->
`EnrollmentError::Tls`). This ensures `.context_to()` works across all
boundaries.

#### I4: Test coverage for error classification

**File:** `src/error.rs:153-217`

Tests cover `is_cert_expired()` across all wrapping layers (direct rustls,
IO-wrapped rustls, WebSocket-wrapped IO-wrapped rustls) and negative cases.
This is critical for correct reconnect/re-enrollment behavior.

#### I5: Supporting infrastructure

- `CertificateRenewalHandler` eliminates boilerplate certificate management
  across services.
- `AuthenticatedContext` encapsulates the authenticated connection state
  without exposing raw WebSocket details.
- `Backoff` provides configurable exponential backoff with jitter for
  reconnection.
- Clean dependency chain: depends on `wire`, `directories`, `shared-types`
  -- no database or provider dependencies.
