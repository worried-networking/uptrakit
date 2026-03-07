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
2. With the master key (KEK), the attacker can unwrap all DEKs from the
   `data_encryption_keys` table and decrypt all `ENC:v3:` ciphertext values
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
- **`ENC:v3:` context-bound envelope encryption.** *(Implemented)* All encrypted
  database columns and settings use `ENC:v3:` format with envelope encryption
  (DEK wraps data) and per-field AAD strings. A ciphertext produced for one column
  cannot be used as a valid ciphertext for another, even if an attacker obtains the
  master key and attempts to relocate ciphertexts.
- **Startup warning for env-var key source.** *(Implemented)* When the master key is
  loaded from `UPTRAKIT_MASTER_KEY` env var (without `--master-key-file`), a `WARN`-
  level log message is emitted at startup marking env-var delivery as **deprecated**,
  nudging operators toward the more secure file-based delivery method.
- **Environment variable cleared after reading.** *(Implemented)* Both the controller
  and SSH agent clear `UPTRAKIT_MASTER_KEY` from the process environment immediately
  after reading it during single-threaded startup. This reduces the window during
  which the key is visible via `/proc/pid/environ` or `ps eww`.
- **Intermediate hex string zeroization.** *(Implemented)* The
  `read_master_key_hex()` helper returns `Zeroizing<String>` so that the raw hex
  representation of the master key is scrubbed from heap memory on drop. This closes
  a gap where the hex string could survive on the heap after an error path between
  reading the key and wrapping it in `SecretString`.
- **Broad trusted-proxy CIDR warning.** *(Implemented)* On startup, if any
  `--trusted-proxy` CIDR has an overly broad prefix length (IPv4 /8 or less, IPv6
  /32 or less, or /0 for either family), a `WARN`-level log message is emitted.
  Overly broad CIDRs undermine IP-based rate limiting and audit logging by trusting
  a large portion of the internet to set forwarded headers.

## Residual risk

- **Process memory exposure.** The `OnceLock` static has `'static` lifetime, so the
  key material (KEK and unwrapped DEKs) is present in process memory for the entire
  lifetime of the controller. A memory dump, core dump, or `/proc/pid/mem` read
  exposes the keys.
- **Environment variable visibility.** `UPTRAKIT_MASTER_KEY` is still accepted; it is
  now warned about at startup (deprecated) and cleared from the process environment
  after reading, but operators may still use it in automation without adopting
  `--master-key-file`.
- **SSH agent uses independent key.** The SSH agent's master key is separate from the
  controller's. Compromise of one does not expose the other — but operators may
  reuse the same key for convenience, negating this isolation.

## Recommended improvements

- ~~Complete the `ENC:v2:` migration for all `EncryptedString` columns~~ — **Done.**
  Automatic v3 re-encryption runs on startup (no CLI flag needed).
- ~~Add a master key rotation workflow~~ — **Done.** The `--rotate-master-key-file`
  flag re-wraps DEKs with a new KEK in O(1) time. See
  [Key Rotation](../security/key-rotation.md).
- Support external key management systems (KMS) such as AWS KMS, HashiCorp Vault, or
  PKCS#11 HSMs for master key storage, removing the key from process memory and
  environment variables.
- Document recommended operator practices: use `--master-key-file` with restrictive
  permissions rather than environment variables; disable core dumps in production;
  ensure `/proc/pid/environ` is not readable by non-root users.

## References

- [Secrets and Encryption](../security/secrets-and-encryption.md)
- [Key Rotation](../security/key-rotation.md)
- [Cryptography](../security/cryptography.md)
- [SSH Agent Secrets](../security/ssh-agent-secrets.md)
- `crates/shared/crypto/src/lib.rs` — `init_master_key()`, `DataKeyRing`,
  DEK wrap/unwrap, `encrypt_str()`, `decrypt_str()`
- `crates/core/controller/src/startup.rs` — `verify_master_key()`,
  `init_data_key_ring()`, `rotate_master_key()`
- `crates/core/controller/src/reencrypt.rs` — automatic v3 re-encryption
