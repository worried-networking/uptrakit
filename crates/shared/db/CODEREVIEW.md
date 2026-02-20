# Code Review: `uptrakit-shared-db`

**Date:** 2026-02-17
**Reviewer:** Claude Opus 4.6 (automated)
**Scope:** Architecture, security, code quality, HA safety, coding standards
**Overall quality: HIGH (88/100)**

All tests pass. The crypto module is one of the strongest application-level encryption-at-rest implementations in the
codebase.

---

## Architecture

The crate has two layers:

- **`crypto.rs`**: AES-256-GCM at-rest encryption via `aws-lc-rs`, with eager encryption at construction time, versioned
  wire format (`ENC:v1:`), legacy plaintext fallback, and HA master key verification.
- **`entity/`**: 34 SeaORM entity models following a highly consistent pattern.

---

## Extensibility Findings

### ~~Significant: all 34 entities in one crate~~ RESOLVED

**Resolution:** The crypto module was extracted into a standalone `uptrakit-crypto` crate
(`crates/shared/crypto/`). `shared-db` re-exports it for backward compatibility. Agent-ssh now
depends on `uptrakit-crypto` directly, dropping its `shared-db` dependency entirely. This
eliminates the compile-time and binary-size cost of 34 entity definitions for the SSH agent.

### Extensibility positives

- **Re-exports enums from shared-types** (`MqttClientConnectionStatus`, `MqttTransport`,
  `OutputStreamType`, `SessionTokenType`) -- clean minimal public API.
- **`EncryptedString` type** provides transparent at-rest AES-256-GCM encryption -- reusable by
  any crate needing encrypted storage.
- **`OnceLock`-based master key initialization** ensures the key is set exactly once.
- Clean separation between `crypto` (reusable) and `entity` (schema-specific) modules.

---

## Crypto Module Findings

### PASS: AES-256-GCM correctness

- Algorithm choice (AES-256-GCM via `aws-lc-rs`) is FIPS-validated and hardware-accelerated.
- 12-byte random nonces via `rand::rng().fill_bytes()` are correct.
- Wire format `nonce || ciphertext || tag` with versioned prefix `ENC:v1:` is clean and allows future algorithm
  migration.
- `seal_in_place_separate_tag` with manual tag append, and `open_in_place` on the decrypt side, are correctly paired.

### PASS: `EncryptedString` design

- Eager encryption at construction ensures `From<EncryptedString> for sea_orm::Value` is infallible.
- Pre-computed `db_value` avoids re-encryption on every insert.
- `Debug`/`Display` redaction prevents secret leakage in logs.
- `PartialEq` on plaintext only (correct since random nonces make ciphertext comparison meaningless).
- Legacy plaintext fallback on read enables gradual migration without downtime.

### PASS: Master key verification (HA)

The startup sequence correctly handles:

1. First instance: creates and stores verification token.
2. Subsequent instances: verifies stored token matches current key.
3. Key mismatch: hard error with clear message about HA key consistency.

### PASS: Error handling

Uses `rootcause` + `thiserror` consistently. All error paths use `report!()`/`bail!()`. No errors silently swallowed in
production code.

### PASS: No production `unwrap`/`panic`

All `unwrap()`, `expect()`, and `panic!()` calls are exclusively in `#[cfg(test)]` blocks.

### ~~MEDIUM: Bearer tokens as plaintext primary keys~~ RESOLVED

**Resolution:** All 4 pending flow entities now use a UUID `id` as primary key with a SHA-256 hash column for lookups:

| Entity | New PK | Hash column |
| --- | --- | --- |
| `pending_device_flow` | `id: Uuid` | `device_code_hash: String` (unique) |
| `pending_account_link` | `id: Uuid` | `link_token_hash: String` (unique) |
| `pending_oidc_registration` | `id: Uuid` | `registration_code_hash: String` (unique) |
| `pending_oidc_token_exchange` | `id: Uuid` | `exchange_code_hash: String` (unique) |

Bearer tokens are hashed via `hash_token()` (SHA-256, same as API tokens) before storage and lookup.
The migration, 4 entity files, and 2 query files (`device_flow.rs`, `oidc_state.rs`) were updated.
`user_code` in device flows remains unhashed (short-lived user-facing code, not a bearer token).

### ~~MEDIUM: Master key not zeroized in memory~~ RESOLVED

Resolved: Master key is now wrapped in `Zeroizing<[u8; 32]>` from the `zeroize` crate. `init_master_key()` accepts
`Zeroizing<[u8; 32]>`. Defense-in-depth since `OnceLock` has `'static` lifetime.

### ~~MEDIUM: `EncryptedString::new()` dev-mode fallback lacks warning~~ RESOLVED

Resolved: `tracing::warn!("master key not configured; storing value as plaintext (development mode)")` is now emitted
when the plaintext fallback path is taken.

### ~~LOW: HA race condition on first token creation~~ RESOLVED

**Resolution:** `verify_master_key()` now uses `insert_setting_if_absent()` (INSERT with ON CONFLICT DO NOTHING) instead
of `upsert_setting()`. If the insert fails because another instance raced and stored a token first, the current instance
re-reads the stored token and verifies it against the current master key. This ensures key mismatches are always detected
even during simultaneous startup.

### ~~LOW: Key verification error discards root cause~~ RESOLVED

Resolved: The `Err` branch in `verify_key_verification_token` now logs the underlying decryption error at `debug` level
via `tracing::debug!(error = %e, "key verification decryption failed")` before returning `MasterKeyMismatch`.

### ~~LOW: No nonce collision documentation~~ RESOLVED

~~With random 96-bit nonces and AES-256-GCM, the birthday bound for nonce collision is ~2^48 encryptions under the same
key. This should be documented as a comment in the crypto module.~~

**Resolution:** Added documentation to `encrypt_value()` in `uptrakit-crypto` covering nonce collision probability
(birthday bound ~2^48), safety margins for the application's use case, and NIST SP 800-38D reference.

### ~~LOW: No background re-encryption mechanism~~ RESOLVED

~~Legacy plaintext values persist indefinitely in the database. A migration script or background task should be
implemented to progressively encrypt them.~~

**Resolution:** Added a startup re-encryption routine (`reencrypt.rs`) in the controller. After master key
verification (Phase 4b), it scans all 5 encrypted columns across 4 tables (`ca_certificates.key_pem`,
`oidc_providers.client_secret`, `mqtt_clients.password`, `mqtt_clients.ca_cert_pem`,
`pending_oidc_flows.pkce_verifier`) for values lacking the `ENC:v1:` prefix. Plaintext values are re-encrypted
in place. The routine is idempotent, HA-safe (last writer wins with identical result), and fault-tolerant
(per-row errors are logged and skipped). `EncryptedString::is_db_value_encrypted()` was added to support the
prefix check without exposing raw DB values.

### ~~LOW: `DeviceAuthStatus` re-export location inconsistency~~ RESOLVED

~~`pending_device_flow.rs` re-exports `DeviceAuthStatus` from within an entity module, while other shared type re-exports
happen in `lib.rs`. Consider moving for consistency.~~

**Resolution:** Moved `DeviceAuthStatus` re-export from `pending_device_flow.rs` to `lib.rs` alongside
`MaskedEmail`, `MqttClientConnectionStatus`, `MqttTransport`, `OutputStreamType`, `SecretString`, and
`SessionTokenType`. Downstream imports updated.

### ~~LOW: `AuthMethod::from_session` data integrity assumption~~ RESOLVED

~~When `kind == "oidc"` but `oidc_provider_id` is `None`, returns `None`. This is a data integrity invariant that depends
on the DB schema enforcing `oidc_provider_id IS NOT NULL WHEN auth_method = 'oidc'`. If no CHECK constraint exists, this
is a latent bug.~~

**Resolution:** Three-pronged fix: (1) Added `CHECK(auth_method != 'oidc' OR oidc_provider_id IS NOT NULL)` to the
sessions table in the initial migration. (2) Replaced `unwrap_or(AuthMethod::Password)` in `session.rs` (both
`verify_refresh_token` and `rotate_refresh_token`) with `ok_or_else(|| report!(AuthError::InvalidSession))` that logs
a warning and rejects the session. (3) Fixed `require_auth.rs` JWT path to return
`AuthFailure::Unauthorized("Invalid OIDC session: missing provider")` instead of falling back to Password. Added
`AuthError::InvalidSession` variant and two tests for corrupted OIDC sessions.

---

## Entity Layer Findings

### PASS: Consistency

All 34 entity files follow a highly consistent pattern: `#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]`, correct
`table_name`, `auto_increment = false` for UUID PKs, empty `ActiveModelBehavior`, and correct `Relation`/`Related`
impls.

### PASS: Relationship definitions

All many-to-many relationships use proper junction tables with `via()`. All one-to-many use correct `has_many`/
`belongs_to`.

### INFORMATIONAL: No entity-level validation

None of the entities implement validation in `ActiveModelBehavior`. All use empty impls. Acceptable if validation is
handled in the service layer.

### INFORMATIONAL: Missing `Eq` derives

Most entity models derive `PartialEq` but not `Eq`. For models without `EncryptedString` fields, adding `Eq` is free
and enables `HashSet`/`HashMap` usage. Entities containing `EncryptedString` (`ca_certificate`, `oidc_provider`,
`mqtt_client`) correctly cannot derive `Eq`.

### INFORMATIONAL: `controller_event.message_json` as `String`

The field stores JSON but uses `column_type = "Text"`. Using `serde_json::Value` with `column_type = "Json"` would
enable database-level JSON validation and query capabilities. May be intentional for SQLite compatibility.

### ~~INFORMATIONAL: Sensitive data in `Debug` derives~~ (FIXED)

~~Entities with sensitive `String` fields (`user.password_hash`) will expose those values through `Debug` output.
`EncryptedString` properly redacts, but plain `String` fields do not get this protection. Note:
`pending_oidc_flow.pkce_verifier` is now `EncryptedString` and benefits from automatic redaction.
Consider custom `Debug` implementations for entities containing security-sensitive fields.~~

**Resolution:** Changed `password_hash` from `Option<String>` to `Option<SecretString>` (redacted Debug/Display, zeroize-on-drop). Changed `email` from `String` to `MaskedEmail` (masked Debug/Display, full value preserved for serialization).

---

## Summary

| Category | Status | Notes |
| --- | --- | --- |
| AES-256-GCM | PASS | Correct algorithm, nonces, wire format |
| Key management | PASS | `OnceLock`, HA verification, single init |
| `EncryptedString` | PASS | Eager encryption, redaction, legacy fallback |
| Entity consistency | PASS | 34 entities follow identical patterns |
| Error handling | PASS | rootcause/thiserror throughout |
| `unwrap`/`panic` | PASS | Zero in production code |
| PKCE verifier | PASS | Now encrypted with `EncryptedString` |
| Bearer token storage | ~~**MEDIUM**~~ FIXED | UUID PKs + SHA-256 hash columns for lookup |
| Master key memory | PASS       | Wrapped in `Zeroizing<[u8; 32]>` (defense-in-depth) |
| RoleMapping fallback | PASS       | Sentinel error value on serialization failure (infallible for `HashMap<String, String>`) |
| HA safety | PASS | Insert-then-verify pattern eliminates first-creation race |
| Extensibility | ~~FAIR~~ GOOD | Crypto extracted to standalone `uptrakit-crypto` crate; agent-ssh no longer depends on shared-db |
