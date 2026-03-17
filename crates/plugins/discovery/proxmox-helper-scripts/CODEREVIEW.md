# Code Review: `uptrakit-plugin-discovery-proxmox-helper-scripts`

- Review date: 2026-03-17
- Scope: current-state review

## Summary

The helper-scripts discovery plugin is functionally rich and now has strong parsing and mapping tests. The active concern is maintainability rather than immediate correctness.

## Strengths

- Good coverage across slug parsing, package inference, and emitted target structure.
- Clean separation between discovery heuristics and emitted `DiscoveryTarget` shapes.
- No active security or SSRF findings were confirmed in this pass.

## Active Findings

### [MEDIUM] Discovery logic is still concentrated in very large files

- Dimension: maintainability
- Scope: `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs`, `crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs`
- Why it matters: the crate mixes parsing heuristics, upstream inference, and target construction in monolithic files. That makes it harder to extend or to reason about obscure edge cases without broad regression risk.
- Failure scenario: a new helper-script pattern or source type is added under time pressure and unintentionally changes existing GitHub, Codeberg, npm, or APT classification behavior.
