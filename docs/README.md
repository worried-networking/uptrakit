# Uptrakit Documentation

This directory holds audience-specific documentation split into five sections, plus deployment and reference links.

## Quick Navigation

- [End-user Guides](#end-user-guides)
- [API and Protocol](#api-and-protocol)
- [Security](#security)
- [Architecture](#architecture)
- [Development](#development)
- [Deployment Guides](#deployment-guides)
- [Additional Resources](#additional-resources)

## End-user Guides

| Guide | Description |
| --- | --- |
| [System Overview](end-user/system-overview.md) | High-level architecture and agent/controller roles. See also: [Wire Protocol](api/wire-protocol.md), [Security Architecture](security/security-architecture.md). |
| [Update Workflow](end-user/update-workflow.md) | Manual update lifecycle, hooks, and history links. See also: [Services and Operations](api/services-operations.md). |
| [Home Assistant and MQTT](end-user/home-assistant-mqtt.md) | MQTT/Home Assistant integration notes. See also: [Auth Flows](api/auth-flows.md), [Services and Operations](api/services-operations.md). |
| [Deployment Map](end-user/deployment-map.md) | Deployment navigation map and pointers. See also: [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md), [Reverse Proxy Security](security/reverse-proxy-security.md). |
| [SSH Agent Host Management](end-user/ssh-agent-host-management.md) | Managing SSH host entries via the `uptrakit-agent-ssh` CLI. See also: [SSH Agent Architecture](architecture/ssh-agent.md), [SSH Agent Secrets](security/ssh-agent-secrets.md). |
| [SSH Agent Bootstrap](end-user/ssh-agent-bootstrap.md) | Automated remote host setup (user creation, key deployment, sudoers). See also: [SSH Agent Secrets](security/ssh-agent-secrets.md), [SSH Agent Architecture](architecture/ssh-agent.md). |
| [CLI Usage Guide](end-user/cli-usage.md) | Command reference for the `uptrakit` binary (all command groups: hosts, software-items, provider-configs, autodiscovery, check, update, history, scheduler, settings). See also: [CLI Output](development/cli-output.md), [Auth Flows](api/auth-flows.md). |
| [Provider Configurations](end-user/provider-configs.md) | Provider config management, supported provider types, autodiscovery, and ignore rules. See also: [Autodiscovery](end-user/autodiscovery.md), [Software Item Entity](architecture/software-item-entity.md). |
| [Update History](end-user/update-history.md) | Viewing update history, status reference, filters, and triggering updates. See also: [Update Workflow](end-user/update-workflow.md), [Update History Entity](architecture/update-history-entity.md). |
| [Profile and API Tokens](end-user/profile-tokens.md) | Account info, API token lifecycle, security best practices. See also: [Auth Flows](api/auth-flows.md), [Auth and Authorization](security/auth-and-authorization.md). |
| [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md) | Reverse proxy deployment choices, headers, and API snippets. See also: [PKI and Certificates](security/pki-certificates.md), [Reverse Proxy Security](security/reverse-proxy-security.md). |

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
| [PKI and Certificates](security/pki-certificates.md) | CA rotation, OCSP, CRL, CaSnapshot, and PKI address behavior. See also: [Reverse Proxy Security](security/reverse-proxy-security.md), [TOFU and TLS](security/tofu-tls.md). |
| [Auth and Authorization](security/auth-and-authorization.md) | Authentication methods, permissions model, JWT, and roles. See also: [Auth Flows](api/auth-flows.md). |
| [Secrets and Encryption](security/secrets-and-encryption.md) | Master key, encrypted fields, and `SecretString`. See also: [Setup](development/setup.md). |
| [Reverse Proxy Security](security/reverse-proxy-security.md) | Proxy trust model and revocation guidance. See also: [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md). |
| [TOFU and TLS](security/tofu-tls.md) | TOFU verifier and CLI hardening. See also: [Auth Flows](api/auth-flows.md). |
| [Filesystem and Dependency Security](security/filesystem-dependency-security.md) | Secure permissions and dependency controls. See also: [Dependency Policy](development/dependency-policy.md). |
| [Secure Development](security/secure-development.md) | Security requirements for contributors. See also: [Coding Standards](development/coding-standards.md), [Testing](development/testing.md). |
| [SSH Agent Secrets](security/ssh-agent-secrets.md) | SSH agent secret storage, master key management, and threat model. See also: [SSH Agent Architecture](architecture/ssh-agent.md), [Secrets and Encryption](security/secrets-and-encryption.md). |

## Architecture

| Guide | Description |
| --- | --- |
| [Multi-tenancy](architecture/multi-tenancy.md) | Database and API multi-tenancy model. See also: [Services and Operations](api/services-operations.md). |
| [Host Entity](architecture/host-entity.md) | Host representation and `machine_id` tracking. See also: [Wire Protocol](api/wire-protocol.md). |
| [Software Item Entity](architecture/software-item-entity.md) | Software item definition and provider configuration. See also: [Provider Guidelines](development/provider-guidelines.md). |
| [Update History Entity](architecture/update-history-entity.md) | Immutable update history records. See also: [Update Workflow](end-user/update-workflow.md). |
| [Scheduler](architecture/scheduler.md) | Centralised DB-backed task scheduler with HA-safe optimistic locking. See also: [HTTP Web API](api/http-web-api.md), [Cross-Controller Communication](development/cross-controller-comm.md). |
| [SSH Agent](architecture/ssh-agent.md) | SSH-backed agent architecture, local DB schema, and self-managed encryption. See also: [SSH Agent Secrets](security/ssh-agent-secrets.md), [Service Lifecycle](development/service-lifecycle.md). |

## Development

| Guide | Description |
| --- | --- |
| [Setup](development/setup.md) | Prerequisites, master key handling, and build/lint commands. See also: [Testing](development/testing.md), [Secrets and Encryption](security/secrets-and-encryption.md). |
| [Testing](development/testing.md) | Required test suites and Docker reverse-proxy coverage. See also: [PR Process](development/pr-process.md). |
| [Coding Standards](development/coding-standards.md) | Core design rules, logging, and quality constraints. See also: [Provider Guidelines](development/provider-guidelines.md). |
| [Error Handling](development/error-handling.md) | rootcause/thiserror patterns, decision guide, anti-patterns, and approved exceptions. See also: [Coding Standards](development/coding-standards.md), [Secure Development](security/secure-development.md). |
| [PR Process](development/pr-process.md) | PR checklist and Conventional Commits expectations. |
| [Dependency Policy](development/dependency-policy.md) | Workspace dependency rules and `cargo deny` guidance. |
| [Provider Guidelines](development/provider-guidelines.md) | Provider lifecycle rules and documentation expectations. See also: [Wire Protocol](api/wire-protocol.md). |
| [AI Guidelines](development/ai-guidelines.md) | Responsible AI usage policy for contributors and assistants. |
| [CLI Output](development/cli-output.md) | CLI output formatting conventions and standards. |
| [Commit Messages](development/commit-messages.md) | Conventional Commits format and examples. |
| [Cross-Controller Communication](development/cross-controller-comm.md) | HA controller-to-controller event propagation. |
| [Graceful Restart](development/graceful-restart.md) | Zero-downtime restart and shutdown behavior. |
| [Quality Gates](development/quality-gates.md) | CI quality gate requirements for all changes. |
| [Update Hooks](development/update-hooks.md) | Pre/post-update hook execution and configuration. |
| [Command Executor](development/command-executor.md) | `CommandExecutor` trait, `CommandSpec`, and `LocalCommandExecutor` for transport-agnostic command dispatch. See also: [Provider Guidelines](development/provider-guidelines.md), [SSH Agent](architecture/ssh-agent.md). |
| [Service Lifecycle](development/service-lifecycle.md) | `ServiceHandler` trait and `run_service_lifecycle()` for building new services. See also: [Services and Operations](api/services-operations.md). |
| [OpenAPI Client](development/openapi-client.md) | Typed HTTP client for the web API (`uptrakit-openapi-client`). See also: [HTTP Web API](api/http-web-api.md), [CLI Usage](end-user/cli-usage.md). |
| [Embedded Frontend](development/embedded-frontend.md) | Building the controller with the frontend embedded in the binary (`embed-frontend` feature). See also: [Setup](development/setup.md), [Deployment Guides](end-user/deployment/README.md). |

## Deployment Guides

| Guide | Description |
| --- | --- |
| [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md) | Reverse proxy deployment options and header formats. |
| [Reverse Proxy Security Overview](security/reverse-proxy-security.md) | Reverse proxy security considerations, revocation, and CA rotation. |
| [Nginx](end-user/deployment/nginx.md) | Nginx reverse proxy configuration guidance. |
| [Nginx Proxy Manager](end-user/deployment/nginx-proxy-manager.md) | Nginx Proxy Manager configuration guidance. |
| [Traefik](end-user/deployment/traefik.md) | Traefik reverse proxy configuration guidance. |
| [Envoy](end-user/deployment/envoy.md) | Envoy reverse proxy configuration guidance. |
| [HAProxy](end-user/deployment/haproxy.md) | HAProxy reverse proxy configuration guidance. |
| [Caddy](end-user/deployment/caddy.md) | Caddy reverse proxy configuration guidance. |

## Additional Resources

- API reference: `/swagger-ui` when built with the `swagger-ui` feature.
- AsyncAPI spec: [`crates/shared/wire/asyncapi.yaml`](../crates/shared/wire/asyncapi.yaml).
- Core project docs: [`README.md`](../README.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), [`ARCHITECTURE.md`](../ARCHITECTURE.md), [`SECURITY.md`](../SECURITY.md).
- TODO tracker: [`TODO.md`](../TODO.md).
