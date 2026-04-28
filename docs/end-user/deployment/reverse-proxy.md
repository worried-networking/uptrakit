---
title: Reverse Proxy Deployment Guide
weight: 10
description: Covers L4 TCP passthrough and L7 TLS termination modes, controller trusted-proxy settings, and client certificate forwarding requirements.
---

# Reverse Proxy Deployment Guide

Deploying Uptrakit behind a reverse proxy can use either TCP passthrough or HTTP termination. Choose the mode that fits your networking needs.

## Deployment Modes

### L4 TLS Passthrough

The proxy forwards raw TCP traffic to the controller. TLS, including mTLS with agents, is handled end-to-end by the controller.

**Pros:** Simple configuration, no certificate handling by the proxy, full agent mTLS preserved.
**Cons:** No HTTP routing or inspection (no path-based routing or virtual hosts).

### L7 TLS Termination

The proxy terminates TLS, optionally verifies client certificates, and forwards certificate metadata to the controller via headers. The proxy trusts
the controller's internal CA for the backend connection.

**Pros:** HTTP routing, host/path-based routing, and traffic management features provided by the proxy.
**Cons:** Additional header configuration and CA trust required, plus forwarding the client certificate to the controller.

## Controller Configuration

Configure the controller via CLI flags, the Web UI (Settings > Network), or the REST API (`/api/v1/settings/network`). CLI flags seed the database on
first run; thereafter the DB value takes precedence unless `--force-settings-override` is used.

| Flag | DB key | Purpose |
| --- | --- | --- |
| `--trusted-proxy` | `network.trusted_proxies` | IP or CIDR of the proxy. Required in L7 mode. |
| `--real-ip-header` | `network.real_ip_header` | Header carrying the client IP (defaults to `X-Forwarded-For`). |
| `--forwarded-client-cert-info-header` | `network.forwarded_client_cert_info_header` | Header carrying structured cert info (`X-Forwarded-Client-Cert-Info` recommended). |
| `--forwarded-client-cert-pem-header` | `network.forwarded_client_cert_pem_header` | Header carrying PEM-encoded cert (use when structured info is unavailable). |
| `--pki-addr` | `network.pki_addr` | URL for PKI endpoints (`http://` recommended for OCSP support). Use with `--pki-http=listener` or `external`. |

Run the controller with the corresponding flags or set the values via the UI/API. Runtime changes via the UI or API apply immediately.

## Header Formats

### Info Header

Structured headers are semicolon-separated `key="value"` pairs, for example:

```text
Subject="CN=<agent-uuid>,O=Uptrakit";Issuer="CN=Uptrakit Internal CA";SerialNumber="01:ab:cd:ef"
```

Envoy's XFCC header uses comma-separated pairs and is also supported:

```text
Subject="CN=<agent-uuid>",Cert="<url-encoded-PEM>"
```

Include the `Cert` field whenever possible; the controller parses the full certificate and uses it to verify the issuer and serial number. When `Cert`
is absent, the controller falls back to the `Subject`, `Issuer`, and `SerialNumber` fields.

### PEM Header

Raw PEM headers may contain either:

- Base64-DER (raw DER encoded as standard base64) — preferred and HTTP-safe.
- URL-encoded PEM (full PEM with newline escaping).

The controller auto-detects both formats and extracts the certificate for identity verification.

## API Reference

**Read current network settings:**

```bash
curl -s -H "Authorization: Bearer <token>" \
  https://controller:8443/api/v1/settings/network
```

**Update proxy configuration:**

```bash
curl -s -X PUT -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  https://controller:8443/api/v1/settings/network \
  -d '{
    "trusted_proxies": ["10.0.0.1/32"],
    "forwarded_client_cert_info_header": "X-Forwarded-Client-Cert-Info",
    "pki_addr": "http://controller:8080"
  }'
```

Requires the `ManageSettings` permission.

## Exporting the CA Certificate

Proxies that terminate TLS must trust the controller's internal CA. Fetch it with:

```bash
curl -k https://controller:8443/api/v1/pki/ca.crt -o ca.crt
```

After CA rotation, re-run this command before updating the proxy trust store.

## Changing the PKI Address

The PKI address is embedded into every managed CA certificate; changing it triggers a CA rotation:

1. Update the PKI address through the Web UI (Settings > Network > PKI Address) or the API (`PUT /api/v1/settings/network` with `pki_addr`).
1. Rotate the CA via the Web UI (Settings > PKI > Rotate CA) or `POST /api/v1/settings/rotate-ca`.
1. Connected agents receive a `CaBundleUpdated` + `RequestCertRenewal` message and refresh certificates automatically.
1. Offline agents detect the updated `ca_bundle_hash` and fetch the new bundle over HTTPS.

## Frontend Sub-Path Deployment

When the frontend SPA is served from a sub-path (e.g., `https://example.com/uptrakit/`) via a reverse proxy
and the API is available at a different prefix, set the `VITE_API_BASE` environment variable **at build time**:

```sh
VITE_API_BASE=/uptrakit/api/v1 npm run build
```

The default value is `/api/v1`, which is correct when both the SPA and the API are served from the same
origin root. Most reverse proxy deployments do not need this setting.

> [!NOTE]
> `VITE_API_BASE` is a Vite build-time variable. It is compiled into the static bundle and cannot
> be changed at runtime without rebuilding.

## Proxy-Specific Guides

- [Traefik](traefik.md)
- [Caddy](caddy.md)
- [Nginx](nginx.md)
- [Nginx Proxy Manager](nginx-proxy-manager.md)
- [Envoy](envoy.md)
- [HAProxy](haproxy.md)

Detailed header configuration, OCSP/CRL setup, and per-proxy examples live in the security-focused reverse proxy guides above.
