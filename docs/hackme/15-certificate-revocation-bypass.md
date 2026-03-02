# ATK-15: Certificate Revocation Bypass

| Field | Value |
| --- | --- |
| Severity | Medium |
| Attack surface | PKI / TLS (CRL and OCSP) |
| Prerequisites | Stolen agent certificate and network access to the controller |
| STRIDE | Denial of Service |

## Attack description

1. An attacker obtains a valid agent certificate (from a compromised host or
   extracted from `state_dir/service.crt` and `state_dir/service.key`).
2. An administrator revokes the certificate via the API.
3. The attacker races to use the certificate before the revocation propagates:
   - **CRL gap.** Reverse proxies that rely on CRL-based revocation check must
     download the updated CRL. CRLs are rebuilt on revocation and periodically (every
     4 hours by default), but proxy-side CRL refresh is operator-managed (recommended
     every 30-60 minutes). During the refresh gap, the proxy continues to accept the
     revoked certificate.
   - **OCSP propagation.** The OCSP responder reads `service_certificates.revoked_at`
     directly from the database, so OCSP is real-time. However, only Nginx natively
     supports OCSP verification for client certificates. HAProxy, Envoy, Traefik, and
     Caddy do not.
   - **Cross-instance delay.** In multi-controller HA deployments, the CRL rebuild is
     triggered via NATS `RequestCrlRenewal`. Without NATS, other controller instances
     do not learn about the revocation until their next periodic CRL rebuild.

4. The attacker connects to a controller instance or reverse proxy that has not yet
   received the updated CRL, using the revoked certificate for mTLS authentication.

## Worst-case impact

- **Continued access after revocation.** The attacker maintains authenticated
  WebSocket access using the revoked certificate for the duration of the CRL refresh
  gap (potentially 30-60 minutes for proxy-side, up to 4 hours for cross-instance
  without NATS).
- **Data exfiltration.** During the access window, the attacker receives plugin
  configs, version check assignments, and other operational data intended for the
  compromised service.
- **Operational disruption.** The attacker can send falsified messages (version check
  results, update status) during the access window, corrupting the controller's
  inventory state.

## Current mitigations

- **Software-level revocation check.** The controller performs a database lookup by
  `(serial_number, service_id)` against the `service_certificates` table on every
  WebSocket connect. If `revoked_at IS NOT NULL`, the connection is immediately closed
  with `CloseReason::CertificateRevoked`. This check is independent of CRL/OCSP and
  takes effect immediately on the revoking controller instance.
- **Immediate CRL rebuild on revocation.** When a certificate is revoked,
  `revocation_notify.notify_one()` fires immediately in `CrlManager::run()`, and a
  NATS `RequestCrlRenewal` message is published to notify remote instances.
- **OCSP responder.** The controller provides a real-time OCSP responder at
  `/api/v1/pki/ocsp` that reads revocation status directly from the database. OCSP
  responses include the revocation time and reason.
- **CRL persistence.** Signed CRLs are persisted in the `crl_cache` table. On
  startup, stale or missing CRLs are immediately regenerated, eliminating the startup
  window where `GET /api/v1/pki/ca.crl` would return a 404.
- **CRL numbering.** CRL numbers are monotonically increasing across controller
  restarts (initialized from `crl_cache.crl_number + 1`), ensuring proxies always see
  a newer CRL.
- **24-hour CRL validity.** Each CRL is valid for 24 hours (`this_update` to
  `next_update`), with scheduled rebuilds every 4 hours providing overlap.
- **CA path length constraint.** `BasicConstraints: pathLenConstraint=0` prevents a
  compromised agent certificate from being used to issue sub-certificates.

## Residual risk

- **CRL refresh is operator-managed.** The controller publishes CRLs, but proxy-side
  refresh frequency depends on the operator's cron job or script. A misconfigured
  refresh interval creates a longer window.
- **Limited proxy OCSP support.** Only Nginx supports OCSP for client certificate
  verification. The majority of supported reverse proxies (HAProxy, Envoy, Traefik,
  Caddy) rely on CRL-only revocation, which has inherent staleness.
- **Cross-instance delay without NATS.** Without NATS, CRL rebuilds on remote
  instances happen only on the periodic schedule (every 4 hours by default). The
  revoking instance updates immediately, but other instances lag.
- **No connection-level revocation for existing sessions.** The software-level
  revocation check runs on WebSocket connect, not on every message. An already-
  established session is not forcibly disconnected when a certificate is revoked;
  the agent must reconnect for the check to fire.
- **CRL size growth.** Over time, the CRL grows as more certificates are revoked.
  Large CRLs increase download time and may cause proxy-side parsing delays, though
  in practice the 7-day default certificate lifetime limits CRL size.

## Recommended improvements

- Add an active session termination mechanism that forcibly closes existing WebSocket
  connections when a certificate is revoked, rather than waiting for reconnection.
- Reduce the default CRL rebuild interval from 4 hours to 1 hour, or make it
  configurable with a recommended production value of 15-30 minutes.
- Publish a recommended CRL refresh script/cron configuration in the deployment
  documentation for each supported reverse proxy.
- For HA deployments without NATS, implement database-polling-based CRL rebuild
  triggers (e.g., monitor the `service_certificates.revoked_at` column for changes).
- Consider implementing OCSP stapling on the controller's TLS listener, allowing
  clients to verify revocation status without a separate OCSP request.
- Add monitoring and alerting for CRL age (time since last rebuild) to detect stale
  CRL conditions before they become a security risk.

## References

- [PKI and Certificates — CRLs](../security/pki-certificates.md#crls)
- [PKI and Certificates — OCSP Responder](../security/pki-certificates.md#ocsp-responder)
- [Reverse Proxy Security — Revocation Checking](../security/reverse-proxy-security.md#revocation-checking)
- `crates/core/controller/src/crl_manager.rs` — `CrlManager`
- `crates/ui/web-api/src/routes/service_ws/connection.rs` — software-level revocation
  check
