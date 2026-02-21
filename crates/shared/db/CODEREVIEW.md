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

### INFORMATIONAL: `controller_event.message_json` as `String`

The field stores JSON but uses `column_type = "Text"`. Using `serde_json::Value` with `column_type = "Json"` would
enable database-level JSON validation and query capabilities. May be intentional for SQLite compatibility.

---

## Summary

| Category | Status | Notes |
| --- | --- | --- |
| AES-256-GCM | PASS | Correct algorithm, nonces, wire format |
| Key management | PASS | `OnceLock`, HA verification, single init |
| `EncryptedString` | PASS | Eager encryption, redaction, legacy fallback |
| Entity consistency | PASS | 34 entities follow identical patterns |
| `Eq` derives | PASS | All 21 eligible entity models now derive `Eq` |
| Error handling | PASS | rootcause/thiserror throughout |
| `unwrap`/`panic` | PASS | Zero in production code |
| PKCE verifier | PASS | Now encrypted with `EncryptedString` |
| Bearer token storage | PASS | UUID PKs + SHA-256 hash columns for lookup |
| Master key memory | PASS | Wrapped in `Zeroizing<[u8; 32]>` (defense-in-depth) |
| RoleMapping fallback | PASS | Sentinel error value on serialization failure (infallible for `HashMap<String, String>`) |
| HA safety | PASS | Insert-then-verify pattern eliminates first-creation race |
| Extensibility | GOOD | Crypto extracted to standalone `uptrakit-crypto` crate; agent-ssh no longer depends on shared-db |
