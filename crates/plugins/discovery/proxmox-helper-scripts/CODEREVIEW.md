# Code Review: `uptrakit-plugin-discovery-proxmox-helper-scripts`

- Review date: 2026-03-17
- Reviewer: claude-opus-4-6
- Scope: full 14-dimension review

## Summary

The helper-scripts discovery plugin is functionally rich and has strong parsing and mapping tests.
The active concern is maintainability of the large discovery.rs file rather than immediate
correctness.

## Strengths

- Good coverage across slug parsing, package inference, and emitted target structure.
- Clean separation between discovery heuristics and emitted `DiscoveryTarget` shapes.
- No active security or SSRF findings were confirmed in this pass.
- Identifier validation is not applicable (discovery plugin, no user-supplied identifiers).

## Active Findings

### [MEDIUM] Discovery logic is concentrated in a 1655-line `discovery.rs`

- **Dimension**: maintainability
- **Scope**: `crates/plugins/discovery/proxmox-helper-scripts/src/discovery.rs` (1655 lines),
  `crates/plugins/discovery/proxmox-helper-scripts/src/plugin.rs` (1034 lines)
- **Description**: The crate mixes parsing heuristics, upstream inference, and target
  construction in monolithic files. Both files include substantial test sections, but the
  production logic itself spans multiple concerns.
- **Why it matters**: A new helper-script pattern or source type added under time pressure
  could unintentionally change existing GitHub, Codeberg, npm, or APT classification behavior.
- **Failure scenario**: A regex or pattern change in one discovery arm silently alters the
  output of another arm because the arms share a single match cascade.
