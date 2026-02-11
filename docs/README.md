# Uptrakit Documentation

This directory holds the detailed, audience-specific documentation split into four sections.

## End-user Guides
| Guide | Description |
| --- | --- |
| `docs/end-user/system-overview.md` | High-level architecture and agent/controller roles. |
| `docs/end-user/update-workflow.md` | Manual update lifecycle, hooks, and history links. |
| `docs/end-user/home-assistant-mqtt.md` | MQTT/Home Assistant integration notes. |
| `docs/end-user/deployment-map.md` | Deployment navigation map and pointers. |
| `docs/end-user/deployment/reverse-proxy.md` | Reverse proxy deployment choices, headers, and API snippets. |

## API & Protocol
| Guide | Description |
| --- | --- |
| `docs/api/wire-protocol.md` | WebSocket lifecycle, message taxonomy, AsyncAPI spec link. |
| `docs/api/http-web-api.md` | Public REST endpoints, rate limits, multi-tenancy, update operations. |
| `docs/api/settings-runtime.md` | DB-managed settings, reconciliation, watch channels. |
| `docs/api/auth-flows.md` | Device auth, tokens, MQTT enrollment, denylist behavior. |
| `docs/api/services-operations.md` | Agent/MQTT flows, update history, tenant-scoped tables. |

## Security
| Guide | Description |
| --- | --- |
| `docs/security/security-architecture.md` | Defense-in-depth principles. |
| `docs/security/cryptography.md` | Crypto primitives and libraries. |
| `docs/security/pki-certificates.md` | CA rotation, OCSP, CRL, CaSnapshot, PKI addresses. |
| `docs/security/auth-and-authorization.md` | Authentication methods, permissions model, JWT/roles. |
| `docs/security/secrets-and-encryption.md` | Master key, encrypted fields, SecretString. |
| `docs/security/reverse-proxy/index.md` | Proxy trust model and revocation guidance. |
| `docs/security/tofu-tls.md` | TOFU verifier and CLI hardening. |
| `docs/security/filesystem-dependency-security.md` | Secure permissions and dependency controls. |
| `docs/security/secure-development.md` | Security links for contributors. |
| `docs/security/reverse-proxy/*.md` | Proxy-specific security + configuration notes. |

## Development
| Guide | Description |
| --- | --- |
| `docs/development/setup.md` | Prerequisites, master key, build/lint commands. |
| `docs/development/testing.md` | Required tests and Docker reverse-proxy suites. |
| `docs/development/coding-standards.md` | Error handling, logging, and design rules. |
| `docs/development/pr-process.md` | PR checklist, Conventional Commits expectations. |
| `docs/development/dependency-policy.md` | Workspace dependency rules and cargo-deny guidance. |
| `docs/development/provider-guidelines.md` | Provider lifecycle documentation requirements. |
| `docs/development/ai-guidelines.md` | Responsible AI usage policy. |

## Deployment Guides
| Guide | Description |
| --- | --- |
| `docs/end-user/deployment/reverse-proxy.md` | Reverse proxy deployment options and header formats. |
| `docs/security/reverse-proxy/index.md` | Reverse proxy security considerations, revocation, CA rotation. |
| `docs/security/reverse-proxy/*.md` | Proxy-specific configuration notes (Nginx, Traefik, Envoy, etc.). |

## Additional Resources
- API reference: `/swagger-ui` when built with the `swagger-ui` feature.
- AsyncAPI spec: `crates/shared/wire/asyncapi.yaml`.
- TODO tracker: [`../TODO.md`](../TODO.md).
