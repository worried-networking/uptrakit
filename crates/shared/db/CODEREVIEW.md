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

### Significant: all 34 entities in one crate

**Location:** `src/entity/`

The crate contains 34 entity models spanning the entire system:

- **Controller-only entities** (not needed by agent-ssh): `oidc_provider`,
  `pending_oidc_flow`, `pending_oidc_registration`, `pending_oidc_token_exchange`,
  `pending_account_link`, `pending_device_flow`, `api_rate_limit`, `api_token`, `auth_method`,
  `session`, `user`, `user_role`, `user_oidc_link`, `role`, `role_permission`, `permission`,
  `mqtt_client`, `mqtt_lease`, `scheduled_task`, `settings_version`, `controller_event`, and more.
- **Shared entities**: `service`, `service_host`, `service_certificate`, `host`,
  `software_item`, `host_software_item`, `provider_config`, `tenant`.
- **Agent-ssh uses**: primarily the `crypto` module for `EncryptedString`, plus its own local
  migrations.

The SSH agent compiles all 34 entity models even though it only needs the crypto module and
potentially a few shared entities for type compatibility.

**Impact:** Increased compile time and binary size for agent-ssh. Conceptual coupling between the
agent and controller-specific schema (OIDC, rate limiting, etc.).

**Recommendation:** Split entities into feature-gated modules:

```toml
[features]
default = ["crypto"]
crypto = []
controller-entities = []
all-entities = ["controller-entities"]
```

The SSH agent would depend on `shared-db` with only the `crypto` feature. The controller and
web-api would enable `all-entities`.

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

### MEDIUM: Bearer tokens as plaintext primary keys

Several pending flow entities store bearer tokens as plaintext primary keys:

| Entity | Field | Risk |
| --- | --- | --- |
| `pending_device_flow` | `device_code` | Attacker with DB access can poll to obtain tokens |
| `pending_account_link` | `link_token` | Attacker can complete account linking |
| `pending_oidc_registration` | `registration_code` | Attacker can complete OIDC registration |
| `pending_oidc_token_exchange` | `exchange_code` | Attacker can complete token exchange |

These tokens are used as primary keys, so hashing would require a lookup-by-hash pattern, and encrypting with
`EncryptedString` would break lookups (random nonces). This is a design constraint requiring careful thought -- consider
storing a hash in a separate indexed column for lookup.

### MEDIUM: Master key not zeroized in memory

**File:** `src/crypto.rs`, line 58

The raw key bytes `[u8; 32]` are stored in a `OnceLock`. If process memory is dumped (core dump, swap), the key is
exposed. Consider using `Zeroizing<[u8; 32]>` from the `zeroize` crate. Since `OnceLock` never drops, this is
defense-in-depth, not critical.

### MEDIUM: `EncryptedString::new()` dev-mode fallback lacks warning

**File:** `src/crypto.rs`, lines 222-231

When `master_key_available()` returns `false`, plaintext is stored directly. While this is controlled by the
`--allow-plaintext-secrets` flag at startup, no `tracing::warn!` is emitted when this fallback path is taken, making it
invisible in production logs.

### LOW: HA race condition on first token creation

If two instances start simultaneously and neither has a stored verification token, both will call
`create_key_verification_token()` and attempt to store it. If they have different master keys, the last writer "wins" and
the other may not detect the mismatch until restart. Consider using an INSERT-or-fail pattern (not upsert) for initial
token creation.

### LOW: Key verification error discards root cause

**File:** `src/crypto.rs`, line 97

The `Err(_)` branch in `verify_key_verification_token` discards the original decryption error, returning only
`MasterKeyMismatch`. For diagnostics, consider logging the underlying error at `debug` level before returning.

### LOW: No nonce collision documentation

With random 96-bit nonces and AES-256-GCM, the birthday bound for nonce collision is ~2^48 encryptions under the same
key. This should be documented as a comment in the crypto module.

### LOW: No background re-encryption mechanism

Legacy plaintext values persist indefinitely in the database. A migration script or background task should be
implemented to progressively encrypt them.

### LOW: `DeviceAuthStatus` re-export location inconsistency

`pending_device_flow.rs` re-exports `DeviceAuthStatus` from within an entity module, while other shared type re-exports
happen in `lib.rs`. Consider moving for consistency.

### LOW: `AuthMethod::from_session` data integrity assumption

When `kind == "oidc"` but `oidc_provider_id` is `None`, returns `None`. This is a data integrity invariant that depends
on the DB schema enforcing `oidc_provider_id IS NOT NULL WHEN auth_method = 'oidc'`. If no CHECK constraint exists, this
is a latent bug.

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

### INFORMATIONAL: Sensitive data in `Debug` derives

Entities with sensitive `String` fields (`user.password_hash`) will expose those values through `Debug` output.
`EncryptedString` properly redacts, but plain `String` fields do not get this protection. Note:
`pending_oidc_flow.pkce_verifier` is now `EncryptedString` and benefits from automatic redaction.
Consider custom `Debug` implementations for entities containing security-sensitive fields.

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
| Bearer token storage | **MEDIUM** | Plaintext PKs for pending flows |
| Master key memory | **MEDIUM** | Not zeroized; defense-in-depth concern |
| RoleMapping fallback | PASS       | Sentinel error value on serialization failure (infallible for `HashMap<String, String>`) |
| HA safety | GOOD | Verification works; minor race on first creation |
| Extensibility | FAIR | All 34 entities in one crate; needs feature gating |
