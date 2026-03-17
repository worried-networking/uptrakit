# Code Review: `uptrakit-crypto`

- Review date: 2026-03-17
- Scope: current-state review (full 14-dimension)

## Summary

The crypto crate remains one of the most robust parts of the workspace. Envelope encryption with
KEK/DEK separation, AES-256-GCM, Argon2id password hashing, ECIES sealed-box encryption, and
three-format backward compatibility are all well-implemented. Two operational correctness issues
persist from the previous review cycle: a silent fallback to empty AAD when a column name is not
registered, and a ValueType deserialization path that cannot carry column context. Both are
confirmed against the current code.

## Strengths

- AES-256-GCM via `aws-lc-rs` with random per-encryption nonces prevents nonce reuse in normal
  paths.
- Envelope encryption (v3) with KEK/DEK separation enables O(1) master-key rotation without data
  re-encryption.
- AAD context-binding prevents ciphertext relocation between different column roles.
- ECIES sealed-box (P-256 ECDH + AES-256-GCM) for end-to-end extension parameter encryption is
  well-tested with thorough edge cases (empty plaintext, large plaintext, wrong key, tampered
  ciphertext, truncated input).
- Argon2id with OWASP-recommended parameters (19 MiB, 2 iterations, parallelism 1).
- Three-format transparent backward compatibility (v1/v2/v3 read paths) with v3 as the write path.
- v1 encryption is gated behind `#[cfg(test)]` -- no new v1 ciphertexts can be produced in
  production.
- Good test depth for key rotation, legacy format handling, and failure behavior.
- `Zeroizing<[u8; 32]>` wrapping on all key material, including DEK bytes.
- Key verification token with dedicated AAD (`KEY_VERIFICATION_AAD`) prevents ciphertext
  relocation attacks.
- Data key ring enforces the active-key-present invariant at construction time.
- Column AAD registry detects duplicate column names at startup with a clear error type.

## Active Findings

### [MEDIUM] AAD lookup silently falls back to empty string on unregistered columns

- **Dimension**: security, correctness
- **Scope**: `crates/shared/crypto/src/encrypted_string.rs:224-225`, `TryGetable` impl
- **Description**: When `column_aad(col_name)` returns `None`, the `TryGetable` path proceeds
  with `unwrap_or("")`. A v3 ciphertext encrypted with the correct column AAD will fail to
  decrypt (returning an error), but the error message is generic and there is no operational
  alert that a column registration is missing.
- **Why it matters**: data becomes unrecoverable until the registration is added and the
  service is restarted. The operator sees a decryption error without actionable context.
- **Failure scenario**: a plugin crate encrypts a field but does not register its column AAD in
  the application startup; the column returns an opaque decryption error in production until the
  registration is added and the service is restarted.

### [LOW] `ValueType` deserialization path has no column context and cannot carry correct AAD

- **Dimension**: security, correctness
- **Scope**: `crates/shared/crypto/src/encrypted_string.rs:146-189`, `ValueType` impl
- **Description**: The `ValueType` deserialization path (used outside of SeaORM entity queries)
  cannot receive a column name, so it always decrypts v3 ciphertexts with empty AAD. The code
  comment at line 151 acknowledges this limitation. A ciphertext encrypted with a non-empty AAD
  will fail decryption on this path.
- **Why it matters**: raw SQL queries or non-ORM code paths that deserialize `EncryptedString`
  from v3 ciphertexts silently fail, which is hard to distinguish from a decryption key mismatch.
- **Failure scenario**: a developer writes a raw query returning an `EncryptedString` column
  and gets a confusing decryption error that only manifests after migration to v3.
