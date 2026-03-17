# Code Review: `uptrakit-plugin-infrastructure-core`

- Review date: 2026-03-17
- Reviewer: claude-opus-4-6
- Scope: full 14-dimension review

## Summary

This crate remains one of the strongest foundations in the workspace. It centralizes plugin traits,
the shared HTTP client builder, command helpers, and testing executors without carrying obvious
active defects. The `SoftwareItemPatch` builder fragility is the only active finding.

## Strengths

- `build_plugin_http_client()` keeps SSRF policy (strict and permissive modes), timeout policy
  (10s connect, 60s request), and TLS policy centralized. No plugin has been found to bypass
  this builder (except Dashboard Icons -- documented in the umbrella review).
- `execute_and_capture()` removes repeated command-execution error mapping from plugin crates.
  All package-manager plugins confirmed using this helper.
- `NoopCommandExecutor` (exposed via `uptrakit_command`) provides a correct no-op implementation
  for controller-side use.
- The testing helpers materially improve package-manager plugin testability.
- `PluginCapability` is `#[non_exhaustive]` and uses the `Other(String)` wire-safe pattern.
- `PluginBase::mask_config_secrets` carries `#[must_use]`, preventing accidental discard.
- `restore_config_secrets` correctly handles non-object JSON by returning the input unchanged.
- The `impl_plugin_base_config!` macro eliminates manual `PluginBase` boilerplate and ensures
  consistent config validation, masking, and form schema delegation across all plugins.
- `SudoCommandEntry` has builder methods with `#[must_use]`, and `SudoHelperScript` uses
  `&'static str` to prevent runtime-constructed paths.
- `HostCompatibility`, `PreUpdateHookResult`, `UpdateLifecycleContext`, `DeliveryMessage`,
  `MessageAction`, `SoftwareItemCreatedEvent`, and `SoftwareItemPatch` all carry
  `#[non_exhaustive]` correctly.

## Active Findings

### [MEDIUM] `SoftwareItemPatch` builder contract is fragile if non-optional fields are ever added

- **Dimension**: extensibility, API stability
- **Scope**: `crates/plugins/infrastructure/core/src/plugin_base.rs`, `SoftwareItemPatch` struct
- **Description**: `SoftwareItemPatch` is `#[non_exhaustive]` with a builder pattern where all
  current fields are `Option<T>`. External `SoftwareItemLifecyclePlugin` implementations
  construct patches using the builder's `with_*()` methods. If a non-optional field is ever added
  to `SoftwareItemPatch`, all external implementations fail to compile because they cannot supply
  the new required value through the existing builder methods.
- **Why it matters**: The risk is contained while all fields remain optional, but this is a
  forward-fragility concern that grows as the patch type evolves.
- **Failure scenario**: Adding a non-optional field to `SoftwareItemPatch` breaks all downstream
  `SoftwareItemLifecyclePlugin` implementations at compile time.
