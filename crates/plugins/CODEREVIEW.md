# Code Review: `crates/plugins` Umbrella

- Review date: 2026-03-17
- Scope: current-state review for plugin crates without their own dedicated `CODEREVIEW.md`

## Covered Crates

- `uptrakit-plugin-generic-shell`
- `uptrakit-plugin-hook-shell`
- `uptrakit-plugin-hook-systemd`
- `uptrakit-plugin-infrastructure-proxmox`
- `uptrakit-notification-plugin-core`
- `uptrakit-notification-plugin-email`
- `uptrakit-notification-plugin-telegram`
- `uptrakit-notification-plugin-webhook`
- `uptrakit-plugin-package-manager-apk`
- `uptrakit-plugin-package-manager-cargo`
- `uptrakit-plugin-package-manager-dnf`
- `uptrakit-plugin-package-manager-mas`
- `uptrakit-plugin-package-manager-pacman`
- `uptrakit-plugin-package-manager-pkg`
- `uptrakit-plugin-package-manager-snap`

## Summary

The plugin subsystem is in much better shape than the older append-only reviews suggested. Config validation, SSRF defense, timeout discipline, and unit coverage are broadly strong. The active issues are now limited to one real panic surface and a few large umbrella-managed crates that are getting harder to change safely.

## Strengths

- The small plugins remain easy to reason about and rely heavily on shared infrastructure instead of duplicating HTTP, command, or secret-handling code.
- The older generic-shell test gap is resolved; the crate now has focused unit tests for placeholder replacement and failure propagation.
- The notification plugins inherit consistent timeout and SSRF behavior from the shared client builder.

## Active Findings

### [MEDIUM] The email notification plugin still panics on malformed non-object config

- Dimension: fault tolerance, coding standards
- Scope: `crates/plugins/notifications/email/src/lib.rs:97`
- Why it matters: `merge_smtp_into_config()` still does `config.as_object_mut().expect("config is always an object")`. A malformed row or unexpected caller can still take down the dispatch path instead of returning a typed error.
- Failure scenario: settings-store corruption, manual DB edit, or a buggy caller passes a non-object JSON value into the plugin path during notification delivery.

### [MEDIUM] Several umbrella-managed plugins are now monolithic enough to raise change risk

- Dimension: maintainability
- Scope: `crates/plugins/infrastructure/proxmox`, `crates/plugins/package-managers/apk`, `crates/plugins/package-managers/dnf`, `crates/plugins/notifications/email`
- Why it matters: the crates still compile and test cleanly, but they now concentrate multiple responsibilities in files that are hundreds to more than a thousand lines long.
- Failure scenario: a future resilience fix for remote execution, package parsing, or notification behavior lands in one branch of a monolithic file and regresses an adjacent concern.

## Split/Merge Notes

- No merge is recommended for the small hook and generic plugins.
- `infrastructure/proxmox` is the clearest future split candidate inside the umbrella: API client, matching logic, agent actions, and extension UI can continue to evolve independently.
