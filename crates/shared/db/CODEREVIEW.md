# Code Review: uptrakit-shared-db

## Summary

Database layer crate providing SeaORM entity definitions (33 entities) and AES-256-GCM encryption at rest via `EncryptedString`. This is the most complex shared crate and the primary layer affected by high-availability (multi-controller) considerations. The crypto module handles key management, encryption/decryption, and transparent SeaORM integration.

## Architecture

- **Module structure**: `lib.rs` exposes `crypto` and `entity` modules. Entity module contains 33 entity files plus `mod.rs` and `prelude.rs`.
- **Public API surface**: `crypto::init_master_key()`, `crypto::master_key_available()`, `crypto::EncryptedString`, all entity `Model`/`Column`/`Entity`/`Relation` types.
- **Dependency choices**: `sea-orm` (ORM), `aws-lc-rs` (AES-256-GCM), `rand` (nonce generation), `uptrakit-shared-types` (SecretString, hex) -- all appropriate.
- **Layering**: Central shared crate depended on by controller, web-api, MQTT service, and other shared crates.
- **Encryption design**: AES-256-GCM with random 12-byte nonce per encryption, stored as `ENC:v1:<hex(nonce || ciphertext || tag)>`. Global master key held in `OnceLock<[u8; 32]>`.

## Security & Safety

- **No `unsafe` code.**
- **No `unwrap`/`panic` in non-test code.** Test code uses `TEST_LOCK.lock().unwrap()` (acceptable per project standards: `panic = "abort"` in release).
- **Secret redaction**: `EncryptedString` redacts in `Debug` (`"EncryptedString(***)"`) and `Display` (`"***REDACTED***"`).
- **Nonce handling**: 12-byte random nonce per encryption via `rand::rng().fill_bytes()`. Uniqueness validated by `test_nonce_uniqueness` test.
- **Tamper detection**: AES-256-GCM authentication tag prevents ciphertext modification. Validated by `test_tampered_ciphertext_fails`.
- **Legacy plaintext acceptance**: Both `ValueType::try_from()` and `TryGetable::try_get_by()` accept non-prefixed strings as plaintext for backward compatibility. This is necessary for migration from unencrypted storage but has HA implications (see below).

## Code Quality

- **Error handling**: `CryptoError` enum with 9 variants, `rootcause::Report` wrapper, `impl_report_conversion!` for hex decode and UTF-8 errors. Compliant with project standards.
- **Test coverage**: 8 tests covering round-trip, nonce uniqueness, prefix detection, debug/display redaction, tamper detection, SeaORM integration, legacy plaintext, and NULL handling with `MockDatabase`.
- **Entity structure**: Clean, consistent SeaORM entity definitions with proper relations and `ActiveModelBehavior` implementations.
- **Custom types**: `RoleMapping` (`HashMap<String, String>` wrapper with JSON storage) and `EncryptedString` both implement full SeaORM integration (`ValueType`, `TryGetable`, `From<T> for Value`).

## Coding Standards Compliance

- Typed error enum (`CryptoError`) with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined (`src/crypto.rs:47`).
- `impl_report_conversion!` for cross-boundary errors -- compliant.
- No `#[allow()]` directives.

## High Availability Considerations

### Positive findings

- **`controller_event` is HA-aware**: Uses `source_controller_id: Uuid` (`src/entity/controller_event.rs:9`) and DB-managed auto-increment `id: i64` (`src/entity/controller_event.rs:8`). Concurrent inserts from multiple controllers are safe.
- **`mqtt_lease` is HA-aware**: Has `instance_id: String` (`src/entity/mqtt_lease.rs:11`) and `heartbeat_at: OffsetDateTime` (`src/entity/mqtt_lease.rs:12`) for multi-instance lease coordination.
- **`api_rate_limit` structure supports HA**: Simple key-based rate limit with `request_count`, `window_start`, `expires_at`. The atomic `INSERT ... ON CONFLICT DO UPDATE` pattern (in web-api, not here) handles concurrent updates.

### Concerns

- **Master key must be identical across all controller instances**: The `OnceLock<[u8; 32]>` (`src/crypto.rs:55`) is per-process. No validation exists at the entity/crypto layer to detect key mismatches between controllers. A misconfigured instance would silently write plaintext or fail to decrypt values encrypted by other instances.
- **`EncryptedString` plaintext fallback**: When encryption fails (`src/crypto.rs:227-230`) or master key is absent (`src/crypto.rs:232-235`), secrets are stored as plaintext. In HA, a misconfigured controller would write plaintext to the shared database while other controllers expect `ENC:v1:` prefixed values.
- **No optimistic concurrency controls**: Entities with mutable state (e.g., `settings_version`, `mqtt_client`, `update_history`) lack a `version` column or SeaORM optimistic locking. The `settings_version` table has counters (`version`, `global_version`, `revocation_version` at `src/entity/settings_version.rs:9-11`) but the entity definition does not use SeaORM's `OptimisticLock` trait.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| ~~DB-01~~ | ~~Medium~~ | ~~Security / HA~~ | ~~`EncryptedString` plaintext fallback on encryption failure or missing master key.~~ **FIXED.** `EncryptedString::new()` is now fallible and encrypts eagerly at construction time. The `From<EncryptedString> for Value` impl uses the pre-computed `db_value` and is infallible. All callers updated to propagate encryption errors. | `src/crypto.rs` |
| ~~DB-02~~ | ~~Medium~~ | ~~HA~~ | ~~Master key must be identical across all controller instances. No validation at the crypto layer to detect key mismatches. A controller with a wrong key would fail to decrypt values from other instances.~~ **FIXED.** `create_key_verification_token()` encrypts a known sentinel; `verify_key_verification_token()` decrypts and verifies it. Controller startup stores a verification token on first run and validates it on subsequent startups, failing with `MasterKeyMismatch` if the key is wrong. | `src/crypto.rs` |
| DB-03 | Medium | HA | No optimistic concurrency controls in entity layer. Entities like `settings_version`, `mqtt_client`, and `update_history` have mutable state but no `version` column or `updated_at`-based optimistic locking via SeaORM. | `src/entity/settings_version.rs:4-13` |
| DB-04 | Low | Code Quality | `RoleMapping` serialization fallback produces empty JSON object `{}` on failure instead of propagating the error. The `From<RoleMapping> for Value` trait constrains this similarly to `EncryptedString`. | `src/entity/oidc_provider.rs:35-41` |
| ~~DB-05~~ | ~~Low~~ | ~~Code Quality~~ | ~~String-typed columns that could use `DeriveActiveEnum` for type safety.~~ **FIXED.** All columns now use typed enums with `DeriveActiveEnum`: `mqtt_client.transport` → `MqttTransport`, `mqtt_client.connection_status` → `MqttClientConnectionStatus`, `session.token_type` → `SessionTokenType`, `pending_device_flow.status` → `DeviceAuthStatus`, `update_output_line.stream` → `OutputStreamType`. | Multiple entity files |
| DB-06 | Low | Code Quality | `available_version.extra` and `software_item.config_override` use `column_type = "JsonBinary"` in entities but the migration may use `json_null()`/`json()`. With SQLite this is inconsequential, but semantically inconsistent. | `src/entity/available_version.rs:14`, `src/entity/software_item.rs:13` |
| DB-07 | Info | Security | No key rotation mechanism. The `ENC:v1:` prefix suggests versioning was planned but not implemented. In HA, key rotation would require coordinated rollout across all controllers. | `src/crypto.rs:72` |
| DB-08 | Info | Security | PKCE verifier, nonce, and CSRF state in `pending_oidc_flow` are stored unencrypted as plain `Text`. These are short-lived ephemeral values with `expires_at`, so encryption is defense-in-depth, not critical. | `src/entity/pending_oidc_flow.rs:8-13` |
| DB-09 | Info | HA (Positive) | `controller_event` uses `source_controller_id` + auto-increment `id`. Concurrent inserts are safe. | `src/entity/controller_event.rs:8-9` |
| DB-10 | Info | HA (Positive) | `mqtt_lease` uses `instance_id` + `heartbeat_at` for multi-instance lease coordination. | `src/entity/mqtt_lease.rs:11-12` |

## Verdict

**Pass.** Crypto implementation is sound (AES-256-GCM, random nonces, tamper detection, secret redaction). DB-01 (plaintext fallback) has been resolved — `EncryptedString::new()` is now fallible with eager encryption, eliminating the silent data-corruption vector. DB-02 (master key mismatch) has been resolved — startup verification with a sentinel token detects wrong keys immediately. Optimistic locking (DB-03) should be addressed as part of HA hardening. The `DeriveActiveEnum` improvements (DB-05) are recommended for type safety.
