# Reverse Proxy Security Model

This page focuses on the security considerations when using a reverse proxy in front of Uptrakit. For the general deployment guide, see [docs/end-user/deployment/reverse-proxy.md](../../end-user/deployment/reverse-proxy.md).

## Trusted Proxies and Header Trust

1. **Trusted proxies required.** The controller only processes certificate-related headers (`X-Forwarded-Client-Cert-Info`, PEM headers, etc.) when the request originates from an IP listed in `--trusted-proxy` (or `network.trusted_proxies`). Headers are stripped from untrusted sources.
2. **CA verification.** The issuer CN from forwarded cert headers must match either the active CA or the most recent non-expired CA maintained in the controller.
3. **Header stripping.** Headers such as the configured cert header, `X-Forwarded-Proto`, `X-Forwarded-Host`, and `Origin` are removed from non-proxy traffic to avoid spoofing.
4. **mTLS precedence.** Direct mTLS connections take priority; if the mTLS verifier already extracted `ServiceIdentity`, all proxy headers are ignored.

## Revocation Checking

### OCSP (recommended)

OCSP provides real-time revocation checks without periodic downloads. Configure the controller with `--pki-addr` (use `http://`), and optionally `--pki-http=listener` to expose `/api/v1/pki/ocsp` over plain HTTP. Nginx requires `http://` responder URLs — HTTPS OCSP URLs are rejected.

Proxies that support OCSP include:

- **Nginx:** `ssl_ocsp leaf` plus `ssl_ocsp_responder http://controller:8080/api/v1/pki/ocsp`. Use `resolver` for hostname lookup. (`--pki-http=listener` ensures the endpoint runs over HTTP.)

### CRL (alternative)

CrLs are static files rebuilt hourly and immediately on revocation. Download and reload them every 30–60 minutes when using proxies that lack OCSP support (HAProxy, Envoy, Traefik, Caddy).

```bash
curl -sk https://controller:8443/api/v1/pki/ca.crl -o /etc/proxy/ssl/ca.crl && nginx -s reload
```

### Proxy matrix

| Proxy | OCSP | CRL | Notes |
| --- | --- | --- | --- |
| Nginx | yes | yes | OCSP requires `http://` responder. |
| HAProxy | no | yes | Refresh CRL manually. |
| Envoy | no | yes | Refresh CRL manually. |
| Traefik | no | no | Rely on controller-side checks. |
| Caddy | no | no | Rely on controller-side checks. |

See each per-proxy guide for detailed configuration examples: [Traefik](traefik.md), [Caddy](caddy.md), [Nginx](nginx.md), [Nginx Proxy Manager](nginx-proxy-manager.md), [Envoy](envoy.md), [HAProxy](haproxy.md).

## CA Rotation and PKI Address Changes

- Changing `--pki-addr` requires CA rotation because the URLs are embedded in the CA certificate.
- Rotate the CA via `POST /api/v1/settings/rotate-ca` or Web UI to generate new AIA/CDP URLs.
- The controller broadcasts `CaBundleUpdated` and `RequestCertRenewal` messages to connected agents.
- Offline agents refresh the bundle when they reconnect, using the `ca_bundle_hash` field.
- Proxies should re-download the CA certificate after rotation (`GET /api/v1/pki/ca.crt`).

## Per-Proxy Security Guides

- [Traefik](traefik.md) — passthrough vs L7, header forwarding, OCSP/CRL notes.
- [Caddy](caddy.md) — PEM headers and TLS backend trust.
- [Nginx](nginx.md) — header definitions, OCSP responder wiring, CRL refresh cron sample.
- [Nginx Proxy Manager](nginx-proxy-manager.md) — GUI configuration + cert trust.
- [Envoy](envoy.md) and [HAProxy](haproxy.md) — raw header examples, CRL refresh tips.
