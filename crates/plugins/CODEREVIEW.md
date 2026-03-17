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

The plugin subsystem is in good shape overall. Config validation, SSRF defense, timeout
discipline, and unit coverage are broadly strong. This review cycle added extensibility findings
around the `SoftwareItemPatch` builder contract and confirmed the existing email panic risk.
The extension handler registration limitation (compile-time only) is an accepted architectural
tradeoff and is now documented explicitly.

## Strengths

- The small plugins remain easy to reason about and rely on shared infrastructure instead of
  duplicating HTTP, command, or secret-handling code.
- The generic-shell plugin now has focused unit tests for placeholder replacement and failure
  propagation.
- All notification plugins inherit consistent timeout and SSRF behavior from the shared client
  builder.
- Plugin HMAC signing (webhook) and HTML escaping (telegram) are correctly applied.

## Active Findings

### [MEDIUM] The email notification plugin still panics on malformed non-object config

- Dimension: fault tolerance, coding standards
- Scope: `crates/plugins/notifications/email/src/lib.rs`
- Why it matters: `merge_smtp_into_config()` calls `config.as_object_mut().expect("config is
  always an object")`. A malformed row or unexpected caller can take down the dispatch path instead
  of returning a typed error.
- Failure scenario: settings-store corruption, manual DB edit, or a buggy caller passes a non-
  object JSON value into the plugin path during notification delivery.

### [MEDIUM] `SoftwareItemPatch` builder contract is fragile for external implementors

- Dimension: extensibility, API stability
- Scope: `crates/plugins/infrastructure/core/src/plugin_base.rs`, `SoftwareItemPatch` struct
- Why it matters: `SoftwareItemPatch` uses a builder pattern with `#[non_exhaustive]`. All current
  fields are `Option<T>`, which makes the builder safe for now. If a non-optional field is ever
  added, every external `SoftwareItemLifecyclePlugin` implementation that constructs a patch via
  the builder will fail to compile.
- Fix: document explicitly that all future `SoftwareItemPatch` fields must remain optional, or
  provide a separate `SoftwareItemPatchBuilder` type that clearly owns the construction contract.

### [MEDIUM] Several umbrella-managed plugins are now monolithic enough to raise change risk

- Dimension: maintainability
- Scope: `crates/plugins/infrastructure/proxmox`, `crates/plugins/package-managers/apk`,
  `crates/plugins/package-managers/dnf`, `crates/plugins/notifications/email`
- Why it matters: the crates still compile and test cleanly, but they now concentrate multiple
  responsibilities in files that are hundreds to more than a thousand lines long.
- Failure scenario: a future resilience fix for remote execution, package parsing, or notification
  behavior lands in one branch of a monolithic file and regresses an adjacent concern.

### [LOW] `handle_service_extension_action()` fails silently when not overridden by infrastructure plugins

- Dimension: extensibility, developer experience
- Scope: `crates/plugins/infrastructure/core/src/plugin_base.rs:PluginBase::handle_service_extension_action`
- Why it matters: the method has a default `None` implementation. An infrastructure plugin that
  exposes `extension_manifests()` UI actions but forgets to override
  `handle_service_extension_action()` will silently ignore all agent-side extension action
  requests. The error message from the registry is generic and does not indicate which method is
  missing.
- Fix: add a documentation note on the method explaining that infrastructure plugins exposing
  extension manifests must override this method to handle agent-side dispatches. Consider a
  capability constant `ConfigurationExtensions` that the registry checks at registration time.

## Split/Merge Notes

- No merge is recommended for the small hook and generic plugins.
- `infrastructure/proxmox` is the clearest future split candidate inside the umbrella: API client,
  matching logic, agent actions, and extension UI can continue to evolve independently.
- Extension handler registration is compile-time via the `register_plugins!` macro — this is an
  accepted tradeoff for the current first-party-only plugin model. If third-party runtime plugins
  are ever required, the registry will need a dynamic handler registration mechanism.
