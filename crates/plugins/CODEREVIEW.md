# Code Review: Plugins (Umbrella)

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

**Crates covered:** `uptrakit-plugin-generic-shell`

Non-trivial plugin crates have individual `CODEREVIEW.md` files in their respective directories:
`infrastructure/core`, `infrastructure/registry`, `releases/docker`, `releases/github`,
`package-managers/apt`, `package-managers/npm`, `package-managers/homebrew`, and
`discovery/proxmox-helper-scripts`.

## Summary

`uptrakit-plugin-generic-shell` (~418 LoC across 4 source files) is the simplest plugin in the
workspace. It executes user-configured shell commands for version checking and update execution.
The plugin delegates all command execution to `CommandExecutor` and follows the standard plugin
patterns established by `uptrakit-plugin-infrastructure-core`.

## Architecture

### Strengths

- `src/plugin.rs` -- Follows the standard `validate() -> new()` construction pattern enforced by
  `register_plugins!`.
- `src/config.rs` -- Configuration is minimal: command strings for check and update operations.
  No secrets, no complex state.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- Command execution delegated entirely to `CommandExecutor` with shell escaping.
- `SecretMasking` default (no-op) is correct -- no secrets in configuration.
- No `unsafe` blocks.

### Issues

No security issues found.

## Code Quality

### Strengths

- Clean, minimal implementation focused on shell command delegation.

### Issues

No code quality issues found.

## High Availability

### Strengths

- Stateless command execution. No shared mutable state.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Consistent with workspace patterns: `bail!`, `report!`, `thiserror`-derived errors.
- Zero `#[allow(clippy::...)]` suppressions.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- Generic shell command approach is inherently extensible -- any operation that can be expressed
  as a shell command is supported without code changes.

### Issues

No extensibility issues found.

---

## Cross-Cutting Plugin Findings

The following findings apply across the plugin subsystem and are documented here for reference.
Individual crate reviews contain the crate-specific details.

### Plugin Extension Checklist

When adding a new plugin, the following steps are required:

| Step | Location | Status |
| ------ | ---------- | -------- |
| New crate with `Plugin` + config struct | `crates/plugins/<name>/` | Clean |
| Implement `SecretMasking` | plugin config struct | Clean |
| One line in `register_plugins!` | `registry.rs` | Clean |
| Dependency in registry `Cargo.toml` | `registry/Cargo.toml` | Clean |
| New variant in `PluginType` | `shared/types/src/plugin_types.rs` | Clean |
| `as_str()`, `FromStr`, `Display` for new variant | `shared/types/src/plugin_types.rs` | Clean |
| **If discovery-capable:** include `PluginCapability::DiscoverLocalSoftware` in `capabilities()` | plugin crate | Clean |
| **If special identifier rules:** implement `validate_package_identifier` | plugin crate | Clean |

All previously "Manual" steps have been eliminated -- discovery support is now fully auto-derived
from the `register_plugins!` macro and the plugin's `capabilities()` method.
