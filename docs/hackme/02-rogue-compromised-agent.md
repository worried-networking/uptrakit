# ATK-02: Rogue or Compromised Agent

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | Agent / wire protocol |
| Prerequisites | Compromised agent host or stolen agent mTLS certificate |
| STRIDE | Tampering, Repudiation |

## Attack description

1. The attacker compromises a host running the Uptrakit agent, or obtains the agent's
   private key and certificate from `state_dir/service.key` and `state_dir/service.crt`.
2. The attacker connects to the controller using the valid mTLS certificate and
   establishes an authenticated WebSocket session.
3. The attacker sends crafted `ServiceMessage` variants to the controller:
   - **Falsified version check results** (`version_check_results`): report incorrect
     installed versions to suppress update alerts or trigger unnecessary updates.
   - **Falsified update results** (`update_result`): claim an update succeeded when it
     was never applied, or report failure to block further attempts.
   - **Falsified discovery results** (`discovery_results`): inject fabricated software
     items into the controller's inventory.
   - **Fabricated host reports** (`report_hosts`): register fake hosts or modify the
     agent's reported hostname, OS, and architecture.
4. The controller trusts all authenticated messages from a valid agent certificate
   and processes them without content verification.

## Worst-case impact

- **Stale or incorrect software inventory.** The controller's view of installed
  versions diverges from reality, causing missed security updates or false positives.
- **Update pipeline disruption.** A rogue agent can block updates for its managed
  hosts by always reporting failure or success without applying changes.
- **Phantom hosts.** Fake `report_hosts` entries pollute the host inventory, wasting
  operator attention and potentially masking real infrastructure.
- **Discovery poisoning.** Injected discovery results can create software items linked
  to attacker-controlled plugin configs (see
  [ATK-09](09-discovery-result-poisoning.md)).
- **Lateral movement preparation.** The rogue agent receives `check_versions` and
  `execute_update` messages containing decrypted plugin configs (API tokens, registry
  credentials) for all software items assigned to its hosts.

## Current mitigations

- **mTLS authentication.** Agents authenticate with ECDSA P-256 client certificates
  issued by the controller's managed CA. Private keys are generated locally and never
  leave the agent host.
- **Certificate lifecycle controls.** Short-lived certificates (7-day default, max 730
  days) with automatic renewal limit the window of a stolen certificate.
- **Sequence-number replay protection.** Every WebSocket message carries a
  monotonically increasing `seq` field. Replayed or out-of-order messages cause
  immediate connection termination.
- **`host_machine_id` routing guard.** The regular agent validates that incoming
  `check_versions` and `execute_update` messages match its local machine ID. A
  mismatch is logged and silently dropped.
- **Certificate revocation.** Administrators can revoke an agent's certificate via the
  API. The controller performs a software-level revocation check against the
  `service_certificates` table on every WebSocket connect, and CRL-based revocation
  is enforced at the TLS layer for reverse-proxy deployments.
- **Agent runs as unprivileged user.** The agent process runs as a non-root user
  (e.g., `uptrakit`) with specific sudo allowlists, limiting what a compromised agent
  can do on the host.
- **Enrollment secret cleared after certificate issuance.** The enrollment secret is
  overwritten with an empty string in `service.json` once the mTLS certificate is
  obtained, preventing its reuse.

## Residual risk

- **No content integrity verification on agent messages.** The controller has no
  mechanism to independently verify that version check results, update outcomes, or
  discovery payloads are truthful. It trusts the authenticated agent implicitly.
- **Credential exposure window.** A rogue agent with valid mTLS continues to receive
  plugin configs containing API tokens and registry credentials for its assigned
  software items until the certificate is revoked.
- **Cross-instance revocation delay.** The JWT token denylist propagates via NATS
  (optional). Without NATS, certificate revocation takes effect only on the controller
  instance that performed the revocation, until other instances rebuild their CRL.
- **File-based key storage.** Agent private keys are stored on disk with `0o600`
  permissions. An attacker with read access to the agent's state directory can extract
  the key and impersonate the agent from any network location.

## Recommended improvements

- Implement anomaly detection for agent-reported data (e.g., sudden version changes,
  versions that do not match known release catalogs, or discovery results with
  suspicious plugin configs).
- Add a "quarantine" mode that flags agents reporting implausible data for manual
  review before accepting their payloads.
- Consider binding agent certificates to source IP ranges or network segments to limit
  the geographic scope of a stolen certificate.
- Provide an audit log of all agent-reported data changes (version transitions, new
  discoveries) with timestamps and source identity for forensic review.
- Document the credential exposure window in operator security guidance and recommend
  immediate certificate revocation as the first response to suspected agent
  compromise.

## References

- [Wire Protocol](../api/wire-protocol.md)
- [PKI and Certificates](../security/pki-certificates.md)
- [Security Architecture](../security/security-architecture.md)
- [Secrets and Encryption](../security/secrets-and-encryption.md)
- `crates/shared/wire/src/lib.rs` — `ServiceMessage`, `ControllerMessage` enums
- `crates/shared/service-sdk/src/identity.rs` — key storage and enrollment secret
  lifecycle
- `crates/ui/web-api/src/routes/service_ws/` — WebSocket handler and authentication
