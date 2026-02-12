# Secrets Handling and Encryption at Rest

## SecretString

`SecretString` (defined in `crates/shared/types/src/secret_string.rs`, re-exported by `uptrakit-internal-wire`) is a
newtype wrapper that prevents accidental logging of sensitive values:

- `Debug` output: `SecretString(***)`
- `Display` output: `***REDACTED***`
- Access the inner value via `.expose_secret()` (returns `&str`)
- Transparent serde: JSON wire format is unchanged (plain string)
- Used for: enrollment tokens, enrollment secrets, MQTT credentials in wire types

Wire fields using `SecretString`: `EnrollPayload.enrollment_token`, `EnrolledPayload.enrollment_secret`,
`MqttTenantConfig.username`, `MqttTenantConfig.password`.

## Encryption at Rest

Sensitive credentials stored in the database are encrypted using AES-256-GCM via the `EncryptedString` SeaORM custom
type (defined in `crates/shared/db/src/crypto.rs`). `EncryptedString` wraps `SecretString` internally, inheriting its
debug/display redaction.

Database columns using encryption:

| Table | Column | Description |
| --- | --- | --- |
| `mqtt_clients` | `password` | MQTT broker password |
| `oidc_providers` | `client_secret` | OIDC client secret |
| `ca_certificates` | `key_pem` | CA private key |

Ciphertext format: `ENC:v1:<hex(nonce || ciphertext || tag)>`. The `v1` marker enables future algorithm changes.
Legacy plaintext detection allows old values to remain readable until rewritten.

## Master Key Management

- A 256-bit master key is required in production via `UPTRAKIT_MASTER_KEY` (64 hex characters) or `--master-key-file`.
- `init_master_key()` loads the key once at startup and caches it in a global `OnceLock`.
- The key is never logged or exposed in API responses.
- `--allow-plaintext-secrets` disables encryption (for development only) and logs a warning.

| Method | Details |
| --- | --- |
| `UPTRAKIT_MASTER_KEY` env var | 64-character hex string (32 bytes) |
| `--master-key-file` CLI arg | Path to a file containing the 64-character hex key |

## Tokens and Secrets

- JWT signing keys live in the `auth.jwt_signing_key` settings entry (base64 encoded, global scope). File-based keys
  (`jwt_signing.key`) are migrated to the database automatically.
- Refresh tokens are stored hashed in `HttpOnly; Secure; SameSite=Strict` cookies and rotated on every use.
- A per-instance `TokenDenylist` in memory enables immediate JWT revocation by `jti` or per user. On logout, all tokens
  for the user are denied for the remaining lifetime.
- Agent/MQTT private keys are generated locally and never leave their hosts.
- CA private keys live in `CaKeyStore` with `zeroize` guard, accessed only by signing components.

## Key Files

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/crypto.rs` | `EncryptedString` type, `init_master_key()`, AES-256-GCM encrypt/decrypt |
| `crates/shared/types/src/secret_string.rs` | `SecretString` newtype with redacted Debug/Display |
