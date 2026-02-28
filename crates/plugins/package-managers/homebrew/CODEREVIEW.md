# Code Review: uptrakit-plugin-package-manager-homebrew

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-plugin-package-manager-homebrew` (~1,059 LoC across 4 source files) provides Homebrew
integration for version checking, update execution, and software discovery on macOS. It correctly
distinguishes between formulae and casks, uses `CommandExecutor` for dependency injection, and
parses JSON output from `brew info --json=v2` fixtures in tests.

The crate is macOS-specific but compiled unconditionally into all agent binaries (tracked as a
registry-level issue). The test suite is thorough for parsing logic but lacks coverage for the
full async method paths.

## Architecture

### Strengths

- `src/plugin.rs` -- Clean separation between parsing helpers (`parse_installed_formulae`,
  `parse_installed_casks`, `parse_installed_version`, `parse_latest_version`) and the async
  plugin trait methods. Parsing logic is independently testable.
- `src/config.rs` -- `HomebrewPackageType` default is `None` at the config level, correctly
  distinguishing "discover all" from "track a specific type".

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- No secrets in configuration. `SecretMasking` default (no-op) is correct.
- No `unsafe` blocks.
- Command execution delegated to `CommandExecutor` with shell escaping.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/plugin.rs:428-724` -- All JSON parsing helpers tested with in-process fixtures. No live
  `brew` binary required for the unit test suite.
- `src/plugin.rs:98-114` -- `parse_installed_formulae`/`parse_installed_casks` skip items
  missing `name` or `version` fields with `continue`. Correct behavior for partially corrupt
  `brew info` output.
- `src/plugin.rs:182-188` -- `is_cask()` helper explicitly documents the distinction between
  formula and cask tracking modes.

### Issues

**[LOW]** `src/plugin.rs:708-722` -- `detect_installed_version` and `fetch_releases` tested only
for the empty-identifier guard. No tests exercise the JSON parsing code path inside these async
methods using sample JSON fixtures, even though `parse_installed_version` and
`parse_latest_version` helpers are tested directly.

## High Availability

### Strengths

- Command execution is stateless. Each call invokes `brew` as a fresh subprocess. No shared
  mutable state means no races between concurrent calls.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Consistent `bail!` and `report!` usage.
- `#[serde(rename_all = "snake_case")]` applied to config types.
- Zero `#[allow(clippy::...)]` suppressions.

### Issues

**[MEDIUM]** `src/plugin.rs` -- All async plugin tests use bare `#[tokio::test]`. Per
`testing.md`, `start_paused = true` is required for all async tests.

## Extensibility

### Strengths

- `HomebrewPackageType` enum allows formula/cask distinction.
- `CommandExecutor` DI enables testing without real `brew` binary.

### Issues

No extensibility issues found.
