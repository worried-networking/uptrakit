# Code Review: `uptrakit-crypto`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The crypto crate remains one of the most robust parts of the workspace. Envelope encryption with
KEK/DEK separation, AES-256-GCM, Argon2id password hashing, and three-format backward compatibility
are all well-implemented. Two operational correctness issues were found in this review cycle: a
silent fallback to empty AAD when a column name is not registered, and a ValueType deserialization
path that cannot carry column context.

## Strengths

- AES-256-GCM via `aws-lc-rs` with random per-encryption nonces prevents nonce reuse in normal
  paths.
- Envelope encryption (v3) with KEK/DEK separation enables O(1) master-key rotation without data
  re-encryption.
- AAD context-binding prevents ciphertext relocation between different column roles.
- Argon2id with OWASP-recommended parameters (19 MiB, 2 iterations, parallelism 1).
- Three-format transparent backward compatibility (v1/v2/v3 read paths) with v3 as the write path.
- Good test depth for key rotation, legacy format handling, and failure behavior.

## Active Findings

### [MEDIUM] AAD lookup silently falls back to empty string on unregistered columns

- Dimension: security, correctness
- Scope: `crates/shared/crypto/src/encrypted_string.rs`, `TryGetable` impl
- Why it matters: when `column_aad(col_name)` returns no registration, the `TryGetable` path
  proceeds with an empty AAD string. A v3 ciphertext encrypted with the correct column AAD will
  fail to decrypt (returning an error), but the error message is generic and there is no operational
  alert that a column registration is missing. Data becomes silently unrecoverable.
- Failure scenario: a plugin crate encrypts a field but does not register its column AAD in the
  application startup; the column returns an opaque decryption error in production until the
  registration is added and the service is restarted.
- Fix: log a `tracing::error!` when `column_aad` returns nothing before falling back, or return a
  typed `TryGetError` with the column name so the operator can identify the missing registration.

### [LOW] `ValueType` deserialization path has no column context and cannot carry correct AAD

- Dimension: security, correctness
- Scope: `crates/shared/crypto/src/encrypted_string.rs`, `ValueType` impl
- Why it matters: the `ValueType` deserialization path (used outside of SeaORM entity queries)
  cannot receive a column name, so it always decrypts v3 ciphertexts with empty AAD. A ciphertext
  encrypted with a non-empty AAD will fail decryption on this path.
- Failure scenario: raw SQL queries or non-ORM code paths that deserialize `EncryptedString` from
  v3 ciphertexts silently fail, which is hard to distinguish from a decryption key mismatch.
- Fix: document this limitation prominently. Consider a `from_db_with_aad(value, aad)` method for
  non-ORM consumers that need explicit AAD injection.

### [INFO] Webhook HMAC secret is not wrapped in `SecretString` during use

- Dimension: security
- Scope: `crates/plugins/notifications/webhook/src/lib.rs`, HMAC computation path
- Why it matters: the webhook secret is extracted from the config as a plain `&str` and used
  directly for HMAC computation. If the config object is logged at any level before this point,
  the secret is visible in log output. The secret is encrypted at rest, so the risk is contained
  to the application's log aggregation pipeline.
- Fix: wrap the extracted secret in `SecretString` temporarily and expose only via
  `expose_secret()` at the HMAC call site.
