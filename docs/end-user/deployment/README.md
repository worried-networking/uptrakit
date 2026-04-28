---
title: Overview
weight: 1
description: Deployment guides for running Uptrakit behind a reverse proxy, covering proxy modes, Docker Compose, external scheduler, and NATS HA setups.
---

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
| [Docker](docker.md) | Docker Compose deployment with auto-enrollment and profiles. |

## Single-binary Deployment

The controller can be built with the `embed-frontend` Cargo feature to embed
the SvelteKit frontend directly into the binary. This eliminates the need to
deploy the `frontend/build/` directory alongside the binary.

See [Embedded Frontend](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) for build
instructions.

## Related Documentation

- End-user docs index: [`docs/end-user/README.md`](../README.md)
- Reverse proxy security model: [`docs/security/reverse-proxy-security.md`](../../security/reverse-proxy-security.md)
- Security docs index: [`docs/security/README.md`](../../security/README.md)
- Top-level docs catalogue: [`docs/README.md`](../../README.md)
