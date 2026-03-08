# ATK-14: Scheduler Credential Exposure

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | External scheduler / credential delivery |
| Prerequisites | Compromise of the external scheduler service or its network path |
| STRIDE | Information Disclosure |

## Attack description

1. The external scheduler (`uptrakit-scheduler`) enrolls as a service with credential
   capabilities: `DatabaseAccess`, `NatsAccess`, `MasterKeyAccess`, `CaManagement`,
   and `Scheduler`.
2. After mTLS authentication, the controller sends a `ServiceCredentials` message
   over the WebSocket containing:
   - `db_url`: full database connection string including credentials.
   - `nats_url`: NATS server URL.
   - `master_key_hex`: the 256-bit master encryption key as a 64-character hex string.
3. An attacker who compromises the external scheduler (or intercepts the WebSocket
   connection) obtains all three credentials.
4. With the master key, the attacker can decrypt all encrypted database values
   (see [ATK-03](03-master-key-compromise.md)).
5. With the database URL, the attacker has direct SQL access to all tables.
6. With the NATS URL, the attacker can publish and subscribe to cross-controller
   messages.

Alternatively, the attacker targets the credential delivery path:

- **CheckVersions over NATS.** `ControllerMessage::CheckVersions` payloads contain
  decrypted plugin configs (GitHub API tokens, Docker registry passwords). This
  message type is not in the `is_nats_publishable()` blocklist, so when NATS is
  configured, plugin credentials are published to JetStream and available to any
  NATS subscriber.
- **Service approval social engineering.** The attacker registers a rogue service
  advertising credential capabilities. The controller displays security warnings in
  the UI when approving services with credential capabilities, but an inattentive
  admin might approve the rogue service.

## Worst-case impact

- **Complete system compromise.** The master key, database credentials, and NATS
  access together provide full control over the Uptrakit deployment: decrypt all
  secrets, forge certificates, modify any data, and inject messages to all
  controller instances.
- **Plugin credential theft via NATS.** An attacker with NATS access can subscribe to
  `uptrakit.events.controller` and capture `CheckVersions` messages containing
  decrypted plugin API tokens and registry credentials.
- **Persistent undetectable access.** Direct database access allows the attacker to
  create backdoor accounts, modify audit logs, and inject data without going through
  the API authentication layer.

## Current mitigations

- **WebSocket-only delivery.** `ServiceCredentials` messages are never published to
  NATS. The `is_nats_publishable()` function explicitly blocks `ServiceCredentials`,
  `TenantAssignments`, `TenantConfigUpdated`, and `TenantRevoked` variants. Credential
  delivery occurs exclusively over the authenticated mTLS WebSocket.
- **Capability-gated credentials.** The controller only populates credential fields
  matching the service's declared capabilities. A service without `MasterKeyAccess`
  does not receive the master key.
- **Admin approval with warnings.** The UI displays per-capability security warnings
  when approving services with credential capabilities (e.g., "This service will
  receive direct database access credentials"). Only admin-approved services receive
  credentials.
- **mTLS authentication.** The scheduler authenticates with an ECDSA P-256 client
  certificate. A compromised network position alone cannot intercept the WebSocket
  without the client certificate or a forged CA.
- **SecretString for wire fields.** Credential fields in `ServiceCredentialsPayload`
  use `SecretString`, which provides `ZeroizeOnDrop` and redacted `Debug` output.
- **NATS transport security.** The controller warns when NATS is configured with
  plaintext transport and recommends `nats-tls://` for production.
- **NATS config encryption.** *(Implemented)* Plugin config fields in
  `CheckVersions`, `ExecuteUpdate`, `ExecuteBatchHostPackageUpdate`, and
  `DiscoverSoftware` messages are encrypted with AES-256-GCM (via the shared
  master key) before NATS publication. Receiving controllers decrypt the configs
  before delivering to agents. Configs are unreadable to NATS subscribers that
  do not possess the master key.

## Residual risk

- ~~`CheckVersions` not blocked from NATS.~~ **Mitigated.** Plugin config fields
  are now AES-256-GCM encrypted before NATS publication. NATS subscribers without
  the master key cannot read plugin credentials.
- **Single-service credential concentration.** The external scheduler receives the
  master key, database URL, and NATS URL — the three most sensitive credentials in
  the system — in a single message. Compromise of the scheduler is equivalent to
  full system compromise.
- **Plaintext NATS is accepted.** `nats://` (plaintext) is a valid configuration.
  NATS messages, including `CheckVersions` with plugin credentials, transit in the
  clear.
- **No credential rotation.** Delivered credentials (database URL, NATS URL) are
  static. If the scheduler is compromised and later remediated, the exposed
  credentials remain valid until manually rotated.
- **Rogue service approval risk.** A rogue service advertising credential capabilities
  could be approved by mistake. The security warnings in the UI are the only
  protection; there is no secondary confirmation step.

## Recommended improvements

- ~~Add `CheckVersions` to the `is_nats_publishable()` blocklist~~ —
  **Replaced.** Plugin configs in all credential-bearing message types are now
  encrypted before NATS publication rather than blocked, preserving external
  scheduler functionality.
- Implement credential rotation for the external scheduler: periodically issue new
  database credentials and master key tokens, invalidating previous ones.
- Add a secondary confirmation step (e.g., email or TOTP verification) for approving
  services with credential capabilities, beyond the UI security warning.
- Consider splitting credential capabilities into separate enrollment tokens, so the
  scheduler enrollment requires a specific high-privilege token rather than general
  admin approval.
- Require `nats-tls://` in production and make plaintext NATS a startup error (not
  just a warning) when credential-bearing messages could be published.
- Add audit logging for all `ServiceCredentials` deliveries, including the receiving
  service identity and the credentials delivered (masked).

## References

- [Secrets and Encryption — Credential Capabilities](../security/secrets-and-encryption.md#credential-capabilities-and-servicecredentials)
- [External Scheduler Deployment](../end-user/deployment/external-scheduler.md)
- [NATS Integration](../development/nats-integration.md)
- [ATK-03: Master Key Compromise](03-master-key-compromise.md)
- `crates/shared/wire/src/payloads.rs` — `ServiceCredentialsPayload`
- `crates/shared/wire/src/messages.rs` — `is_nats_publishable()`
- `crates/ui/web-api/src/routes/service_ws/` — credential delivery logic
