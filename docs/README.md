# Uptrakit Documentation

This directory holds audience-specific documentation split into four sections, plus deployment and reference links.

## Quick Navigation

- [End-user Guides](#end-user-guides)
- [API and Protocol](#api-and-protocol)
- [Security](#security)
- [Development](#development)
- [Deployment Guides](#deployment-guides)
- [Additional Resources](#additional-resources)

## End-user Guides

| Guide | Description |
| --- | --- |
| [System Overview](end-user/system-overview.md) | High-level architecture and agent/controller roles. See also: [Wire Protocol](api/wire-protocol.md), [Security Architecture](security/security-architecture.md). |
| [Update Workflow](end-user/update-workflow.md) | Manual update lifecycle, hooks, and history links. See also: [Services and Operations](api/services-operations.md). |
| [Home Assistant and MQTT](end-user/home-assistant-mqtt.md) | MQTT/Home Assistant integration notes. See also: [Auth Flows](api/auth-flows.md), [Services and Operations](api/services-operations.md). |
| [Deployment Map](end-user/deployment-map.md) | Deployment navigation map and pointers. See also: [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md), [Reverse Proxy Security](security/reverse-proxy/index.md). |
| [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md) | Reverse proxy deployment choices, headers, and API snippets. See also: [PKI and Certificates](security/pki-certificates.md), [Reverse Proxy Security](security/reverse-proxy/index.md). |

## API and Protocol

| Guide | Description |
| --- | --- |
| [Wire Protocol](api/wire-protocol.md) | WebSocket lifecycle, message taxonomy, and AsyncAPI references. See also: [AsyncAPI Spec](../crates/shared/wire/asyncapi.yaml). |
| [HTTP Web API](api/http-web-api.md) | Public REST endpoints, rate limits, multi-tenancy, and update operations. See also: [Auth Flows](api/auth-flows.md), [Settings Runtime](api/settings-runtime.md). |
| [Settings Runtime](api/settings-runtime.md) | DB-managed settings, reconciliation, and watch channels. See also: [Services and Operations](api/services-operations.md). |
| [Auth Flows](api/auth-flows.md) | Device auth, tokens, MQTT enrollment, and denylist behavior. See also: [Auth and Authorization](security/auth-and-authorization.md), [Secrets and Encryption](security/secrets-and-encryption.md). |
| [Services and Operations](api/services-operations.md) | Agent/MQTT flows, update history, and tenant-scoped tables. See also: [Update Workflow](end-user/update-workflow.md). |

## Security

| Guide | Description |
| --- | --- |
| [Security Architecture](security/security-architecture.md) | Defense-in-depth principles. See also: [Cryptography](security/cryptography.md), [Secure Development](security/secure-development.md). |
| [Cryptography](security/cryptography.md) | Crypto primitives and libraries. See also: [PKI and Certificates](security/pki-certificates.md), [Secrets and Encryption](security/secrets-and-encryption.md). |
| [PKI and Certificates](security/pki-certificates.md) | CA rotation, OCSP, CRL, CaSnapshot, and PKI address behavior. See also: [Reverse Proxy Security](security/reverse-proxy/index.md), [TOFU and TLS](security/tofu-tls.md). |
| [Auth and Authorization](security/auth-and-authorization.md) | Authentication methods, permissions model, JWT, and roles. See also: [Auth Flows](api/auth-flows.md). |
| [Secrets and Encryption](security/secrets-and-encryption.md) | Master key, encrypted fields, and `SecretString`. See also: [Setup](development/setup.md). |
| [Reverse Proxy Security](security/reverse-proxy/index.md) | Proxy trust model and revocation guidance. See also: [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md). |
| [TOFU and TLS](security/tofu-tls.md) | TOFU verifier and CLI hardening. See also: [Auth Flows](api/auth-flows.md). |
| [Filesystem and Dependency Security](security/filesystem-dependency-security.md) | Secure permissions and dependency controls. See also: [Dependency Policy](development/dependency-policy.md). |
| [Secure Development](security/secure-development.md) | Security requirements for contributors. See also: [Coding Standards](development/coding-standards.md), [Testing](development/testing.md). |
| [Per-Proxy Security Guides](security/reverse-proxy/index.md#per-proxy-security-guides) | Security and configuration notes for Nginx, Nginx Proxy Manager, Traefik, Envoy, HAProxy, and Caddy. |

## Development

| Guide | Description |
| --- | --- |
| [Setup](development/setup.md) | Prerequisites, master key handling, and build/lint commands. See also: [Testing](development/testing.md), [Secrets and Encryption](security/secrets-and-encryption.md). |
| [Testing](development/testing.md) | Required test suites and Docker reverse-proxy coverage. See also: [PR Process](development/pr-process.md). |
| [Coding Standards](development/coding-standards.md) | Error handling, logging, and core design rules. See also: [Provider Guidelines](development/provider-guidelines.md). |
| [PR Process](development/pr-process.md) | PR checklist and Conventional Commits expectations. |
| [Dependency Policy](development/dependency-policy.md) | Workspace dependency rules and `cargo deny` guidance. |
| [Provider Guidelines](development/provider-guidelines.md) | Provider lifecycle rules and documentation expectations. See also: [Wire Protocol](api/wire-protocol.md). |
| [AI Guidelines](development/ai-guidelines.md) | Responsible AI usage policy for contributors and assistants. |

## Deployment Guides

| Guide | Description |
| --- | --- |
| [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md) | Reverse proxy deployment options and header formats. |
| [Reverse Proxy Security Overview](security/reverse-proxy/index.md) | Reverse proxy security considerations, revocation, and CA rotation. |
| [Nginx](security/reverse-proxy/nginx.md) | Nginx-specific hardening and configuration guidance. |
| [Nginx Proxy Manager](security/reverse-proxy/nginx-proxy-manager.md) | Nginx Proxy Manager-specific guidance. |
| [Traefik](security/reverse-proxy/traefik.md) | Traefik-specific hardening and configuration guidance. |
| [Envoy](security/reverse-proxy/envoy.md) | Envoy-specific hardening and configuration guidance. |
| [HAProxy](security/reverse-proxy/haproxy.md) | HAProxy-specific hardening and configuration guidance. |
| [Caddy](security/reverse-proxy/caddy.md) | Caddy-specific hardening and configuration guidance. |

## Additional Resources

- API reference: `/swagger-ui` when built with the `swagger-ui` feature.
- AsyncAPI spec: [`crates/shared/wire/asyncapi.yaml`](../crates/shared/wire/asyncapi.yaml).
- Core project docs: [`README.md`](../README.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), [`ARCHITECTURE.md`](../ARCHITECTURE.md), [`SECURITY.md`](../SECURITY.md).
- TODO tracker: [`TODO.md`](../TODO.md).
