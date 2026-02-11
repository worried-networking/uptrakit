# Secrets Handling and Encryption at Rest

- Sensitive credentials are encrypted at rest using AES-256-GCM via `EncryptedString` (`crates/shared/db/src/crypto.rs`). `EncryptedString` wraps
  `SecretString` for secure debug/display redaction.
- `SecretString` (from `uptrakit-shared-types`) redacts values and exposes the inner string through `.expose_secret()`.
- Database columns using encryption:

  | Table | Column | Description |
  | --- | --- | --- |
  | `mqtt_clients` | `password` | MQTT broker password |
  | `oidc_providers` | `client_secret` | OIDC client secret |
  | `ca_certificates` | `key_pem` | CA private key |

- Ciphertext format: `ENC:v1:<hex(nonce || ciphertext || tag)>`. The `v1` marker enables future algorithm changes.
- Legacy plaintext detection allows old values to remain readable until rewritten.

## Master Key Management

- A 256-bit master key is required in production via `UPTRAKIT_MASTER_KEY` (64 hex characters) or `--master-key-file`.
- `init_master_key()` loads the key once at startup and caches it in a global `OnceLock`.
- The key is never logged or exposed in API responses.
- `--allow-plaintext-secrets` disables encryption (for development only) and logs a warning.

## Tokens and Secrets

- JWT signing keys live in the `auth.jwt_signing_key` settings entry (base64 encoded, global scope). File-based keys (`jwt_signing.key`) are migrated
  to the database automatically.
- Refresh tokens are stored hashed in `HttpOnly; Secure; SameSite=Strict` cookies and rotated on every use.
- A per-instance `TokenDenylist` in memory enables immediate JWT revocation by `jti` or per user. On logout, all tokens for the user are denied for
  the remaining lifetime.
- Agent/MQTT private keys are generated locally and never leave their hosts.
- CA private keys live in `CaKeyStore` with `zeroize` guard, accessed only by signing components.
