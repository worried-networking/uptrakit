# Code Review: `uptrakit-plugin-infrastructure-registry`

- Review date: 2026-03-17
- Reviewer: claude-opus-4-6
- Scope: full 14-dimension review

## Summary

The registry crate remains the central integration point for plugin construction, validation,
capability lookup, and extension exposure. The compile-time plugin embedding is an accepted
tradeoff and is not treated as a defect. This review confirmed the existing allocation-path
finding and validated the macro-expansion discoverability concern.

## Strengths

- The `register_plugins!` macro eliminates a large amount of hand-written dispatch code and
  keeps plugin additions purely additive (one line in the macro invocation).
- Validation and sample-config generation remain consistent across plugin types.
- `mask_config_secrets` and `mask_config_secrets_str` carry `#[must_use]`, preventing accidental
  use of masked output as authoritative config.
- The `mask_secrets_for` fallback correctly logs at error level and returns the original config,
  ensuring masking failures are never silent.
- `probe_plugin_host_compatibility` correctly handles the `#[non_exhaustive]` wildcard arm with
  `tracing::warn!` and a safe fallback (assume compatible).
- `compatible_sudo_commands_for_host` runs compatibility probes concurrently via
  `futures_util::future::join_all`, avoiding sequential latency.
- Notification plugin construction is feature-gated (`notifications-webhook`,
  `notifications-telegram`, `notifications-email`), keeping binary size minimal when channels
  are not needed.
- The `with_dashboard_icons` builder spawns the refresh loop with a `CancellationToken` for
  clean shutdown.

## Active Findings

### [MEDIUM] Secret masking and restoration still rely on JSON round-trips

- **Dimension**: architecture, allocation awareness
- **Scope**: `crates/plugins/infrastructure/registry/src/registry.rs`, plus the shared macro path
  in `crates/plugins/infrastructure/core/src/plugin_base.rs`
- **Description**: Deserializing, mutating, and reserializing plugin configs is acceptable for
  admin paths, but it keeps masking behavior runtime-typed and allocation-heavy at the central
  registry boundary.
- **Why it matters**: A future config-schema mismatch or secret-masking bug only surfaces at
  runtime because the registry path operates through `serde_json::Value` instead of strongly
  typed API boundaries.
- **Failure scenario**: A config type adds a field with `#[serde(deny_unknown_fields)]` or a
  non-default rename, and the round-trip silently drops or corrupts it. The error is only
  discovered when an admin saves config and finds the stored value differs from what was
  submitted.

### [LOW] Extension handler registration is compile-time only via the macro

- **Dimension**: extensibility, architecture
- **Scope**: `crates/plugins/infrastructure/registry/src/registry.rs`, `register_plugins!` macro
- **Description**: Adding a plugin with extension actions requires a single-line macro update,
  which is fine for first-party plugins. However, there is no runtime handler registration path.
- **Why it matters**: Any future requirement for dynamically loaded or third-party plugins to
  self-register extension handlers would require a new mechanism.
- **Note**: This is an accepted tradeoff for the current first-party-only model.

### [LOW] `register_plugins!` macro expansion is invisible to IDEs

- **Dimension**: developer experience, maintainability
- **Scope**: `crates/plugins/infrastructure/registry/src/registry.rs`, macro invocation
- **Description**: Generated dispatch methods (`create_plugin`, `validate_config`,
  `mask_config_secrets`, `handle_extension_action`, etc.) are not navigable via IDE "go to
  definition". New contributors must mentally expand the macro or run `cargo expand` to
  understand the generated API.
- **Why it matters**: The macro is elegant and correct, but the generated API surface is large
  (20+ methods) and not self-documenting from an IDE perspective.
