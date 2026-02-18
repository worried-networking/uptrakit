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
type (defined in `crates/shared/db/src/crypto.rs`). `EncryptedString` stores the plaintext (wrapped in `SecretString`
for redacted debug/display) alongside a pre-computed database representation (encrypted ciphertext, or plaintext in dev
mode).

`EncryptedString::new()` is **fallible** — it encrypts eagerly at construction time and returns
`Result<Self, Report<CryptoError>>`. This prevents silent plaintext fallback: if the master key is present but
encryption fails, the error propagates immediately instead of silently storing unencrypted secrets in the database.
When no master key is configured (development mode with `--allow-plaintext-secrets`), construction succeeds with
plaintext as the database value.

Database columns using encryption:

| Table | Column | Description |
| --- | --- | --- |
| `mqtt_clients` | `password` | MQTT broker password |
| `oidc_providers` | `client_secret` | OIDC client secret |
| `ca_certificates` | `key_pem` | CA private key |
| `pending_oidc_flows` | `pkce_verifier` | PKCE code verifier for in-flight OIDC authorization |
| `ssh_hosts` | `private_key` | SSH private key (agent-ssh local DB) |

Ciphertext format: `ENC:v1:<hex(nonce || ciphertext || tag)>`. The `v1` marker enables future algorithm changes.
Legacy plaintext detection allows old values to remain readable until rewritten.

## Master Key Management

- A 256-bit master key is required in production via `UPTRAKIT_MASTER_KEY` (64 hex characters) or `--master-key-file`.
- `init_master_key()` loads the key once at startup and caches it in a global `OnceLock`. It accepts
  `Zeroizing<[u8; 32]>` so the key bytes are scrubbed from memory when intermediate copies are dropped
  (defense-in-depth — the `OnceLock` static has `'static` lifetime). It returns `Report<CryptoError>` — see the
  `CryptoError` enum in `crates/shared/db/src/crypto.rs` for the full set of typed error variants (e.g.
  `AlreadyInitialized`, `NotInitialized`, `KeyCreation`, `Encryption`, `Decryption`, `MasterKeyMismatch`).
- The key is never logged or exposed in API responses.
- `--allow-plaintext-secrets` disables encryption (for development only) and logs a warning. When `EncryptedString::new()`
  stores plaintext (no master key configured), a `tracing::warn!` is emitted for observability.

| Method | Details |
| --- | --- |
| `UPTRAKIT_MASTER_KEY` env var | 64-character hex string (32 bytes) |
| `--master-key-file` CLI arg | Path to a file containing the 64-character hex key |

### Master Key Verification (HA Safety)

In multi-controller (HA) deployments, all instances must share the same master key. A misconfigured
instance using a different key would silently fail to decrypt values encrypted by other instances.

To prevent this, the controller performs **startup key verification**:

1. On first startup (when no verification token exists), `create_key_verification_token()` encrypts a
   known sentinel value (`uptrakit-master-key-ok-v1`) and stores the ciphertext in the
   `crypto.master_key_verification` settings entry.
2. On subsequent startups, `verify_key_verification_token()` reads the stored ciphertext, decrypts it,
   and verifies it matches the expected sentinel. If decryption fails or the plaintext does not match,
   the controller aborts with a `MasterKeyMismatch` error and a clear diagnostic message.

This ensures that key mismatches are detected immediately at startup rather than surfacing as
mysterious decryption failures at runtime. The verification token is stored as a global (non-tenant-scoped)
setting under `SettingKey::MasterKeyVerification`.

See also: [Cross-Controller Communication](../development/cross-controller-comm.md) for other HA
considerations.

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
| `crates/shared/db/src/crypto.rs` | `EncryptedString` type, `init_master_key()`, AES-256-GCM encrypt/decrypt, key verification |
| `crates/shared/types/src/secret_string.rs` | `SecretString` newtype with redacted Debug/Display |
| `crates/ui/web-api/src/setting_key.rs` | `SettingKey::MasterKeyVerification` — stores the key verification token |
| `crates/core/controller/src/startup.rs` | `verify_master_key()` — startup phase that validates the master key |
