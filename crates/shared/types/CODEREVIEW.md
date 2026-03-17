# Code Review: `uptrakit-shared-types`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The crate is stable and well-tested, but it is still too broad for the number of downstream users it serves. This is now primarily a maintainability and crate-boundary concern rather than a correctness concern.

## Strengths

- The wire-safe enum work and SSRF helpers materially improved the crate relative to older review history.
- Serialization and round-trip coverage across the exported value types are good.
- No active security defects were confirmed in this pass.

## Active Findings

### [MEDIUM] The crate still mixes too many unrelated concerns behind a high-fanout boundary

- Dimension: maintainability, extensibility, crate structure
- Scope: `crates/shared/types/src/lib.rs`
- Why it matters: plugin types, discovery types, MQTT connection types, update-state enums, and auth-adjacent values still live together in one crate that almost the whole workspace imports.
- Failure scenario: a change needed by one subsystem triggers widespread rebuilds, broad review surfaces, and unclear ownership because the crate boundary is too coarse.

## Split/Merge Notes

- Best split candidate: move plugin/discovery-specific types closer to the plugin infrastructure crates.
- No merge is recommended; the problem is over-aggregation, not excessive fragmentation.
