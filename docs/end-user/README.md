---
title: Overview
weight: 1
description: User-facing guides for operating Uptrakit, covering update workflows, deployment orientation, and Home Assistant integration.
---

# End-user Documentation

This folder contains user-facing guides for operating Uptrakit, including update workflows, deployment orientation, and Home Assistant integration.

## Contents

| Document | Description |
| --- | --- |
| [System Overview](system-overview.md) | High-level architecture and operational model for controller, agents, and plugins. |
| [CLI Usage Guide](cli-usage.md) | Command reference for the `uptrakit` binary (all command groups with examples). |
| [Plugin Configurations](plugin-configs.md) | Managing plugin configs, supported plugin types, and autodiscovery. |
| [Manual Software Tracking](manual-software-tracking.md) | Setting up tracking for software that cannot be autodiscovered (e.g. standalone binaries from GitHub releases). |
| [Autodiscovery](autodiscovery.md) | How autodiscovery works, reviewing pending items, and the ignore list. |
| [Update Workflow](update-workflow.md) | Manual update process, scheduling behavior, and history/reporting expectations. |
| [Update History](update-history.md) | Viewing update history, status reference, and triggering updates from the web UI. |
| [Notifications](notifications.md) | Notification channels, rules, event types, and delivery log. |
| [Profile and API Tokens](profile-tokens.md) | Account info, API token lifecycle, and security best practices. |
| [Home Assistant and MQTT](home-assistant-mqtt.md) | MQTT setup and Home Assistant update entity integration. |
| [Deployment Map](deployment-map.md) | Navigation guide for deployment-related docs and configuration entry points. |
| [Deployment Guides](deployment/README.md) | Deployment-specific references, including reverse proxy setup guidance. |
| [Docker Deployment](deployment/docker.md) | Docker Compose deployment with auto-enrollment and profiles. |
| [Database Data Migration](db-migration.md) | Moving data from SQLite to PostgreSQL using `uptrakit-controller db-migrate`. |

## Related Documentation

- Top-level docs catalogue: [`docs/README.md`](../README.md)
- API and protocol behavior: [`docs/api/README.md`](../api/README.md)
- Security controls and hardening: [`docs/security/README.md`](../security/README.md)
- Contributor/developer setup: [`docs/development/README.md`](../development/README.md)
