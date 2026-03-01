# Code Review: uptrakit-crypto

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-crypto` (~761 LoC, single `lib.rs`) provides AES-256-GCM encryption at rest for sensitive
database fields via the `EncryptedString` SeaORM custom type. The crate uses `aws-lc-rs` (FIPS 140-3
validated), wraps the master key in `Zeroizing<[u8; 32]>`, and correctly redacts `Debug`/`Display`
output. A versioned prefix (`ENC:v1:`) enables future cipher migration.

The test suite is comprehensive (18 tests covering round-trip, nonce uniqueness, tampering, SeaORM
integration, plaintext mode, and all error variants). The main concern is that `sea-orm` is gated
behind a feature flag but the crate is consumed by `uptrakit-shared-db` which always enables it,
making the gate effective only in theory.

## Architecture

### Strengths

- `Cargo.toml:10-11` -- `sea-orm` is an optional dependency behind a `sea-orm` feature flag.
  Crates that need only `encrypt_str`/`decrypt_str` (without ORM integration) can depend on
  `uptrakit-crypto` without pulling in SeaORM.
- `src/lib.rs:137` -- `ENC:v1:` version prefix on encrypted values enables future cipher migration
  without a full re-encryption pass. Decryption checks the prefix to dispatch to the correct
  algorithm.
- `src/lib.rs:59-64` -- Global master key via `OnceLock<Zeroizing<[u8; 32]>>`. Immutable after
  initialization. No runtime locking on the read path.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/lib.rs:10` -- Uses `aws-lc-rs` (AWS-LC, a fork of BoringSSL with FIPS 140-3 validation)
  for AES-256-GCM. The nonce is a randomly generated 96-bit value drawn from `rand::rng()`.
- `src/lib.rs:59-64` -- Master key typed as `OnceLock<Zeroizing<[u8; 32]>>`. Key material is
  zeroed on drop via `Zeroizing`. `init_master_key()` accepts `Zeroizing<[u8; 32]>`, ensuring
  callers cannot hand over a plain array.
- `src/lib.rs:323-333` -- `Debug` writes `"EncryptedString(***)"`, `Display` writes
  `"***REDACTED***"`. Both verified by `test_debug_display_redact`.
- `src/lib.rs:108-134` -- Key verification sentinel allows HA deployments to verify all instances
  share the same master key. Decryption failure maps to `MasterKeyMismatch`, preventing oracle
  attacks.
- Zero `unsafe` blocks in production code.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/lib.rs:416-761` -- 18 tests covering: round-trip correctness, nonce uniqueness, prefix
  detection, `Debug`/`Display` redaction, ciphertext tampering, SeaORM `Value` round-trip,
  `ValueType` error cases, `Nullable` contract, clone/equality semantics, key verification
  token creation and tampering, all error variant paths (`AlreadyInitialized`,
  `CiphertextTooShort`, `Decryption`, `HexDecode`), and plaintext mode.
- `src/lib.rs:423` -- `TEST_LOCK: Mutex<()>` pattern correctly serializes all tests that touch
  the global `MASTER_KEY` within the same test binary.
- `src/lib.rs:18-50` -- `CryptoError` enum with 9 typed variants covers all failure modes.
  `thiserror`-derived `Display` on every variant.

### Issues

No code quality issues found.

## High Availability

### Strengths

- `src/lib.rs:59-64` -- `OnceLock` for the master key means no locking contention on the
  encrypt/decrypt hot path. Multiple threads can encrypt/decrypt concurrently without
  coordination.
- `src/lib.rs:108-134` -- Key verification token enables HA deployments to detect master key
  mismatches between controller instances before accepting traffic.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- `Cargo.toml:24-25` -- Uses workspace lints. Zero `#[allow(clippy::...)]` suppressions.
- `src/lib.rs:52-57` -- `impl_report_conversion!` macro for error conversion. Uses `report!`,
  `bail!`, and `context_to()` consistently throughout.
- `src/lib.rs:14` -- Depends on `uptrakit-shared-macros` for macro, `uptrakit-shared-types`
  for `SecretString` and hex utilities. All dependencies workspace-pinned.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- `src/lib.rs:137` -- `ENC:v1:` prefix makes it straightforward to introduce a new cipher
  (e.g., `ENC:v2:` for XChaCha20-Poly1305) by adding a second branch in the decrypt path.
- `src/lib.rs:338-411` -- SeaORM integration (`ValueType`, `TryGetable`, `Nullable`,
  `From<EncryptedString> for Value`) is gated behind the `sea-orm` feature. The type is a
  drop-in replacement for `String` in entity definitions.
- `src/lib.rs:339-352` -- Legacy plaintext migration path: values without the `ENC:v1:` prefix
  are accepted as plaintext, enabling rolling migration without a downtime re-encryption step.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `src/lib.rs:416-761` -- 18 inline tests covering: AES-GCM round-trip (empty, ASCII,
  Unicode, 10 KB), nonce uniqueness, `is_encrypted` prefix detection, `Debug`/`Display`
  redaction, ciphertext tampering (tag corruption), SeaORM `Value` round-trip, `ValueType`
  error cases, `Nullable` contract, clone/equality semantics, key-verification token creation
  and tampering, all `CryptoError` variants (`AlreadyInitialized`, `CiphertextTooShort`,
  `Decryption`, `HexDecode`), and plaintext migration mode.
- `tests/not_initialized.rs` -- Isolated integration-test binary runs in a fresh process
  so that `MASTER_KEY` (a `OnceLock`) is guaranteed unset. Tests `EncryptedString::new` and
  `encrypt_str` both return `CryptoError::NotInitialized`. Using a separate binary is the
  correct approach; it would be impossible to test this invariant reliably inside the same
  binary that also initialises the key.
- `src/lib.rs:423` -- `TEST_LOCK: Mutex<()>` serialises all tests sharing the global
  `MASTER_KEY` within the same binary, preventing order-dependent failures when tests are
  run in parallel.

### Issues

No test coverage issues found.
