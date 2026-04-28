---
title: Plugin Reference
weight: 1
description: Per-plugin configuration reference for the Uptrakit package-manager plugins.
---

Uptrakit ships built-in plugins for common Linux package managers. Each plugin page
documents the configuration schema, supported options, and behavior notes specific to
that package manager.

| Plugin | Description |
| --- | --- |
| [APT](apt.md) | Debian and Ubuntu package management via `apt`. |
| [DNF](dnf.md) | Fedora and RHEL package management via `dnf`. |
| [Docker](docker.md) | Docker image and container update tracking. |
| [Pacman](pacman.md) | Arch Linux package management via `pacman`. |

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — managing plugin configs, supported plugin types, and autodiscovery.
- [Manual Software Tracking](../manual-software-tracking.md) — setting up tracking for software that cannot be autodiscovered.
