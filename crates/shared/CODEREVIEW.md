# Code Review: `crates/shared` Umbrella

- Review date: 2026-03-17
- Scope: current-state review for shared crates without their own dedicated `CODEREVIEW.md`

## Covered Crates

- `uptrakit-audit-log`
- `uptrakit-backoff`
- `uptrakit-build-info`
- `uptrakit-directories`
- `uptrakit-extension-framework`
- `uptrakit-shared-macros`
- `uptrakit-config-merge`

## Summary

The shared utility layer remains strong overall. Most of these crates are small, stable, and easy to reason about. The only active concern in this umbrella is maintainability pressure in the largest schema/helper crates.

## Strengths

- `backoff`, `build-info`, `macros`, and `config-merge` stay small and focused.
- `directories` still provides good platform-aware permission handling and path validation.
- The shared crates continue to enforce workspace conventions instead of weakening them.

## Active Findings

### [MEDIUM] `uptrakit-extension-framework` is still a monolithic schema crate

- Dimension: maintainability, crate structure
- Scope: `crates/shared/extension-framework/src/lib.rs`
- Why it matters: the crate centralizes a large amount of wire-schema and UI-schema behavior in one file, which raises the risk of incidental regressions when new extension features are added.
- Failure scenario: a future extension-form or action change subtly changes existing serialization or validation because too many adjacent concerns live in one module.

### [LOW] `uptrakit-directories` is drifting toward the same monolithic shape

- Dimension: maintainability
- Scope: `crates/shared/directories/src/lib.rs`
- Why it matters: it still works well, but path expansion, permission hardening, validation, and I/O helpers are now packed into one large file.
- Failure scenario: platform-specific path behavior is changed for one call site and unintentionally affects another because the implementation surface is no longer small.
