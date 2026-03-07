# Code Review: uptrakit-crypto

- **Review date**: 2026-03-02
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
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
- `src/lib.rs:252-253,524-525,617-618,686-687` -- Random 96-bit nonces generated via
  `rand::rng().fill_bytes()` for each encryption operation. Properly uses
  `Nonce::assume_unique_for_key()` -- acceptable given random generation per operation.
- `src/lib.rs` -- Context-bound AAD on v2/v3 ciphertexts prevents ciphertext relocation
  attacks (e.g., moving a password hash to a different column). `register_column_aad` validates
  uniqueness at startup.
- `src/lib.rs:108-201` -- Proper DEK/KEK separation (envelope encryption): master key wraps
  DEKs; DEKs encrypt data. O(1) master key rotation (re-wrap DEKs only). DEK key_id verified
  after unwrapping, preventing key confusion.
- `src/ecies.rs` -- Proper ephemeral-static ECDH with P-256. Ephemeral key per encryption
  provides CCA2 security. Ephemeral public key used as AAD, binding ciphertext to the specific
  exchange. Input validation on recipient public key length.
- Zero `unsafe` blocks in production code.

*(2026-03-06 parallel review -- security: 28 positive security findings confirmed across
AES-256-GCM, envelope encryption, master key management, ECIES, nonce generation, AAD binding,
and key material zeroization.)*

### Issues

**[LOW]** `src/lib.rs:101-106,431-445` -- `PLAINTEXT_MODE` (`AtomicBool`) allows storing
secrets without encryption when `--allow-plaintext-secrets` is passed. If accidentally enabled
in production, all secrets are stored in plaintext. The guard is purely flag-based with no
compile-time enforcement. The CLI flag requirement is reasonable defense-in-depth. Consider
feature-gating `enable_plaintext_mode()` behind a `dev-only` or `testing` Cargo feature, or
adding an explicit startup banner when active that monitoring can alert on.
*(2026-03-06 parallel review -- security)*

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

**[LOW]** `src/lib.rs:151-154` -- `DataKeyRing::new` uses `assert!` to validate that
`active_key_id` is present in the `keys` map. While documented with `# Panics`, this is
production crypto code where a startup failure should return a
`Result<Self, Report<CryptoError>>` rather than panic. The coding standard says "`unwrap()`,
`expect()`, and `panic!()` are forbidden in production code." The `assert!` compiles in release
builds. Recommendation: convert `DataKeyRing::new` to return `Result`, adding a
`MissingActiveKey` variant.
*(2026-03-06 parallel review -- code quality)*

**[LOW]** `src/lib.rs:170` -- `DataKeyRing::active_key()` uses
`.expect("active key must exist in ring")`. This is called in the hot path during encryption.
The `expect` is only reachable after `new` succeeds and the ring is immutable thereafter, so
in practice it cannot fire. Consider converting to an infallible accessor if `new` is changed
to return `Result`.
*(2026-03-06 parallel review -- code quality)*

~~**[LOW]** `src/lib.rs:63,72+89-92` -- Dual `#[from]` + `impl_report_conversion!` on
`CryptoError::HexDecode` and `CryptoError::InvalidUtf8`. When callers use `.context_to()?`,
only the `impl_report_conversion!` is exercised -- the `#[from]` generates unused `From` impls.
Remove `#[from]` from these variants to align with the project's documented guidance.
*(2026-03-06 parallel review -- code quality, coding standards)*~~ *(Fixed.)*

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
