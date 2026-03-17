# Code Review: `uptrakit-plugin-infrastructure-core`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

This crate remains one of the strongest foundations in the workspace. It centralizes plugin traits,
the shared HTTP client builder, command helpers, and testing executors without carrying obvious
active defects. This review added one new finding around the `SoftwareItemPatch` builder contract
for future extensibility.

## Strengths

- `build_plugin_http_client()` keeps SSRF policy (strict and permissive modes), timeout policy
  (10 s connect, 60 s request), and TLS policy centralized. No plugin has been found to bypass
  this builder.
- `execute_and_capture()` removes repeated command-execution error mapping from plugin crates.
- `NoopCommandExecutor` (exposed via `uptrakit_command`) provides a correct no-op implementation
  for controller-side use.
- The testing helpers materially improve package-manager plugin testability.
- `PluginCapability` is `#[non_exhaustive]` and uses the `Other(String)` wire-safe pattern.

## Active Findings

### [MEDIUM] `SoftwareItemPatch` builder contract is fragile if non-optional fields are ever added

- Dimension: extensibility, API stability
- Scope: `crates/plugins/infrastructure/core/src/plugin_base.rs`, `SoftwareItemPatch` struct
- Why it matters: `SoftwareItemPatch` is `#[non_exhaustive]` with a builder pattern where all
  current fields are `Option<T>`. External `SoftwareItemLifecyclePlugin` implementations
  construct patches using the builder's `with_*()` methods. If a non-optional field is ever added
  to `SoftwareItemPatch`, all external implementations fail to compile because they cannot supply
  the new required value through the existing builder methods.
- The risk is contained while all fields remain optional, but this is a forward-fragility concern
  that grows as the patch type evolves.
- Fix: document that `SoftwareItemPatch` will only ever have `Option<T>` fields, or provide a
  typed `SoftwareItemPatchBuilder` struct that is explicitly the stable construction API.
