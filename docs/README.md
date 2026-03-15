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
| [Notifications](end-user/notifications.md) | Setting up notification channels (Webhook, Telegram, Email), creating rules, scoping, actionable notifications. See also: [Notifications API](api/notifications.md), [Notifications Security](security/notifications-security.md). |
| [CLI Usage Guide](end-user/cli-usage.md) | Command reference for the `uptrakit` binary (all command groups: hosts, software-items, plugin-configs, autodiscovery, check, update, history, scheduler, settings, notifications). See also: [CLI Output](development/cli-output.md), [Auth Flows](api/auth-flows.md). |
| [Plugin Configurations](end-user/plugin-configs.md) | Plugin config management, supported plugin types, autodiscovery, and ignore rules. See also: [Autodiscovery](end-user/autodiscovery.md), [Software Item Entity](architecture/software-item-entity.md). |
| [Proxmox VE Integration](end-user/proxmox.md) | Proxmox VE VM/CT discovery, manual host matching, and plugin configuration. See also: [Proxmox Plugin Development](development/proxmox-plugin.md). |
| [Unified Software Tracking](architecture/unified-software-tracking.md) | Software tracking data model, featured vs non-featured items, batch updates, and host summaries. See also: [Autodiscovery](end-user/autodiscovery.md), [Software Item Entity](architecture/software-item-entity.md). |
| [Update History](end-user/update-history.md) | Viewing update history, status reference, filters, and triggering updates. See also: [Update Workflow](end-user/update-workflow.md), [Update History Entity](architecture/update-history-entity.md). |
| [Interactive Updates](end-user/interactive-updates.md) | Bidirectional terminal I/O for update sessions with PTY allocation and stdin forwarding. See also: [Interactive Updates API](api/interactive-updates.md), [Interactive Updates Security](security/interactive-updates.md). |
| [Audit Logs](end-user/audit-logs.md) | Viewing audit logs (tenant and system), filter reference, permissions, and CLI usage. See also: [Audit Logs API](api/audit-logs.md), [Audit Logs Security](security/audit-logs.md). |
| [User Management](end-user/user-management.md) | Managing users, roles, access presets, and lockout prevention. See also: [User Management API](api/user-management.md), [Auth and Authorization](security/auth-and-authorization.md). |
| [Profile and API Tokens](end-user/profile-tokens.md) | Account info, API token lifecycle, security best practices. See also: [Auth Flows](api/auth-flows.md), [Auth and Authorization](security/auth-and-authorization.md). |
| [Zero-Configuration Discovery](end-user/zeroconf-discovery.md) | Automatic controller discovery via mDNS/DNS-SD on the LAN. See also: [Zeroconf Development](development/zeroconf-discovery.md), [Zeroconf Security](security/zeroconf-discovery.md). |
| [Reverse Proxy Deployment](end-user/deployment/reverse-proxy.md) | Reverse proxy deployment choices, headers, and API snippets. See also: [PKI and Certificates](security/pki-certificates.md), [Reverse Proxy Security](security/reverse-proxy-security.md). |

## API and Protocol

| Guide | Description |
| --- | --- |
| [Wire Protocol](api/wire-protocol.md) | WebSocket lifecycle, message taxonomy, and AsyncAPI references. See also: [AsyncAPI Spec](../crates/shared/wire/asyncapi.yaml). |
| [HTTP Web API](api/http-web-api.md) | Public REST endpoints, rate limits, multi-tenancy, and update operations. See also: [Auth Flows](api/auth-flows.md), [Settings Runtime](api/settings-runtime.md). |
| [Settings Runtime](api/settings-runtime.md) | DB-managed settings, reconciliation, and watch channels. See also: [Services and Operations](api/services-operations.md). |
| [Auth Flows](api/auth-flows.md) | Device auth, tokens, MQTT enrollment, and denylist behavior. See also: [Auth and Authorization](security/auth-and-authorization.md), [Secrets and Encryption](security/secrets-and-encryption.md). |
| [Services and Operations](api/services-operations.md) | Agent/MQTT flows, update history, and tenant-scoped tables. See also: [Update Workflow](end-user/update-workflow.md). |
| [Enrollment Tokens](api/enrollment-tokens.md) | Enrollment token CRUD, capability scoping, usage limits, TTL, and enrollment flow. See also: [Auth and Authorization](security/auth-and-authorization.md). |
| [Autodiscovery API](api/autodiscovery.md) | Autodiscovery trigger endpoints and software ignore rule management. See also: [Autodiscovery Guide](end-user/autodiscovery.md), [Unified Software Tracking](architecture/unified-software-tracking.md). |
| [Host Tags API](api/host-tags.md) | Host tag CRUD, batch delete, and host tag assignment endpoints. See also: [Host Tags Architecture](architecture/host-tags.md), [CLI Usage Guide](end-user/cli-usage.md). |
| [Notifications](api/notifications.md) | Notification channel, rule, and delivery log REST API reference. See also: [Notifications Guide](end-user/notifications.md), [Notifications Security](security/notifications-security.md). |
| [Audit Logs API](api/audit-logs.md) | Audit log REST API reference: endpoints, filter parameters, response schema, permissions. See also: [Audit Logs Guide](end-user/audit-logs.md), [Audit Logs Security](security/audit-logs.md). |
| [User Management API](api/user-management.md) | User, role, permission, and access preset management endpoints. See also: [Auth and Authorization](security/auth-and-authorization.md), [User Management Guide](end-user/user-management.md). |
| [Batch Actions](api/batch-actions.md) | Batch/group action endpoints for bulk management operations (services, hosts, software items, etc.). See also: [CLI Usage Guide](end-user/cli-usage.md), [Extensions API](api/extensions.md). |
| [Interactive Updates API](api/interactive-updates.md) | Interactive WebSocket endpoint, message formats, and wire protocol for bidirectional update sessions. See also: [Interactive Updates Guide](end-user/interactive-updates.md), [Interactive Updates Security](security/interactive-updates.md). |

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
| [Notifications Security](security/notifications-security.md) | Notification secret storage, callback verification, action token lifecycle, and tenant isolation. See also: [Notifications API](api/notifications.md), [Notifications Development](development/notifications.md). |
| [Audit Logs Security](security/audit-logs.md) | Audit log data scope, tenant isolation (two-table design, no FK on tenant_id), access permissions, retention/GDPR, backend security properties. See also: [Audit Logs Development](development/audit-logs.md), [Audit Logs API](api/audit-logs.md). |
| [Interactive Updates Security](security/interactive-updates.md) | Threat model, permission model, audit logging, and wire protocol security for interactive update sessions. See also: [Interactive Updates API](api/interactive-updates.md), [Interactive Updates Development](development/interactive-updates.md). |
| [Zero-Configuration Discovery Security](security/zeroconf-discovery.md) | mDNS threat model, MITM mitigation, and recommendations for high-security environments. See also: [Zeroconf Development](development/zeroconf-discovery.md), [Zeroconf Guide](end-user/zeroconf-discovery.md). |

## Architecture

| Guide | Description |
| --- | --- |
| [Multi-tenancy](architecture/multi-tenancy.md) | Database and API multi-tenancy model. See also: [Services and Operations](api/services-operations.md). |
| [Host Entity](architecture/host-entity.md) | Host representation and `machine_id` tracking. See also: [Wire Protocol](api/wire-protocol.md). |
| [Software Item Entity](architecture/software-item-entity.md) | Software item definition and plugin configuration. See also: [Plugin Guidelines](development/plugin-guidelines.md), [Plugin System](development/plugin-system.md). |
| [Update History Entity](architecture/update-history-entity.md) | Immutable update history records. See also: [Update Workflow](end-user/update-workflow.md). |
| [Scheduler](architecture/scheduler.md) | Centralised DB-backed task scheduler with HA-safe optimistic locking (embedded or external). See also: [Scheduler Engine](development/scheduler-engine.md), [External Scheduler Deployment](end-user/deployment/external-scheduler.md), [HTTP Web API](api/http-web-api.md), [Cross-Controller Communication](development/cross-controller-comm.md). |
| [Unified Software Tracking](architecture/unified-software-tracking.md) | Unified software tracking model, `featured` flag routing, batch updates, and host update summaries. See also: [Autodiscovery](end-user/autodiscovery.md), [Software Item Entity](architecture/software-item-entity.md). |
| [Host Tags](architecture/host-tags.md) | User-defined host labels, color palette, tag assignments, and tenant isolation. See also: [Host Tags API](api/host-tags.md), [Host Entity](architecture/host-entity.md). |
| [SSH Agent](architecture/ssh-agent.md) | SSH-backed agent architecture, local DB schema, and self-managed encryption. See also: [SSH Agent Secrets](security/ssh-agent-secrets.md), [Service Lifecycle](development/service-lifecycle.md). |

## Development

| Guide | Description |
| --- | --- |
| [Setup](development/setup.md) | Prerequisites, master key handling, and build/lint commands. See also: [Testing](development/testing.md), [Secrets and Encryption](security/secrets-and-encryption.md). |
| [Testing](development/testing.md) | Required test suites and Docker reverse-proxy coverage. See also: [PR Process](development/pr-process.md). |
| [Coding Standards](development/coding-standards.md) | Core design rules, logging, and quality constraints. See also: [Plugin Guidelines](development/plugin-guidelines.md). |
| [Error Handling](development/error-handling.md) | rootcause/thiserror patterns, decision guide, anti-patterns, and approved exceptions. See also: [Coding Standards](development/coding-standards.md), [Secure Development](security/secure-development.md). |
| [PR Process](development/pr-process.md) | PR checklist and Conventional Commits expectations. |
| [Dependency Policy](development/dependency-policy.md) | Workspace dependency rules and `cargo deny` guidance. |
| [Plugin Guidelines](development/plugin-guidelines.md) | Plugin lifecycle rules, capabilities, host compatibility detection, and lifecycle hooks. See also: [Wire Protocol](api/wire-protocol.md), [Plugin System](development/plugin-system.md). |
| [Plugin System](development/plugin-system.md) | Plugin system architecture, discovery flow, and capability extension model. |
| [AI Guidelines](development/ai-guidelines.md) | Responsible AI usage policy for contributors and assistants. |
| [CLI Output](development/cli-output.md) | CLI output formatting conventions and standards. |
| [Commit Messages](development/commit-messages.md) | Conventional Commits format and examples. |
| [Cross-Controller Communication](development/cross-controller-comm.md) | HA controller-to-controller event propagation via NATS JetStream. |
| [NATS Integration](development/nats-integration.md) | NATS JetStream development guide: feature flags, architecture, testing. |
| [Graceful Restart](development/graceful-restart.md) | Zero-downtime restart and shutdown behavior. |
| [Quality Gates](development/quality-gates.md) | CI quality gate requirements for all changes. |
| [Update Lifecycle Plugins](development/update-hooks.md) | Systemd and shell hook plugins for pre/post-update lifecycle hooks. |
| [Command Executor](development/command-executor.md) | `CommandExecutor` trait, `CommandSpec`, and `LocalCommandExecutor` for transport-agnostic command dispatch. See also: [Plugin Guidelines](development/plugin-guidelines.md), [SSH Agent](architecture/ssh-agent.md). |
| [Service Lifecycle](development/service-lifecycle.md) | `ServiceHandler` trait and `run_service_lifecycle()` for building new services. See also: [Services and Operations](api/services-operations.md). |
| [OpenAPI Client](development/openapi-client.md) | Typed HTTP client for the web API (`uptrakit-openapi-client`). See also: [HTTP Web API](api/http-web-api.md), [CLI Usage](end-user/cli-usage.md). |
| [Scheduler Engine](development/scheduler-engine.md) | Scheduler engine crate internals: `TaskExecutor`, `SchedulerNotifier`, executor details. See also: [Scheduler](architecture/scheduler.md), [External Scheduler Deployment](end-user/deployment/external-scheduler.md). |
| [Notifications](development/notifications.md) | Notification subsystem architecture: dispatcher, channel trait, adding new channels, feature flags, event hooks. See also: [Notifications API](api/notifications.md), [Notifications Security](security/notifications-security.md). |
| [Audit Logs](development/audit-logs.md) | Audit log subsystem: crate structure, backend selection, filter config, separate DB, retention, testing, REST API (query module, route handlers, permissions). See also: [Audit Logs Security](security/audit-logs.md), [Audit Logs API](api/audit-logs.md). |
| [Proxmox VE Plugin](development/proxmox-plugin.md) | Proxmox VE infrastructure plugin internals: client, discovery, matching, extensions, DB schema. See also: [Proxmox End-User Guide](end-user/proxmox.md). |
| [Embedded Frontend](development/embedded-frontend.md) | Building the controller with the frontend embedded in the binary (`embed-frontend` feature). See also: [Setup](development/setup.md), [Deployment Guides](end-user/deployment/README.md). |
| [Docker](development/docker.md) | Docker image build, CI workflow, and local development with docker-compose. See also: [Docker Deployment](end-user/deployment/docker.md). |
| [Interactive Updates](development/interactive-updates.md) | Interactive updates feature: PTY allocation, feature gate strategy, architecture, testing. See also: [Interactive Updates API](api/interactive-updates.md), [Interactive Updates Security](security/interactive-updates.md). |
| [Zero-Configuration Discovery](development/zeroconf-discovery.md) | mDNS/DNS-SD zero-configuration discovery architecture, TXT records, feature flags, and testing. See also: [Zeroconf Guide](end-user/zeroconf-discovery.md), [Zeroconf Security](security/zeroconf-discovery.md). |

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
| [NATS](end-user/deployment/nats.md) | NATS JetStream deployment for multi-controller HA setups. |
| [External Scheduler](end-user/deployment/external-scheduler.md) | External scheduler binary deployment, enrollment, and credential flow. |
| [Docker](end-user/deployment/docker.md) | Docker Compose deployment with auto-enrollment and profiles. |

## Additional Resources

- API reference: `/swagger-ui` when built with the `swagger-ui` feature.
- AsyncAPI spec: [`crates/shared/wire/asyncapi.yaml`](../crates/shared/wire/asyncapi.yaml).
- Core project docs: [`README.md`](../README.md), [`CONTRIBUTING.md`](../CONTRIBUTING.md), [`ARCHITECTURE.md`](../ARCHITECTURE.md), [`SECURITY.md`](../SECURITY.md).
- TODO tracker: [`TODO.md`](../TODO.md).
