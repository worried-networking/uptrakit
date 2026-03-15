# Secrets Handling and Encryption at Rest

## SecretString

`SecretString` (defined in `crates/shared/types/src/secret_string.rs`, re-exported by `uptrakit-internal-wire` and
`uptrakit-web-api-types`) is a newtype wrapper that prevents accidental logging of sensitive values:

- `Debug` output: `SecretString(***)`
- `Display` output: `***REDACTED***`
- Access the inner value via `.expose_secret()` (returns `&str`)
- Transparent serde: JSON wire format is unchanged (plain string)
- `ZeroizeOnDrop`: memory is zeroed when the value is dropped
- Feature-gated `ToSchema` derive for OpenAPI schema generation (`openapi` feature)
- Used for: enrollment tokens, enrollment secrets, MQTT credentials in wire types, **and all secret fields in HTTP
  API request/response types** (passwords, tokens, client secrets, access/refresh tokens)

### Security caveats

When using `SecretString`, be aware of the following implementation trade-offs:

- **`Clone`** creates a new heap allocation containing the secret. Each clone must be properly dropped (not
  leaked or stored in a non-zeroizing container) to maintain the zeroize guarantee.
- **`Serialize`** is transparent (`#[serde(transparent)]`) -- it emits the plaintext value. Never serialize
  structs containing `SecretString` to logs or debug output. Use `Debug` formatting instead, which renders
  `SecretString(***)`.
- **`PartialEq`** uses standard `String` comparison, which short-circuits on the first mismatched byte. Do
  **not** use `PartialEq` for authentication comparisons where timing side channels are a concern; use a
  constant-time comparison function instead (e.g., `subtle::ConstantTimeEq` or
  `argon2::password_hash::PasswordVerifier`).

Wire fields using `SecretString`: `EnrollPayload.enrollment_token`, `EnrolledPayload.enrollment_secret`,
`MqttTenantConfig.username`, `MqttTenantConfig.password`.

HTTP API fields using `SecretString` (in `uptrakit-web-api-types`): `RegisterRequest.password`,
`LoginRequest.password`, `AuthResponse.access_token`, `AuthResponse.refresh_token`,
`RefreshResponse.access_token`, `RefreshResponse.refresh_token`, `LogoutRequest.refresh_token`,
`RefreshRequest.refresh_token`, `CreateOidcProviderRequest.client_secret`,
`UpdateOidcProviderRequest.client_secret`, `OidcLinkRequest.link_token`, `OidcLinkRequest.password`,
`OidcCompleteRegistrationRequest.registration_code`,
`OidcCompleteRegistrationRequest.registration_token`, `RegisterRequest.registration_token`,
`UpdateRegistrationSettingsRequest.token`, `CreateMqttClientRequest.password`,
`CreateApiTokenResponse.token`, `EnrollmentTokenResponse.token`,
`MqttEnrollmentTokenResponse.token`, `DeviceAuthPollResponse.token`.

### Credential-holding structs and `Debug`

Any internal struct that holds a secret value **must** use `SecretString` (not `String`) for that
field. This ensures the masking guarantee is enforced at the type level rather than relying on a
hand-written `Debug` impl that could accidentally be removed or forgotten.

**Required pattern:**

```rust
// ✓ Correct — Debug is auto-derived; SecretString's Debug impl emits "***"
#[derive(Debug)]
struct SmtpSettings {
    host: String,
    password: Option<SecretString>,
}

// ✗ Wrong — Debug leaks the password to tracing logs and panic messages
#[derive(Debug)]
struct SmtpSettings {
    host: String,
    password: Option<String>,
}
```

When reading the password back (e.g., passing it to a mail client), call `.expose_secret()`:

```rust
if let Some(pw) = &config.password {
    client.set_password(pw.expose_secret());
}
```

See also: [Coding Standards — Credential-Holding Types and Debug](../development/coding-standards.md#credential-holding-types-and-debug).

### Entity field usage

The `User` entity model (`crates/shared/db/src/entity/user.rs`) uses `SecretString` and `MaskedEmail`
for sensitive fields:

- **`password_hash: Option<SecretString>`** -- the user's password hash is wrapped in `SecretString`,
  ensuring it is redacted in `Debug`/`Display` output and zeroed from memory on drop. Access the raw
  hash via `.expose_secret()`.
- **`email: MaskedEmail`** -- the user's email address is wrapped in `MaskedEmail`, which masks the
  local part in `Debug`/`Display` output (e.g., `an***@example.com`) while preserving the full value
  for serialization, API responses, and database storage. Access the full email via `.expose_email()`.

See also: [Coding Standards](../development/coding-standards.md).

## MaskedEmail

`MaskedEmail` (defined in `crates/shared/types/src/masked_email.rs`, re-exported by `uptrakit-shared-types`)
is a newtype wrapper for email addresses that masks the local part in `Debug` and `Display` output while
preserving the full value for serialization and database storage.

- `Debug` output: `MaskedEmail(an***@example.com)`
- `Display` output: `an***@example.com`
- Access the full, unmasked email via `.expose_email()` (returns `&str`)
- Transparent serde: JSON wire format is unchanged (plain string)
- Implements `FromStr` with basic email validation (requires exactly one `@` with non-empty local and domain)
- Feature-gated SeaORM integration (`sea-orm` feature) -- see
  [Coding Standards](../development/coding-standards.md#seaorm-integration-for-custom-types)

The masking algorithm splits the local part (before `@`) by delimiters (`.`, `_`, `+`, `-`), shows
`ceil(len/3)` leading characters per segment (minimum 1) followed by `***`, and always shows the
domain in full. Original delimiters are preserved.

## Encryption at Rest

Sensitive credentials stored in the database are encrypted using AES-256-GCM via the `EncryptedString` SeaORM custom
type (defined in `crates/shared/crypto/src/lib.rs`). `EncryptedString` stores the plaintext (wrapped in `SecretString`
for redacted debug/display) alongside a pre-computed database representation (encrypted ciphertext).

`EncryptedString::new(plaintext, aad)` is **fallible** — it encrypts eagerly at construction time and returns
`Result<Self, Report<CryptoError>>`. The master key **must** be initialized before calling this function. If no master
key has been configured, `EncryptedString::new()` returns `Err(CryptoError::NotInitialized)`. There is no plaintext
fallback or development mode: a missing master key is a hard failure that must be resolved before the service starts.

Database columns using encryption:

| Table | Column | AAD string | Description |
| --- | --- | --- | --- |
| `mqtt_clients` | `password` | `uptrakit:mqtt_clients:password` | MQTT broker password |
| `mqtt_clients` | `ca_cert_pem` | `uptrakit:mqtt_clients:ca_cert_pem` | Custom MQTT CA certificate |
| `oidc_providers` | `client_secret` | `uptrakit:oidc_providers:client_secret` | OIDC client secret |
| `ca_certificates` | `key_pem` | `uptrakit:ca_certificates:key_pem` | CA private key |
| `pending_oidc_flows` | `pkce_verifier` | `uptrakit:pending_oidc_flows:pkce_verifier` | PKCE code verifier for in-flight OIDC authorization |
| `ssh_hosts` | `private_key` | `uptrakit:ssh_hosts:private_key` | SSH private key (agent-ssh local DB) |
| `notification_channels` | `config` | `uptrakit:notification_channels:config` | Channel config JSON (bot tokens, webhook secrets, HMAC keys) |

### Envelope encryption

The system uses **envelope encryption**: a master key (KEK — key encryption key) wraps data encryption
keys (DEKs) stored in the `data_encryption_keys` table. Data is encrypted with DEKs, never directly
with the KEK. This enables O(1) master key rotation — re-wrap the DEKs only, no data re-encryption.

```text
Master Key (KEK, 32 bytes, from file/env)
  └── wraps → DEK (32 bytes, stored encrypted in data_encryption_keys table)
                └── encrypts → ENC:v3:<key_id>:<hex(nonce || ciphertext || tag)>
```

Each DEK is identified by a `key_id` — the first 8 hex characters of `SHA-256(DEK)`. The `key_id` is
embedded in the ciphertext prefix so the correct DEK can be looked up at decryption time.

### Ciphertext formats

Three wire formats coexist for backward compatibility:

| Format | Prefix | AAD | Key | Used by |
| --- | --- | --- | --- | --- |
| v1 | `ENC:v1:` | empty | KEK direct | Legacy (read-only during migration) |
| v2 | `ENC:v2:` | caller-supplied | KEK direct | Migration compat, key verification |
| v3 | `ENC:v3:<key_id>:` | caller-supplied | DEK (envelope) | Current default |

The `is_encrypted()` helper returns `true` for all three prefixes.

`ENC:v3:` ciphertexts are **context-bound**: the AAD string is mixed into the GCM authentication tag.
A ciphertext encrypted for one purpose (e.g. `"uptrakit:mqtt_clients:password"`) cannot be used as
a valid ciphertext for any other purpose. This prevents ciphertext relocation attacks where an attacker
moves an encrypted value from one column to another.

### Low-level helpers: `encrypt_str` and `decrypt_str`

Two public functions in `crates/shared/crypto/src/lib.rs` provide lower-level access to the AES-256-GCM
primitive:

- **`pub fn encrypt_str(plaintext: &str, aad: &str) -> Result<String>`** — encrypts with a
  caller-supplied AAD. Produces `ENC:v3:` when the data key ring is available, `ENC:v2:` otherwise.
- **`pub fn decrypt_str(stored: &str, aad: &str) -> Result<String>`** — decrypts all three formats:
  `ENC:v3:` ciphertexts look up the DEK by `key_id`; `ENC:v2:` uses the KEK directly;
  `ENC:v1:` is read for backward compatibility during migration.

These helpers are **not** a replacement for `EncryptedString`. Use `EncryptedString` for structured entity
fields backed by SeaORM columns; use `encrypt_str` / `decrypt_str` only for ad-hoc string values (such as
settings entries) that cannot use the SeaORM custom type.

### Automatic re-encryption to v3

When the controller starts with a master key and a data key ring, a re-encryption routine runs
automatically. It scans all encrypted columns listed above (except `ssh_hosts.private_key`, which
lives in the agent-ssh local DB) and all encrypted settings entries for values that are not yet
in `ENC:v3:` format. Source formats handled:

- **Plaintext** (no `ENC:` prefix) — legacy values from before encryption was enabled.
- **`ENC:v1:`** — oldest format (empty AAD, KEK-direct).
- **`ENC:v2:`** — intermediate format (caller AAD, KEK-direct).

All are upgraded to `ENC:v3:` (caller AAD, DEK envelope). Values already at `ENC:v3:` are skipped.

The routine has these properties:

- **Automatic** — runs on every startup, no CLI flag needed.
- **Idempotent** — already-v3 values are skipped via `EncryptedString::needs_v3_upgrade()`.
- **HA-safe** — concurrent controllers may race on the same row; the last writer wins,
  which is fine because the result is always a correctly encrypted v3 value.
- **Fault-tolerant** — errors on individual rows are logged at `warn` level and skipped.
  The controller still starts successfully.
- **Observable** — on completion, if any values were re-encrypted, an `info`-level log
  line reports the total count and per-table breakdowns.

The SSH agent performs the same automatic migration for `ssh_hosts.private_key` in its
local SQLite database.

Implementation: `crates/core/controller/src/reencrypt.rs` (controller columns and settings),
`crates/core/agent-ssh/src/main.rs` (SSH agent column).

See also: [Secure Development](secure-development.md) for coding standards related to
encryption.

## Master Key Management

- A 256-bit master key (KEK) is required in production via `UPTRAKIT_MASTER_KEY` (64 hex characters) or `--master-key-file`.
- `init_master_key()` loads the key once at startup and caches it in a global `OnceLock`. It accepts
  `Zeroizing<[u8; 32]>` so the key bytes are scrubbed from memory when intermediate copies are dropped
  (defense-in-depth — the `OnceLock` static has `'static` lifetime). It returns `Report<CryptoError>` — see the
  `CryptoError` enum in `crates/shared/crypto/src/lib.rs` for the full set of typed error variants (e.g.
  `AlreadyInitialized`, `NotInitialized`, `KeyCreation`, `Encryption`, `Decryption`, `MasterKeyMismatch`).
- The key is never logged or exposed in API responses.
- A missing or uninitialized master key is a **fatal startup error**. All components that call `EncryptedString::new()`,
  `encrypt_str()`, or `decrypt_str()` will receive `Err(CryptoError::NotInitialized)` and must propagate this
  failure — there is no plaintext fallback.

### Master Key Rotation

The `--rotate-master-key-file` CLI flag (available on both the controller and SSH agent) performs
O(1) master key rotation by re-wrapping all DEKs with the new KEK. No data is re-encrypted —
only the DEK wrappers change.

See [Key Rotation](key-rotation.md) for the full procedure, including HA rolling restart
instructions.

| Method | Details |
| --- | --- |
| `UPTRAKIT_MASTER_KEY` env var | 64-character hex string (32 bytes). **Deprecated — not recommended for production.** Env vars are visible in `/proc/pid/environ`, container inspection output, and orchestration manifests. The variable is cleared from the process environment after reading. A `WARN`-level deprecation message is emitted at startup when this method is used without `--master-key-file`. |
| `--master-key-file` CLI arg | Path to a file containing the 64-character hex key. Use `chmod 0600` to restrict to the service user. **Recommended method.** |

### Master Key Verification (HA Safety)

In multi-controller (HA) deployments, all instances must share the same master key. A misconfigured
instance using a different key would silently fail to decrypt values encrypted by other instances.

To prevent this, the controller performs **startup key verification**:

1. On first startup (when no verification token exists), `create_key_verification_token()` encrypts a
   known sentinel value (`uptrakit-master-key-ok-v1`) using `ENC:v2:` with AAD
   `"uptrakit:master-key-verification"` and stores the ciphertext in the
   `crypto.master_key_verification` settings entry using `insert_setting_if_absent()` (INSERT with
   ON CONFLICT DO NOTHING). If another controller instance raced and stored a token first, the current
   instance detects the conflict, re-reads the stored token, and verifies it against the current key.
2. On subsequent startups, `verify_key_verification_token()` reads the stored ciphertext, decrypts it,
   and verifies it matches the expected sentinel. It accepts both `ENC:v2:` (current, context-bound)
   and `ENC:v1:` (legacy installations) tokens for backward compatibility.
   If decryption fails or the plaintext does not match, the controller aborts with a `MasterKeyMismatch`
   error and a clear diagnostic message.

This ensures that key mismatches are detected immediately at startup rather than surfacing as
mysterious decryption failures at runtime. The verification token is stored as a global (non-tenant-scoped)
setting under `SettingKey::MasterKeyVerification`.

See also: [Cross-Controller Communication](../development/cross-controller-comm.md) for other HA
considerations.

## Bearer Token Hashing

Short-lived bearer tokens used in pending authentication flows are stored as SHA-256 hashes rather than plaintext.
This prevents an attacker with database access from using leaked tokens to complete authentication flows.

| Table | Token field | Hash column | Notes |
| --- | --- | --- | --- |
| `pending_device_flows` | `device_code` | `device_code_hash` | `user_code` remains unhashed (short-lived, user-facing, consonant alphabet) |
| `pending_account_links` | `link_token` | `link_token_hash` | |
| `pending_oidc_token_exchanges` | `exchange_code` | `exchange_code_hash` | |
| `pending_oidc_registrations` | `registration_code` | `registration_code_hash` | |

All four tables use a UUID `id` as primary key and a `*_hash TEXT NOT NULL UNIQUE` column for hash-based lookups.
The hashing uses the same `hash_token()` function (SHA-256, hex-encoded) used by `api_token` entities.

Lookup pattern: callers hash the raw token with `hash_token()` and filter by the hash column. The raw token is
never stored in the database.

See also: [Auth Flows](../api/auth-flows.md) for the authentication flow descriptions.

## Credential Capabilities and ServiceCredentials

Services advertising credential capabilities (`DatabaseAccess`, `NatsAccess`, `MasterKeyAccess`) receive
a `ServiceCredentials` message from the controller after mTLS authentication. This message carries
infrastructure secrets:

| Capability | Field | Content |
| --- | --- | --- |
| `DatabaseAccess` | `db_url` | Database connection string (contains credentials) |
| `NatsAccess` | `nats_url` | NATS server URL |
| `MasterKeyAccess` | `master_key_hex` | 256-bit master encryption key as 64-char hex |

### Security invariants

- **Never published to NATS.** `is_credential_message()` in `NotificationService` filters
  `ServiceCredentials` from NATS publication. Credentials are delivered exclusively via the
  authenticated WebSocket connection.
- **Capability-gated.** The controller only populates fields matching the service's capabilities.
  A service without `MasterKeyAccess` will not receive the master key, even if it has other
  credential capabilities.
- **Admin approval required.** The frontend displays per-capability security warnings when
  approving services with credential capabilities (e.g., "This service will receive direct
  database access credentials."). Only trusted services should be approved.
- **WebSocket-only delivery.** Follows the same pattern as MQTT credential messages
  (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`), which are also local-only.

Currently, the only service using credential capabilities is the external scheduler
(`uptrakit-scheduler`), which advertises all five capabilities: `Scheduler`, `DatabaseAccess`,
`NatsAccess`, `MasterKeyAccess`, `CaManagement`.

See also: [External Scheduler Deployment](../end-user/deployment/external-scheduler.md),
[Cross-Controller Communication](../development/cross-controller-comm.md).

## Tokens and Secrets

- JWT signing keys live in the `auth.jwt_signing_key` settings entry (base64 encoded, global scope). File-based keys
  (`jwt_signing.key`) are migrated to the database automatically.
- Refresh tokens are stored hashed in `HttpOnly; Secure; SameSite=Strict` cookies and rotated on every use.
- A per-instance `TokenDenylist` in memory enables immediate JWT revocation by `jti` or per user. On logout, all tokens
  for the user are denied for the remaining lifetime.
- Agent/MQTT private keys are generated locally and never leave their hosts.
- CA private keys live in `CaKeyStore` with `zeroize` guard, accessed only by signing components.

## NATS Transport Security

NATS carries cross-controller `ControllerMessage` payloads — software state updates, CA rotation
requests, and (in multi-controller HA deployments) messages that previously traversed the
authenticated WebSocket. Because NATS sits on the internal network, not the public internet, it is
sometimes misconfigured with plaintext transport.

### Requirements

| Environment | Requirement |
| --- | --- |
| Production | `nats-tls://` scheme, or `nats://` with `tls_required: true` on the NATS server |
| Development / CI | `nats://` is accepted; a warning is emitted |

### Operator warning

`NatsConnection::connect()` (`crates/shared/nats/src/connection.rs`) inspects the URL scheme at
startup. When the scheme is `nats` (plaintext), it emits a `tracing::warn!` with a pointer to this
document:

```text
WARN uptrakit_nats::connection: connecting to NATS over plaintext (nats://);
     use nats-tls:// or enable TLS on the server side in production
```

This warning fires on every startup, not just the first connection attempt, so it is visible in
monitoring and log aggregators.

### Why plaintext NATS is risky

NATS messages are **not** end-to-end encrypted at the application layer. All encryption relies on
the transport. A plaintext `nats://` connection exposes:

- Software state payloads (`software_states`) — reveals which packages are installed and which are
  outdated on managed hosts.
- CA rotation triggers — allows an observer to detect infrastructure events.
- Cross-controller routing metadata — exposes controller identifiers and tenant structures.

`ServiceCredentials` and MQTT tenant credentials are **never published to NATS** (filtered by
`NotificationService`), so credential exposure over plaintext NATS is not a risk for those message
types.

### Configuration guidance

For the NATS server, add to `nats-server.conf`:

```text
tls {
  cert_file: "/etc/nats/server-cert.pem"
  key_file:  "/etc/nats/server-key.pem"
  ca_file:   "/etc/nats/ca.pem"
  verify:    false   # set to true to require client certs
}
```

For the controller, set `UPTRAKIT_NATS_URL=nats-tls://nats-host:4222` (or the corresponding CLI
flag). See [NATS Deployment](../end-user/deployment/nats.md) and
[NATS Integration](../development/nats-integration.md#connection-url-and-tls) for details.

### Plugin config encryption in NATS messages

Plugin config fields in `CheckVersions`, `ExecuteUpdate`,
`ExecuteBatchUpdate`, and `DiscoverSoftware` messages are encrypted
with AES-256-GCM before NATS publication. This is separate from TLS transport
encryption — even with `nats-tls://`, configs are encrypted at the application
layer so they are unreadable at rest in JetStream storage and to any NATS
subscriber without the master key.

See [NATS Integration — Plugin Config Protection](../development/nats-integration.md#plugin-config-protection).

## Key Files

| File | Purpose |
| --- | --- |
| `crates/shared/crypto/src/lib.rs` | `EncryptedString` type, `init_master_key()`, `DataKeyRing`, AES-256-GCM encrypt/decrypt, key verification, `ENC:v1:`/`ENC:v2:`/`ENC:v3:` formats, DEK wrap/unwrap |
| `crates/shared/types/src/secret_string.rs` | `SecretString` newtype with redacted Debug/Display |
| `crates/ui/web-api-auth/src/settings_store.rs` | JWT signing key storage with AAD `"uptrakit:settings:jwt_signing_key"` |
| `crates/ui/web-api-auth/src/setting_key.rs` | `SettingKey::MasterKeyVerification` — stores the key verification token |
| `crates/core/controller/src/startup.rs` | `verify_master_key()`, `init_data_key_ring()`, `rotate_master_key()` |
| `crates/core/controller/src/reencrypt.rs` | Automatic v3 re-encryption of all encrypted columns and settings |
| `crates/core/agent-ssh/src/main.rs` | SSH agent DEK ring init, v3 re-encryption, master key rotation |
