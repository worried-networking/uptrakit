# ATK-03: Master Key Compromise

| Field | Value |
| --- | --- |
| Severity | Critical |
| Attack surface | Cryptography (AES-256-GCM master key) |
| Prerequisites | Access to the `UPTRAKIT_MASTER_KEY` env var, `--master-key-file`, or controller process memory |
| STRIDE | Information Disclosure |

## Attack description

1. The attacker obtains the 256-bit master key through one of:
   - Reading the `UPTRAKIT_MASTER_KEY` environment variable from the process
     environment, container manifest, or orchestration config.
   - Reading the `--master-key-file` from disk (requires filesystem access to the
     controller host).
   - Dumping the controller process memory (the key is stored in a global `OnceLock`
     with `Zeroizing` wrapper, but the static has `'static` lifetime so the key is
     present for the entire process lifetime).
2. With the master key, the attacker can decrypt all `ENC:v1:<hex>` ciphertext values
   in the database.
3. The attacker gains access to all encrypted secrets.

## Worst-case impact

Compromise of the master key exposes every encrypted credential in the system:

| Table | Column | Exposed secret |
| --- | --- | --- |
| `ca_certificates` | `key_pem` | CA private key (can forge agent/MQTT certificates) |
| `oidc_providers` | `client_secret` | OIDC client secrets (can impersonate the app to IdPs) |
| `mqtt_clients` | `password` | MQTT broker passwords |
| `mqtt_clients` | `ca_cert` | Custom MQTT CA certificates |
| `pending_oidc_flows` | `pkce_verifier` | In-flight OIDC PKCE verifiers |
| `notification_channels` | `config` | Webhook secrets, Telegram bot tokens, HMAC keys |
| `global_settings` | `auth.jwt_signing_key` | JWT signing key (can forge arbitrary JWT tokens) |

With the CA private key, the attacker can:

- **Forge agent certificates** and enroll rogue agents without the enrollment flow.
- **Forge server certificates** and perform man-in-the-middle attacks between agents
  and the controller.
- **Sign CRLs** and revoke legitimate agent certificates, causing denial of service.

With the JWT signing key, the attacker can:

- **Forge JWT access tokens** with arbitrary permissions, including `owner` role.
- **Bypass all API authorization** and perform any administrative action.

## Current mitigations

- **Key is never logged or exposed in API responses.** The master key is wrapped in
  `Zeroizing<[u8; 32]>` and stored in a `OnceLock`. It does not appear in debug
  output, tracing spans, or error messages.
- **Filesystem permissions.** The `--master-key-file` should be stored with `0o600`
  permissions, readable only by the service user.
- **HA key verification.** On startup, the controller encrypts a sentinel value
  (`uptrakit-master-key-ok-v1`) and verifies it against the stored token. A key
  mismatch is a fatal startup error, preventing an HA node with a wrong key from
  silently corrupting data.
- **No plaintext fallback in production.** A missing master key is a hard failure.
  The `--allow-plaintext-secrets` flag is required to bypass encryption and produces
  a runtime warning.
- **AES-256-GCM authentication.** Ciphertext includes a 16-byte GCM tag that detects
  tampering. An attacker cannot modify encrypted values without the key.
- **Per-value random nonces.** Each encryption uses a fresh 12-byte random nonce,
  preventing ciphertext comparison attacks.
- **`ENC:v2:` context-bound ciphertexts.** *(Implemented)* The JWT signing key and
  master-key verification token are now encrypted using `ENC:v2:` format with a
  per-field AAD string. A ciphertext produced for `"uptrakit:settings:jwt_signing_key"`
  cannot be used as a valid ciphertext for the key-verification slot, and vice versa,
  even if an attacker obtains the master key and attempts to relocate ciphertexts.
- **Startup warning for env-var key source.** *(Implemented)* When the master key is
  loaded from `UPTRAKIT_MASTER_KEY` env var (without `--master-key-file`), a `WARN`-
  level log message is emitted at startup, nudging operators toward the more secure
  file-based delivery method.

## Residual risk

- **Static key with no rotation.** The master key is a single static value with no
  built-in rotation mechanism. Compromise is permanent until the key is manually
  changed and all encrypted values are re-encrypted.
- **`EncryptedString` DB columns still use `ENC:v1:` (empty AAD).** CA private keys,
  OIDC client secrets, MQTT passwords, webhook secrets, etc. are not yet bound to a
  column context. Migration to `ENC:v2:` per-column AAD is tracked in `TODO.md`.
- **Process memory exposure.** The `OnceLock` static has `'static` lifetime, so the
  key material is present in process memory for the entire lifetime of the
  controller. A memory dump, core dump, or `/proc/pid/mem` read exposes the key.
- **Environment variable visibility.** `UPTRAKIT_MASTER_KEY` is still accepted; it is
  now warned about at startup but not prohibited. Operators may still use it in
  automation without adopting `--master-key-file`.
- **SSH agent uses independent key.** The SSH agent's master key is separate from the
  controller's. Compromise of one does not expose the other — but operators may
  reuse the same key for convenience, negating this isolation.

## Recommended improvements

- Complete the `ENC:v2:` migration for all `EncryptedString` columns, binding each
  ciphertext to its table and column via a dedicated AAD string.
- Add a master key rotation workflow that re-encrypts all stored values under a new
  key, with a migration period where both old and new keys are accepted.
- Support external key management systems (KMS) such as AWS KMS, HashiCorp Vault, or
  PKCS#11 HSMs for master key storage, removing the key from process memory and
  environment variables.
- Document recommended operator practices: use `--master-key-file` with restrictive
  permissions rather than environment variables; disable core dumps in production;
  ensure `/proc/pid/environ` is not readable by non-root users.

## References

- [Secrets and Encryption](../security/secrets-and-encryption.md)
- [Cryptography](../security/cryptography.md)
- [SSH Agent Secrets](../security/ssh-agent-secrets.md)
- `crates/shared/crypto/src/lib.rs` — `init_master_key()`, `encrypt_value()`,
  `decrypt_value()`
- `crates/core/controller/src/startup.rs` — `verify_master_key()`
- `crates/core/controller/src/reencrypt.rs` — startup re-encryption of legacy values
