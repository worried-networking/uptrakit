# Uptrakit Documentation

Entry point for all Uptrakit documentation.

## Core Documents

| Document | Description |
| --- | --- |
| [README.md](../README.md) | Project overview, quick-start commands, license |
| [ARCHITECTURE.md](../ARCHITECTURE.md) | System design, component diagram, technology stack, key decisions |
| [SECURITY.md](../SECURITY.md) | Security policy, vulnerability reporting, cryptographic details, certificate lifecycle |
| [CONTRIBUTING.md](../CONTRIBUTING.md) | Development setup, testing, commit conventions, PR guidelines |
| [AGENTS.md](../AGENTS.md) | AI agent guide: codebase layout, quality gates, error handling patterns, architecture rules |
| [TODO.md](../TODO.md) | Project roadmap and progress tracker |

## API Documentation

| Resource | Description |
| --- | --- |
| OpenAPI / Swagger UI | Available at `/swagger-ui` when the controller is built with the `swagger-ui` feature flag |
| [AsyncAPI spec](../crates/shared/wire/asyncapi.yaml) | Agent-controller WebSocket wire protocol (message types, payloads, connection lifecycle) |

## Planned Documentation

The following guides are planned but not yet written. Contributions are welcome.

- **Getting Started Guide** -- End-to-end walkthrough: install the controller, enroll an agent, add a software item, trigger a version check.

- **Deployment Guide** -- System requirements, network configuration, systemd service setup, Docker deployment, database backend selection, TLS configuration.

- **Provider Development Guide** -- How to implement a new provider: trait implementation, remote vs. local split, version comparison, testing, and configuration fields.

- **Home Assistant Integration Guide** -- MQTT broker setup, auto-discovery configuration, entity attributes, triggering updates from Home Assistant.

- **CLI Reference** -- Command structure, output formatting options, configuration file, and usage examples for `uptrakit-cli`.

- **Troubleshooting Guide** -- Common issues: agent connection failures, certificate problems, enrollment errors, database migration issues, MQTT connectivity.
