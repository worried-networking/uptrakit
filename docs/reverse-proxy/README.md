# Reverse Proxy Deployment Guide

This guide covers deploying Uptrakit behind a reverse proxy. Two deployment modes are supported:

## Deployment Modes

### L4 TLS Passthrough

The proxy forwards raw TCP traffic to the controller without terminating TLS. The controller handles mTLS directly with agents. No `--trusted-proxy` or cert-forwarding flags are needed.

**Pros:** Simplest setup, full mTLS preserved, no certificate configuration on the proxy.
**Cons:** Proxy cannot inspect HTTP traffic, no path-based routing, no HTTP-level load balancing.

**Use when:** You only need basic TCP forwarding and want to preserve end-to-end mTLS.

### L7 TLS Termination

The proxy terminates TLS, optionally verifies client certificates, and forwards certificate information to the controller via HTTP headers. The proxy connects to the controller backend over HTTPS, trusting the controller's internal CA.

**Pros:** Full HTTP-level features (path routing, load balancing, caching, compression).
**Cons:** More complex setup, requires configuring client cert forwarding and CA trust.

**Use when:** You need HTTP-level features, multiple backends, or want the proxy to handle public TLS certificates.

## Controller Configuration

All settings below can be configured in three ways:

- **CLI flags** — set at startup (e.g., `--trusted-proxy=10.0.0.1`)
- **Web UI** — Settings > Network in the controller dashboard (changes apply immediately, no restart required)
- **REST API** — `GET /api/v1/settings/network` and `PUT /api/v1/settings/network` (see [API reference](#api-reference) below)

CLI flags seed the database on first run. After that, the database value takes precedence unless `--force-settings-override` is used. Runtime changes via the Web UI or API are applied immediately without restarting the controller.

| Flag | DB Key | Description |
| --- | --- | --- |
| `--trusted-proxy` | `network.trusted_proxies` | Proxy IP/CIDR (repeatable). Required for L7 mode; not needed for L4 passthrough. |
| `--real-ip-header` | `network.real_ip_header` | Header for real client IP (default: `X-Forwarded-For`) |
| `--forwarded-client-cert-info-header` | `network.forwarded_client_cert_info_header` | Header for structured cert info (L7 only) |
| `--forwarded-client-cert-pem-header` | `network.forwarded_client_cert_pem_header` | Header for PEM-encoded cert (L7 fallback) |
| `--pki-addr` | `network.pki_addr` | URL for PKI endpoints (e.g. `https://controller.internal:8443` or `http://controller:8080`). Embeds OCSP, CRL, and CA Issuer URLs in certificates via AIA/CDP extensions. Supports both `http://` and `https://` schemes. |

### Info Header Format

The structured info header uses semicolon-separated `key="value"` pairs:

```
Subject="CN=<agent-uuid>,O=Uptrakit";Issuer="CN=Uptrakit Internal CA";SerialNumber="01:ab:cd:ef"
```

Envoy's XFCC header uses comma-separated pairs (also supported):

```
Subject="CN=<agent-uuid>",Cert="<url-encoded-PEM>"
```

Fields:
- **Cert** (preferred when present): URL-encoded PEM certificate. Provides full identity extraction including serial number and issuer verification. Used by Envoy XFCC.
- **Subject** (required if no Cert): Distinguished name containing the agent UUID as CN
- **Issuer** (required for CA verification when using Subject): Distinguished name of the issuing CA
- **SerialNumber** / **Serial** (optional): Certificate serial number in hex

When a `Cert` field is present, the controller parses the full certificate from it. Otherwise it falls back to the Subject/SerialNumber/Issuer fields. When the serial number is absent in Subject-only mode, the controller uses agent-id-only lookup (finds any non-revoked cert for the agent UUID).

### PEM Header Format

The PEM header contains the client certificate in one of two formats:

- **Base64-DER** (recommended): The raw DER certificate encoded as standard base64. HTTP-header safe. Used by Caddy (`certificate_der_base64`).
- **URL-encoded PEM**: The full PEM certificate with URL-encoding applied to handle newlines and special characters.

The controller auto-detects the format and parses it to extract the agent identity and verify the issuer.

### API Reference

**Read current settings:**

```bash
curl -s -H "Authorization: Bearer <token>" \
  https://controller:8443/api/v1/settings/network
```

**Update settings** (all fields are optional — only included fields are changed):

```bash
curl -s -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  https://controller:8443/api/v1/settings/network \
  -d '{
    "trusted_proxies": ["10.0.0.1/32", "172.16.0.0/12"],
    "forwarded_client_cert_info_header": "X-Forwarded-Client-Cert-Info",
    "pki_addr": "http://controller:8080"
  }'
```

Requires `ManageSettings` permission. See the proxy-specific guides for which fields to set for each proxy.

## Security Model

1. **Trusted proxies required:** Certificate headers are only processed from requests originating from configured trusted proxy addresses (set via `--trusted-proxy`, the Web UI, or the API). All cert-related and proxy headers are stripped from non-proxy requests.

2. **CA CN verification:** The forwarded certificate's issuer CN must match either:
   - The active CA certificate's CN
   - The previous CA certificate's CN (only if the previous CA cert is not expired)

3. **Header stripping:** From direct (non-proxy) clients, the following headers are automatically removed:
   - The configured info header
   - The configured PEM header
   - `X-Forwarded-Proto`
   - `X-Forwarded-Host`

4. **mTLS takes precedence:** If the mTLS acceptor already extracted an `AgentIdentity` (direct TLS connection), all proxy cert headers are ignored.

## Exporting the CA Certificate

Proxies in L7 mode need the controller's CA certificate to trust the backend connection:

```bash
curl -k https://controller:8443/api/v1/pki/ca.crt -o ca.crt
```

Re-export after CA rotation (the controller broadcasts `CaBundleUpdated` to connected agents, but proxies must be updated manually or via automation).

## OCSP and CRL Revocation Checking

When a PKI address is configured (via `--pki-addr`, the Web UI, or the API), the controller embeds [AIA (Authority Information Access)](https://datatracker.ietf.org/doc/html/rfc5280#section-4.2.2.1) and [CDP (CRL Distribution Points)](https://datatracker.ietf.org/doc/html/rfc5280#section-4.2.1.13) extensions in both CA and agent certificates. These extensions point to the following controller endpoints:

| Extension | Endpoint | Description |
| --- | --- | --- |
| AIA OCSP | `POST /api/v1/pki/ocsp` | OCSP responder (RFC 6960). Also supports `GET /api/v1/pki/ocsp/{base64}`. |
| AIA CA Issuers | `GET /api/v1/pki/ca.crt` | CA certificate download |
| CDP CRL | `GET /api/v1/pki/ca.crl` | Certificate Revocation List |

All PKI endpoints are unauthenticated.

### Recommended approach

**OCSP is the preferred revocation checking method** when the proxy supports it. OCSP checks revocation in real-time on a per-certificate basis with no periodic refresh needed.

**CRL-based checking** requires periodic download of the CRL file. The controller rebuilds CRLs every hour and immediately on revocation events. Proxies using CRL-based checking should configure periodic CRL refresh (recommended interval: 30-60 minutes) to stay in sync. OCSP avoids this staleness window entirely.

### Proxy support matrix

| Proxy | OCSP support (client certs) | CRL support | Notes |
| --- | --- | --- | --- |
| **Nginx** | `ssl_ocsp leaf` (1.19.0+) | `ssl_crl` directive | OCSP recommended |
| **HAProxy** | No | `crl-file` on `bind` | CRL only; requires periodic refresh |
| **Envoy** | No | `crl` in `validation_context` | CRL only; requires sidecar refresh |
| **Traefik** | No | No | Revocation handled at the application layer |
| **Caddy** | No | No | Revocation handled at the application layer |

For proxies without OCSP or CRL support (Traefik, Caddy), the controller's mTLS verifier already checks CRLs for direct connections. In L7 mode, agents connect through the proxy without mTLS verification at the proxy level — revocation is enforced by the controller itself.

### Changing the PKI address

Changing `--pki-addr` requires CA rotation because the URLs are embedded in the CA certificate. The process:

1. Update the PKI address via the **Web UI** (Settings > Network > PKI Address) or the **API** (`PUT /api/v1/settings/network` with `"pki_addr": "..."`). The response includes a warning about required CA rotation.
2. Trigger CA rotation via the **Web UI** (Settings > PKI > Rotate CA) or the **API** (`POST /api/v1/settings/rotate-ca`).
3. The controller generates a new CA with updated AIA/CDP URLs and broadcasts `CaBundleUpdated` + `RequestCertRenewal` to all connected agents.
4. Connected agents refresh their CA bundle and renew certificates immediately.
5. Offline agents detect CA staleness via `ca_bundle_hash` on reconnect.

## Proxy-Specific Guides

| Proxy | Guide |
| --- | --- |
| Traefik | [traefik.md](traefik.md) |
| Caddy | [caddy.md](caddy.md) |
| Nginx | [nginx.md](nginx.md) |
| Nginx Proxy Manager | [nginx-proxy-manager.md](nginx-proxy-manager.md) |
| Envoy | [envoy.md](envoy.md) |
| HAProxy | [haproxy.md](haproxy.md) |
