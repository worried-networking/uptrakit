# PKI and Certificate Lifecycle

Uptrakit operates an internal PKI for agents and MQTT services.

## Asset Lifetimes

| Asset | Lifetime | Renewal Window |
| --- | --- | --- |
| CA certificate | 5 years | Rotate 6 months before expiry |
| Server HTTPS cert | 90 days | Renew 30 days before expiry |
| Agent/MQTT client cert | 365 days (configurable) | Configurable via `renewal_window_hours` |

## CA Basic Constraints and Path Length

The controller CA is issued with `BasicConstraints: CA=true, pathLenConstraint=0`
(`IsCa::Ca(BasicConstraints::Constrained(0))` in rcgen). This means:

- The CA flag is set, so the certificate can sign leaf (end-entity) certificates.
- `pathLenConstraint=0` **prevents any certificate signed by this CA from being used as an
  intermediate CA** to issue further certificates. Even if an agent certificate were compromised,
  it cannot be used to mint additional certificates.

This is a defence-in-depth measure. The constraint is enforced by RFC 5280 §4.2.1.9 and is
verified at TLS validation time by conforming clients. A unit test (`ca_basic_constraints_path_len_is_zero`)
asserts this on every build.

See also: [Secure Development](secure-development.md) for the general PKI security requirements.

## Certificate Issuance

- Agents and MQTT services enroll using a UUIDv7 `service_id` and CSR.
- Each CSR contains CN=`service_id`. The controller validates the CSR signature and signs it with the managed CA.
- Private keys never leave the agent/service.
- Renewals reuse the CSR flow with a fresh keypair.
- The controller stores CA history in the database and includes all non-expired certificates in the trust bundle.

## CA Rotation Flow

1. Background task checks every 24 hours for CAs entering the 6-month rotation window. Admins can also trigger rotation
   via `POST /api/v1/settings/rotate-ca`.
1. On rotation, the current CA row is marked inactive, a new CA row is inserted, and `pki.active_ca_fingerprint` is
   updated.
1. All non-expired historical CAs remain trusted via the bundle (`bundle_pem`).
1. CRLs are partitioned per CA (`ca_fingerprint`).
1. Connected agents receive `CaBundleUpdated` + `RequestCertRenewal` messages.
1. Offline agents detect staleness via `ca_bundle_hash` and fetch the bundle over HTTPS.
1. New agent certs are signed by the active CA.

## PKI Address and AIA/CDP Extensions

When `--pki-addr` is configured, the controller embeds AIA (Authority Information Access) and CDP (CRL Distribution
Points) extensions in both CA and agent certificates:

| Extension | URL |
| --- | --- |
| AIA OCSP | `{pki_addr}/api/v1/pki/ocsp` |
| AIA CA Issuers | `{pki_addr}/api/v1/pki/ca.crt` |
| CDP CRL | `{pki_addr}/api/v1/pki/ca.crl` |

`--pki-addr` accepts both `http://` and `https://` URLs. **`http://` is recommended** because Nginx only supports
`http://` OCSP responder URLs -- `https://` AIA URLs are silently ignored by Nginx's `ssl_ocsp` directive. When the PKI
address uses `http://`, the `--pki-http` flag controls how plain HTTP serving is handled:

| `--pki-http` value | Behaviour |
| --- | --- |
| `listener` | The controller starts a plain HTTP listener on the port from `--pki-addr`, serving only PKI routes (`/healthz`, `/api/v1/pki/ca.crt`, `/api/v1/pki/ca.crl`, `/api/v1/pki/ocsp`). Required for Nginx `ssl_ocsp_responder` which only supports `http://` OCSP responder URLs. |
| `external` | PKI HTTP is handled by an external component (e.g. reverse proxy). Suppresses the warning about `http://` scheme without `--pki-http`. |
| (not set) | If `--pki-addr` uses `http://`, the controller logs a warning. |

At startup, the controller validates the existing CA certificate's embedded URLs against the reconciled `pki_addr`:

- PKI address set and matching CA extensions: OK
- PKI address set but different from CA extensions: **startup failure** (suggests updating the setting or rotating the
  CA)
- PKI address set but CA has no extensions: **startup failure** (suggests rotating the CA to regenerate with extensions)
- PKI address not set but CA has extensions: **startup failure** (suggests providing `--pki-addr` or rotating the CA to
  regenerate without extensions)
- Neither set: OK

Changing the PKI address requires CA rotation (the URLs are embedded in the CA certificate). See the
[reverse proxy security guide](reverse-proxy-security.md) for the full flow.

## OCSP Responder

The controller provides an OCSP responder at `/api/v1/pki/ocsp` (both POST and GET). It accepts standard RFC 6960 OCSP
requests and returns signed OCSP responses:

- **good**: certificate is valid and not revoked
- **revoked**: certificate has been revoked (includes revocation time and reason)
- **unknown**: certificate serial not found

The responder supports both SHA-1 and SHA-256 hash algorithms in requests per RFC 6960. Nginx/OpenSSL always uses SHA-1
(`1.3.14.3.2.26`) for OCSP requests. `ResponderID::ByKey` uses SHA-1 as required by RFC 6960 Section 2.3. Responses are
signed with the active CA's private key using ECDSA P-256 SHA-256.

Only Nginx natively supports OCSP verification of client certificates (via `ssl_ocsp` directive, since v1.19.0).
HAProxy, Envoy, Traefik, and Caddy do not.

## CRLs

CRLs are rebuilt hourly and immediately on revocation. Proxies should refresh the file every 30-60 minutes when relying
on CRLs. The CRL endpoint is `/api/v1/pki/ca.crl`.

## External CA

Pass `--ca-cert` and `--ca-key` to disable managed CA and rotation. The controller uses the provided CA as-is.

## Server Certificate Auto-Renewal

When the server HTTPS certificate (also CA-signed) approaches expiry, a background task generates a new one and
hot-reloads the TLS listener. Admins can also trigger renewal manually via
`POST /api/v1/settings/renew-server-certificate`.

## Server Certificate SAN Sanity Checks

At startup, the controller validates that `--san` values match the existing managed server certificate's SANs:

1. **`--san` is incompatible with `--tls-cert`/`--tls-key`**: the controller rejects this combination because SANs are
   only configurable for controller-managed certificates.
1. **SAN mismatch + same CA**: if `--san` values are not present in the existing cert's SANs and the cert was signed by
   the currently active CA, the cert is silently regenerated.
1. **SAN mismatch + different CA**: if the cert needing SAN regeneration was signed by a different CA (e.g. after CA
   rotation), the controller fails with a multi-step fix message guiding the admin through manual certificate renewal.

Shared PKI utility functions (`SanCollection`, `collect_sans`, `cert_signed_by_ca`) live in
`crates/ui/web-api/src/pki_utils.rs` and are used by both the web API handlers and the controller startup logic.

## State Management

### CaSnapshot Sharing

Runtime CA state is split into **public** and **private** components:

- **`CaPublicSnapshot`** (public certificates, fingerprints, CRL data) is shared via a `tokio::sync::watch` channel. API
  handlers and route middleware read from this channel. It contains no private key material.
- **`CaKeyStore`** (private keys wrapped in `zeroize::Zeroizing<String>`) is shared via
  `Arc<tokio::sync::RwLock<CaKeyStore>>`. Only the OCSP responder, CRL manager, cert signer, and server cert renewal
  code access the key store. The `Debug` impl redacts all key material.

When adding new code that needs CA certificates or fingerprints, read from `AppState.ca_snapshot`. When adding code that
needs to **sign** (OCSP responses, CRLs, certificates), also accept a `CaKeyStoreRef` and look up keys by fingerprint.

Controllers poll the `pki.ca_version` settings key to detect CA changes made by other instances and reload both the
public snapshot and key store.

### Settings Snapshot Sharing

Runtime settings are shared via a `tokio::sync::watch` channel holding an atomic `SettingsSnapshot` struct. This
replaces the previous 6-`RwLock` pattern that was susceptible to torn reads.

- **Readers** call synchronous methods (e.g. `settings.registration()`, `settings.authentication()`) that borrow the
  watch channel -- no `.await` needed.
- **Writers** acquire a `tokio::sync::Mutex` and publish via `send_modify()` for atomic updates.
- **`reload_from_db()`** builds a complete `SettingsSnapshot` from the database and publishes it atomically.
- **Version counters** (`version`, `global_version`) use `Ordering::Acquire`/`Release` for cross-instance cache
  invalidation.

When adding code that reads settings, use the synchronous reader methods. When adding code that modifies settings, use
the `set_*` methods (e.g. `settings.set_registration(...)`) which acquire the write mutex.

## JWT Signing Key

The JWT signing key is stored in the database settings table (key: `auth.jwt_signing_key`, base64-encoded, marked as
global). All HA instances share the same key. On first startup, the controller generates a 64-byte random key and stores
it. Existing file-based keys (`jwt_signing.key`) are automatically migrated to the database on startup.

## JWT Token Denylist

An in-memory `TokenDenylist` (`src/auth/token_denylist.rs`) provides immediate JWT revocation within each controller
instance. It supports:

- **Per-JTI denial**: individual tokens denied by their `jti` claim.
- **Per-user denial**: all tokens for a user issued before a given timestamp.

The denylist is checked on every JWT-authenticated request in the `authenticate_jwt` middleware. On logout, all tokens
for the user are denied for the remaining access token lifetime (15 min). A periodic purge task cleans expired entries.

**Known limitation**: the denylist is per-instance (in-memory). Cross-instance revocation relies on natural token
expiry. DB-backed HA sync is deferred.
