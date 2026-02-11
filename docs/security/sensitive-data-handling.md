# Sensitive string handling

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

## Encryption at rest

Sensitive credentials stored in the database are encrypted using AES-256-GCM via the `EncryptedString` SeaORM custom
type (defined in `crates/shared/db/src/crypto.rs`). `EncryptedString` wraps `SecretString` internally, inheriting its
debug/display redaction.

## Master key

A 256-bit master encryption key is **mandatory in production** for the controller. It can be provided in two ways:

| Method | Details |
| --- | --- |
| `UPTRAKIT_MASTER_KEY` env var | 64-character hex string (32 bytes) |
| `--master-key-file` CLI arg | Path to a file containing the 64-character hex key |

`init_master_key()` is called at controller startup. For development only, the controller can start without a key by
passing `--allow-plaintext-secrets` (disables encryption at rest and logs a warning). When the flag is set and a key is
provided, a warning is still logged but encryption remains enabled.

## Encrypted fields

| Table | Column | Description |
| --- | --- | --- |
| `mqtt_clients` | `password` | MQTT broker password |
| `oidc_providers` | `client_secret` | OIDC client secret |
| `ca_certificates` | `key_pem` | CA private key PEM |

## Ciphertext format

Ciphertext is stored as `"ENC:v1:<hex(nonce || ciphertext || tag)>"`. On read, `EncryptedString` transparently detects
legacy plaintext values (no `ENC:v1:` prefix) and passes them through, enabling gradual upgrade of existing data without
migration.

## Key files

| File | Purpose |
| --- | --- |
| `crates/shared/db/src/crypto.rs` | `EncryptedString` type, `init_master_key()`, AES-256-GCM encrypt/decrypt |
