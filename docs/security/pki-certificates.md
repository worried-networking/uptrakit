# PKI and Certificate Lifecycle

Uptrakit operates an internal PKI for agents and MQTT services.

## Asset Lifetimes

| Asset | Lifetime | Renewal Window |
| --- | --- | --- |
| CA certificate | 5 years | Rotate 6 months before expiry |
| Server HTTPS cert | 90 days | Renew 30 days before expiry |
| Agent/MQTT client cert | 365 days (configurable) | Configurable via `renewal_window_hours` |

## Certificate Issuance

- Agents and MQTT services enroll using a UUIDv7 `service_id` and CSR.
- Each CSR contains CN=`service_id`. The controller validates the CSR signature and signs it with the managed CA.
- Private keys never leave the agent/service.
- Renewals reuse the CSR flow with a fresh keypair.
- The controller stores CA history in the database and includes all non-expired certificates in the trust bundle.

## CA Rotation Flow

1. Background task checks every 24 hours for CAs entering the 6-month rotation window. Admins can also trigger rotation via `POST /api/v1/settings/rotate-ca`.
2. On rotation, the current CA row is marked inactive, a new CA row is inserted, and `pki.active_ca_fingerprint` is updated.
3. All non-expired historical CAs remain trusted via the bundle (`bundle_pem`).
4. CRLs are partitioned per CA (`ca_fingerprint`).
5. Connected agents receive `CaBundleUpdated` + `RequestCertRenewal` messages.
6. Offline agents detect staleness via `ca_bundle_hash` and fetch the bundle over HTTPS.
7. New agent certs are signed by the active CA.

## OCSP and CRLs

- The controller exposes `/api/v1/pki/ocsp` for OCSP requests (POST and GET) and `/api/v1/pki/ca.crl` for CRLs.
- OCSP supports SHA-1 and SHA-256 (Nginx uses SHA-1 for requests). Responses are signed with ECDSA P-256 SHA-256.
- CRLs are rebuilt hourly and on revocation. Proxies should refresh the file every 30–60 minutes when relying on CRLs.

## PKI Address and Extensions

- `--pki-addr` embeds Authority Information Access (OCSP, CA Issuers) and CRL Distribution Points in CA and agent certificates.
- `http://` scheme is recommended so proxies like Nginx can use OCSP (`ssl_ocsp_responder` only supports HTTP).
- `--pki-http=listener` starts a plain HTTP service for PKI routes, required for proxies such as Nginx’s OCSP responder.
- `--pki-http=external` suppresses warnings when an external proxy handles PKI HTTP.
- Changing `--pki-addr` requires CA rotation because the URLs are baked into the certificate extensions.

## State Management

- CA metadata flows through `CaPublicSnapshot` (watch channel) for API handlers.
- Private keys live in `CaKeyStore` (`zeroize::Zeroizing<String>` behind `Arc<RwLock>`). Only signing code accesses it.
- When adding code that uses CA material, use `AppState.ca_snapshot` and request a `CaKeyStoreRef` if signing is required.
