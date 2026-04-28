---
title: PKI and Certificate Lifecycle
weight: 30
description: Uptrakit operates an internal PKI for agents and MQTT services, covering CA lifecycle, certificate issuance, and renewal windows.
---

# PKI and Certificate Lifecycle

Uptrakit operates an internal PKI for agents and MQTT services.

## Asset Lifetimes

| Asset | Lifetime | Renewal Window |
| --- | --- | --- |
| CA certificate | 5 years | Rotate 6 months before expiry |
| Server HTTPS cert | 90 days | Renew 30 days before expiry |
| Agent/MQTT client cert | 168 h (default, max 17 520 h) | `min(14 days, lifetime / 5)` — see below |

## Agent/MQTT Certificate Renewal Window

The renewal window determines how early a service initiates a certificate renewal before the cert expires.

### Automatic mode (default)

The window is computed per-certificate as:

```text
renewal_window = min(14 days, cert_lifetime / 5)
```

Examples:

| Cert lifetime | 1/5 | Ceiling | Effective window |
| --- | --- | --- | --- |
| 2 h | < 1 h | 336 h | **< 1 hour (rounds down to 0 — service renews immediately)** |
| 12 h | 2 h | 336 h | **2 hours** |
| 168 h (7 days) | 33 h | 336 h | **33 hours** |
| 720 h (30 days) | 144 h | 336 h | **144 hours** |
| 1 680 h (70 days) | 336 h | 336 h | **336 hours (14 days)** |
| 8 760 h (365 days) | 1 752 h | 336 h | **336 hours (14 days)** |

This formula applies at **two layers**:

1. **Server-side scheduler** (`ServiceCertCheckExecutor`): queries DB for certs approaching the
   per-cert window (`not_after ≤ now + min(14 days, (not_after − not_before) / 5)`) and sends
   `RequestCertRenewal` to the owning service.
2. **On-service timer** (`cert_handler.rs`): when the service connects, the controller sends
   `renewal_window_hours` in `ServiceSettingsPayload`; the service uses this to schedule its
   local proactive renewal timer.

### Custom override

An admin can pin the renewal window via `PUT /api/v1/settings/agent-certificates`:

```json
{ "renewal_window_hours": 48 }
```

Send `0` to reset to automatic mode:

```json
{ "renewal_window_hours": 0 }
```

The API response includes both the raw override and the effective value:

```json
{
  "lifetime_hours": 168,
  "renewal_window_hours_override": null,
  "effective_renewal_window_hours": 33
}
```

### Per-service override

Each individual service can have its own certificate lifetime, independent of the global default.
Set it via `PUT /api/v1/services/{id}`:

```json
{ "cert_lifetime_hours": 48 }
```

Send `0` to clear the per-service override and revert to the global default:

```json
{ "cert_lifetime_hours": 0 }
```

Valid values: `0` (clear) or `1`–`17520`. When set, this value takes precedence over the global
setting at certificate signing time. The per-service value is visible in `GET /api/v1/services/{id}`
as `cert_lifetime_hours` (omitted when the global default applies).

See also: [Services Operations](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) for the full `UpdateServiceRequest` reference.

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

### DER encoding implementation

AIA and CDP extension bodies are manually DER-encoded in `crates/core/controller/src/pki.rs`.
The encoder supports the 2-byte long-form length encoding (`0x82`), which covers extension bodies
up to 65 535 bytes. Extension bodies exceeding this limit are rejected with `PkiError::LengthOverflow`
rather than silently truncating the high byte and producing a structurally malformed extension.

In practice, AIA/CDP extension bodies are small (two or three short URLs; typically under 300 bytes)
so the overflow guard is a safety net, not an expected code path.

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

### Overview

CRLs (Certificate Revocation Lists) are signed by each active CA and served at `/api/v1/pki/ca.crl`. Reverse proxies
that rely on CRL validation should refresh the file every 30–60 minutes.

### Three-Path Rebuild Model

CRL rebuilds are triggered by three separate mechanisms:

| Trigger | Mechanism |
| --- | --- |
| Certificate revoked — same controller | `revocation_notify.notify_one()` fires immediately in `CrlManager::run()`; the CRL is rebuilt and the NATS `RequestCrlRenewal` message is published to notify remote instances |
| Certificate revoked — remote controllers | `ControllerMessage::RequestCrlRenewal` NATS event received; `event_delivery` calls `revocation_notify.notify_one()` on each receiving controller instance |
| Periodic refresh | `CrlRenewal` scheduled task (default interval: 14 400 seconds / 4 hours, jitter: 120 seconds) executes `CrlRenewalExecutor`, which calls `SchedulerNotifier::signal_crl_renewal()` — fires `revocation_notify` on the embedded scheduler and publishes NATS for all instances |

The default renewal interval (every 4 hours) is configurable via the existing scheduler task management API
(`PUT /api/v1/scheduler/tasks/{id}`). No separate global setting is needed.

### DB Persistence (`crl_cache` Table)

Signed CRLs are persisted in the `crl_cache` table after every rebuild:

| Column | Type | Description |
| --- | --- | --- |
| `ca_fingerprint` | `TEXT` (PK) | CA certificate fingerprint |
| `crl_pem` | `TEXT` | PEM-encoded CRL |
| `crl_number` | `BIGINT` | Monotonically increasing CRL number |
| `this_update` | `TIMESTAMPTZ` | CRL issuance time |
| `next_update` | `TIMESTAMPTZ` | CRL validity expiry (24 h after issuance) |
| `updated_at` | `TIMESTAMPTZ` | Last row update time |

No `tenant_id` column — CRLs are global PKI state (not per-tenant).

At startup, `CrlManager` tries to load each CA's CRL from `crl_cache`. A cached entry is used if:

- The row exists for the CA's fingerprint, **and**
- `next_update` is more than 1 hour in the future (fresh buffer).

If the cache is missing or stale, a fresh CRL is generated and persisted immediately. This eliminates the
startup window where `GET /api/v1/pki/ca.crl` would return a 404 before the first rebuild.

The CRL number is initialized from `crl_cache.crl_number + 1` on startup so the counter is monotonically
increasing across controller restarts.

### CRL Validity

Each CRL is valid for 24 hours (`this_update` to `next_update`). CRLs are signed by the corresponding CA's
private key using ECDSA P-256 SHA-256.

### Implementation Details

- `CrlManager::run()` listens exclusively on `revocation_notify` — no periodic polling timer.
- `revocation_notify` is fired by all three triggers above (local revocation, NATS event, scheduler).
- Periodic renewal is delegated entirely to the `CrlRenewal` scheduler task; removing the 60-second poll
  loop from the CRL manager makes the polling interval observable and configurable from the UI.
- OCSP is unaffected — the responder reads `service_certificates.revoked_at` directly from the database.

See also:

- [Scheduler Engine](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) — `CrlRenewalExecutor` details
- [Cross-Controller Communication](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) — `RequestCrlRenewal` NATS message
- [Wire Protocol](https://github.com/worried-networking/uptrakit/tree/main/docs/api/) — `request_crl_renewal` message definition

## External CA

Pass `--ca-cert` and `--ca-key` to disable managed CA and rotation. The controller uses the provided CA as-is.

## Server Certificate Auto-Renewal

When the server HTTPS certificate (also CA-signed) approaches expiry, a background task generates a new one and
hot-reloads the TLS listener. Admins can also trigger renewal manually via
`POST /api/v1/settings/renew-server-certificate`.

## Server Certificate SAN Sanity Checks

At startup, the controller validates that the canonical SAN list matches the existing managed server certificate's SANs:

1. **`--san` is incompatible with `--tls-cert`/`--tls-key`**: the controller rejects this combination because SANs are
   only configurable for controller-managed certificates.
1. **SAN mismatch + same CA**: if the canonical SANs do not match the existing cert's SANs and the cert was signed by
   the currently active CA, the cert is silently regenerated.
1. **SAN mismatch + different CA**: if the cert needing SAN regeneration was signed by a different CA (e.g. after CA
   rotation), the controller fails with a multi-step fix message guiding the admin through manual certificate renewal.

### SAN resolution

SANs follow a three-case resolution model:

1. **`--san` provided**: the CLI values become the canonical list (no auto-detection). Stored in DB as `network.sans`.
1. **No `--san`, DB has `network.sans`**: the stored value is used as-is.
1. **No `--san`, no DB value (first start)**: hostname, FQDN, and localhost are auto-detected, saved to DB, and used.

Shared PKI utility functions (`SanCollection`, `auto_detect_sans`, `parse_san_list`, `cert_signed_by_ca`) live in
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
