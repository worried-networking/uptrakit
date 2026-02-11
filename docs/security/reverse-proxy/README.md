# Reverse Proxy Security Guides

This folder contains reverse proxy security guidance, including trusted proxy behavior, certificate forwarding, and revocation checking options.

## Contents

| Document | Description |
| --- | --- |
| [Reverse Proxy Security Model](index.md) | Shared trust model, header validation, OCSP/CRL strategy, and CA rotation implications. |
| [Nginx](nginx.md) | Nginx hardening and configuration details. |
| [Nginx Proxy Manager](nginx-proxy-manager.md) | Nginx Proxy Manager-specific security and configuration guidance. |
| [Traefik](traefik.md) | Traefik guidance for secure forwarding and certificate header handling. |
| [Envoy](envoy.md) | Envoy guidance for trusted forwarding and certificate metadata handling. |
| [HAProxy](haproxy.md) | HAProxy guidance for secure forwarding and revocation strategy. |
| [Caddy](caddy.md) | Caddy guidance for secure reverse-proxy deployment. |

## Related Documentation

- Security docs index: [`docs/security/README.md`](../README.md)
- End-user reverse proxy deployment guide: [`docs/end-user/deployment/reverse-proxy.md`](../../end-user/deployment/reverse-proxy.md)
- Top-level docs catalogue: [`docs/README.md`](../../README.md)
