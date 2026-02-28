# Code Review: uptrakit-plugin-infrastructure-registry

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-plugin-infrastructure-registry` (~1,292 LoC across 3 source files) provides the
`PluginRegistry` and `PluginOps` trait that dispatch to concrete plugin implementations. The
`register_plugins!` macro eliminates all dispatch duplication. Discovery capability is automatically
derived from each plugin's `capabilities()` method. The crate is well-tested with comprehensive
dispatch table coverage.

## Architecture

### Strengths

- `src/registry.rs:43-156` -- `register_plugins!` macro generates all six dispatch methods
  (`create_plugin`, `validate_config`, `mask_config_secrets`, `restore_config_secrets`,
  `create_plugin_for_discovery`, `discovery_plugins`) with consistent error handling. Adding a
  new plugin requires exactly one line in this macro invocation plus a `Cargo.toml` dependency.
- `src/lib.rs:57-86` -- `PluginOps` trait decouples the web API from the concrete registry.
  `AppState` holds `Arc<dyn PluginOps>`. Route handlers testable in isolation by substituting
  a mock implementation.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/registry.rs:17-39` -- `mask_secrets_for` and `restore_secrets_for` use JSON round-trip
  pattern, ensuring masking logic never diverges from the serialized representation.
- No `unsafe` blocks.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/registry.rs:244-597` -- Tests cover: config parsing for all plugins, valid/invalid
  configs for each, `create_plugin` round-trip for all, `mask_config_secrets` and
  `restore_config_secrets` for plugins with secrets, capability verification on constructed
  plugins, and string-type variants of all `PluginOps` methods.

### Issues

**[LOW]** `src/registry.rs:243` -- Tests use `LocalCommandExecutor` directly, not a mock
executor. Acceptable for construction and config tests since plugins are not invoked, but
introducing a `MockCommandExecutor` would enable more thorough registry-level tests.

## High Availability

### Strengths

- Plugin construction validated synchronously at configuration time. Runtime failures minimized.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Consistent `bail!` and `report!` usage. `RegistryError` with `thiserror`-derived `Display`.
- Zero `#[allow(clippy::...)]` suppressions.

### Issues

**[MEDIUM]** `src/registry.rs` -- No `#[must_use]` on `mask_config_secrets` and
`mask_config_secrets_str`. Both return a `serde_json::Value` representing the masked config.
If a caller forgets to use the return value, masking has no effect. Adding `#[must_use]` would
produce a compiler warning.

## Extensibility

### Strengths

- Adding any plugin requires exactly one line in the `register_plugins!` macro. For
  discovery-capable plugins, include `PluginCapability::DiscoverLocalSoftware` in
  `capabilities()` -- `discovery_plugins()` is fully auto-derived.
- `PluginOps` trait enables mock implementations for testing.

### Issues

**[MEDIUM]** `src/registry.rs:151-156` -- No feature-flag gating for platform-specific plugins.
`HomebrewPlugin` is macOS-specific, `ProxmoxHelperScriptsPlugin` is Proxmox VE-specific. Both
compiled unconditionally into all agent binaries. A Linux agent accepts valid `HomebrewPlugin`
configuration and fails only at runtime when `brew` is absent.
