# Code Review: Plugins (Umbrella)

- **Review date**: 2026-03-06
- **Parallel review date**: 2026-03-06
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
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

No architectural issues found for `generic-shell`.

**[LOW]** Plugin config validation across all plugins happens via a JSON round-trip pattern:
`mask_secrets_for<T>` in the registry deserializes config from `serde_json::Value`, calls
`with_secrets_masked()`, then re-serializes. This double serialization occurs on every API
response that includes plugin configs. While not a hot path, it is architecturally wasteful.
(Confirmed by Architecture and Extensibility parallel reviews.)

## Security and Safety

### Strengths

- Command execution delegated entirely to `CommandExecutor` with shell escaping.
- `SecretMasking` default (no-op) is correct -- no secrets in configuration.
- No `unsafe` blocks.
- All plugins building `reqwest::Client` set `.connect_timeout(10s)` and `.timeout(60s)`,
  satisfying the workspace HTTP client requirement. (Confirmed by Security parallel review.)
- Plugin SSRF protection is sound: GitHub enforces HTTPS-only and rejects private hosts,
  Docker checks the registry host against `is_private_host()`, GitLab and Forgejo enforce
  HTTPS and reject private/loopback addresses. (Confirmed by Security parallel review.)

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
- Zero `#[allow(clippy::...)]` suppressions across most plugins.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- Generic shell command approach is inherently extensible -- any operation that can be expressed
  as a shell command is supported without code changes.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `src/config.rs` and `src/plugin.rs` for the shell plugin follow the project convention:
  configuration tests (default values, serialisation round-trips) and plugin construction
  tests are present via the registry-level tests in `infrastructure/registry`.

### Issues

**[MEDIUM]** `crates/plugins/generic/shell/src/plugin.rs` -- The shell plugin has no unit
tests for `check_version`, `execute_update`, or the command-construction logic. The plugin
relies on `CommandExecutor` dependency injection, making it straightforward to test with a
`FixedOutputExecutor` mock (as used in the npm plugin). At minimum, a test verifying that the
configured command string is passed correctly to the executor would prevent regressions if
command-building logic changes.

No test issues found.

---

## Cross-Cutting HTTP Reliability

### Issues

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
