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

| Flag | DB Key | Description |
| --- | --- | --- |
| `--trusted-proxy` | `network.trusted_proxies` | Proxy IP/CIDR (repeatable). Required for L7 mode; not needed for L4 passthrough. |
| `--real-ip-header` | `network.real_ip_header` | Header for real client IP (default: `X-Forwarded-For`) |
| `--forwarded-client-cert-info-header` | `network.forwarded_client_cert_info_header` | Header for structured cert info (L7 only) |
| `--forwarded-client-cert-pem-header` | `network.forwarded_client_cert_pem_header` | Header for PEM-encoded cert (L7 fallback) |

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

## Security Model

1. **Trusted proxies required:** Certificate headers are only processed from requests originating from configured `--trusted-proxy` addresses. All cert-related and proxy headers are stripped from non-proxy requests.

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
curl -k https://controller:8443/api/v1/ca.crt -o ca.crt
```

Re-export after CA rotation (the controller broadcasts `CaBundleUpdated` to connected agents, but proxies must be updated manually or via automation).

## Proxy-Specific Guides

| Proxy | Guide |
| --- | --- |
| Traefik | [traefik.md](traefik.md) |
| Caddy | [caddy.md](caddy.md) |
| Nginx | [nginx.md](nginx.md) |
| Nginx Proxy Manager | [nginx-proxy-manager.md](nginx-proxy-manager.md) |
| Envoy | [envoy.md](envoy.md) |
| HAProxy | [haproxy.md](haproxy.md) |
