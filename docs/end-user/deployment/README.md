# End-user Deployment Guides

This folder contains end-user deployment documentation for running Uptrakit behind a reverse proxy.

## Contents

| Document | Description |
| --- | --- |
| [Reverse Proxy Deployment](reverse-proxy.md) | Reverse proxy deployment patterns, trusted proxy settings, and forwarding requirements. |
| [Nginx](nginx.md) | Nginx reverse proxy configuration (L4 passthrough and L7 termination). |
| [Nginx Proxy Manager](nginx-proxy-manager.md) | Nginx Proxy Manager GUI configuration and certificate trust. |
| [Traefik](traefik.md) | Traefik reverse proxy configuration and header forwarding. |
| [Caddy](caddy.md) | Caddy reverse proxy configuration and PEM header handling. |
| [Envoy](envoy.md) | Envoy reverse proxy configuration and XFCC header handling. |
| [HAProxy](haproxy.md) | HAProxy reverse proxy configuration and CRL refresh strategy. |

## Related Documentation

- End-user docs index: [`docs/end-user/README.md`](../README.md)
- Reverse proxy security model: [`docs/security/reverse-proxy-security.md`](../../security/reverse-proxy-security.md)
- Security docs index: [`docs/security/README.md`](../../security/README.md)
- Top-level docs catalogue: [`docs/README.md`](../../README.md)
