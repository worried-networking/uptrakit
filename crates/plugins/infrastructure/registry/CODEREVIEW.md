# Code Review: `uptrakit-plugin-infrastructure-registry`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The registry crate remains the central integration point for plugin construction, validation,
capability lookup, and extension exposure. The compile-time plugin embedding is an accepted
tradeoff and is not treated as a defect. This review confirmed the existing allocation-path finding
and added a new low-severity finding about macro-expansion discoverability.

## Strengths

- The `register_plugins!` macro still eliminates a large amount of hand-written dispatch code and
  keeps plugin additions purely additive (one line in the macro invocation).
- Validation and sample-config generation remain consistent across plugin types.
- `mask_config_secrets` and `mask_config_secrets_str` carry `#[must_use]`, preventing accidental
  use of masked output as authoritative config.
- The crate passed the current clippy and test sweep with no functional regressions.

## Active Findings

### [MEDIUM] Secret masking and restoration still rely on JSON round-trips

- Dimension: architecture, allocation awareness
- Scope: `crates/plugins/infrastructure/registry/src/registry.rs`, plus the shared macro path in
  `crates/plugins/infrastructure/core/src/plugin_base.rs`
- Why it matters: deserializing, mutating, and reserializing plugin configs is acceptable for admin
  paths, but it still keeps masking behavior runtime-typed and allocation-heavy at the central
  registry boundary.
- Failure scenario: a future config-schema mismatch or secret-masking bug only surfaces at runtime
  because the registry path operates through `serde_json::Value` instead of strongly typed API
  boundaries.

### [LOW] Extension handler registration is compile-time only via the macro

- Dimension: extensibility, architecture
- Scope: `crates/plugins/infrastructure/registry/src/registry.rs`, `register_plugins!` macro
- Why it matters: adding a plugin with extension actions requires a single-line macro update, which
  is fine for first-party plugins. However, there is no runtime handler registration path. Any
  future requirement for dynamically loaded or third-party plugins to self-register extension
  handlers would require a new mechanism (e.g., `PluginRegistry::register_extension_handler()`).
- Note: this is an accepted tradeoff for the current first-party-only model. Document it
  explicitly in `AGENTS.md` if it is not already noted there.

### [LOW] `register_plugins!` macro expansion is invisible to IDEs

- Dimension: developer experience, maintainability
- Scope: `crates/plugins/infrastructure/registry/src/registry.rs`, macro invocation
- Why it matters: generated dispatch methods (`create_plugin`, `validate_config`,
  `mask_config_secrets`, `handle_extension_action`, etc.) are not navigable via IDE "go to
  definition". New contributors must mentally expand the macro or run `cargo expand` to understand
  the generated API.
- Fix: maintain the macro (it is elegant), but add a documentation comment at `impl PluginRegistry`
  listing all generated methods and pointing to the macro definition. Add a
  `docs/development/plugin-guidelines.md` section: "The `register_plugins!` macro generates all
  dispatch methods automatically; see the macro definition for the full expansion pattern."
