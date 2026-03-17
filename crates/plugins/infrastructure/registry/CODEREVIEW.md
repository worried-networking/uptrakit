# Code Review: `uptrakit-plugin-infrastructure-registry`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The registry crate remains the central integration point for plugin construction, validation, capability lookup, and extension exposure. The compile-time plugin embedding is an accepted tradeoff in this repository and is not treated as a defect in this review.

## Strengths

- The registry macro still eliminates a large amount of hand-written dispatch code.
- Validation and sample-config generation remain consistent across plugin types.
- The crate passed the current clippy and test sweep with no functional regressions.

## Active Findings

### [LOW] Secret masking and restoration still rely on JSON round-trips

- Dimension: architecture, allocation awareness
- Scope: `crates/plugins/infrastructure/registry/src/registry.rs`, plus the shared macro path in `crates/plugins/infrastructure/core/src/plugin_base.rs`
- Why it matters: deserializing, mutating, and reserializing plugin configs is acceptable for admin paths, but it still keeps masking behavior runtime-typed and allocation-heavy at the central registry boundary.
- Failure scenario: a future config-schema mismatch or secret-masking bug only surfaces at runtime because the registry path operates through `serde_json::Value` instead of strongly typed API boundaries.
