# Code Review: Plugins (Umbrella)

- **Review date**: 2026-03-06
- **Parallel review date**: 2026-03-06
- **Reviewer**: AI code review (architecture|security|quality|HA|standards|extensibility|tests|consistency|maintainability|database|crate-structure)
- **Branch**: docs/codereview-backend

**Crates covered:** `uptrakit-plugin-generic-shell`

Non-trivial plugin crates have individual `CODEREVIEW.md` files in their respective directories:
`infrastructure/core`, `infrastructure/registry`, `releases/docker`, `releases/github`,
`package-managers/apt`, `package-managers/npm`, `package-managers/homebrew`, and
`discovery/proxmox-helper-scripts`.

## Summary

`uptrakit-plugin-generic-shell` (~418 LoC across 4 source files) is the simplest plugin in the
workspace. It executes user-configured shell commands for version checking and update execution.
The plugin delegates all command execution to `CommandExecutor` and follows the standard plugin
patterns established by `uptrakit-plugin-infrastructure-core`.

## Architecture

### Strengths

- `src/plugin.rs` -- Follows the standard `validate() -> new()` construction pattern enforced by
  `register_plugins!`.
- `src/config.rs` -- Configuration is minimal: command strings for check and update operations.
  No secrets, no complex state.

### Issues

No architectural issues found for `generic-shell`.

**[LOW]** Plugin config validation across all plugins happens via a JSON round-trip pattern:
`mask_secrets_for<T>` in the registry deserializes config from `serde_json::Value`, calls
`with_secrets_masked()`, then re-serializes. This double serialization occurs on every API
response that includes plugin configs. While not a hot path, it is architecturally wasteful.
(Confirmed by Architecture and Extensibility parallel reviews.)

## Security and Safety

### Strengths

- Command execution delegated entirely to `CommandExecutor` with shell escaping.
- `SecretMasking` default (no-op) is correct -- no secrets in configuration.
- No `unsafe` blocks.
- All plugins building `reqwest::Client` set `.connect_timeout(10s)` and `.timeout(60s)`,
  satisfying the workspace HTTP client requirement. (Confirmed by Security parallel review.)
- Plugin SSRF protection is sound: GitHub enforces HTTPS-only and rejects private hosts,
  Docker checks the registry host against `is_private_host()`, GitLab and Forgejo enforce
  HTTPS and reject private/loopback addresses. (Confirmed by Security parallel review.)

### Issues

No security issues found.

## Code Quality

### Strengths

- Clean, minimal implementation focused on shell command delegation.

### Issues

No code quality issues found.

## High Availability

### Strengths

- Stateless command execution. No shared mutable state.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Consistent with workspace patterns: `bail!`, `report!`, `thiserror`-derived errors.
- Zero `#[allow(clippy::...)]` suppressions across most plugins.

### Issues

**[MEDIUM]** `crates/plugins/package-managers/mas/src/plugin.rs:143` -- `#[allow(dead_code)]`
on the `config` field of `MasPlugin`. The comment says "never read after construction." Per the
project coding standard, no `#[allow(clippy::...)]` or `#[allow(dead_code)]` suppressions are
approved. The field should be removed, prefixed with `_config`, or given a trivial accessor
method to eliminate the suppression. (Confirmed by Coding Standards parallel review, finding #6.)

**[LOW]** `crates/plugins/infrastructure/proxmox/src/client.rs:73` -- `.as_u16()` used in a
`tracing::trace!` structured field (`status = status.as_u16()`). The coding standard approves
`.as_u16()` only inside serde serialization helpers. Tracing structured fields should use
`status = %status` or `status = ?status` instead. (Confirmed by Coding Standards parallel
review, finding #9.)

## Extensibility

### Strengths

- Generic shell command approach is inherently extensible -- any operation that can be expressed
  as a shell command is supported without code changes.

### Issues

No extensibility issues found.

## Tests

### Strengths

- `src/config.rs` and `src/plugin.rs` for the shell plugin follow the project convention:
  configuration tests (default values, serialisation round-trips) and plugin construction
  tests are present via the registry-level tests in `infrastructure/registry`.

### Issues

**[MEDIUM]** `crates/plugins/generic/shell/src/plugin.rs` -- The shell plugin has no unit
tests for `check_version`, `execute_update`, or the command-construction logic. The plugin
relies on `CommandExecutor` dependency injection, making it straightforward to test with a
`FixedOutputExecutor` mock (as used in the npm plugin). At minimum, a test verifying that the
configured command string is passed correctly to the executor would prevent regressions if
command-building logic changes.

**[MEDIUM]** 22+ `thiserror` Display format tests across plugin error modules violate the
project testing philosophy documented in `docs/development/testing.md`. These tests construct
an error variant with known input and assert the `to_string()` output matches the
`#[error("...")]` format string. They test `thiserror`'s formatting behavior, not application
logic. Affected files:

- `crates/plugins/releases/docker/src/error.rs` (lines 60-138) -- 10 tests
- `crates/plugins/releases/github/src/error.rs` (lines 46-68) -- 3 tests
- `crates/plugins/releases/gitlab/src/error.rs` (lines 46-64) -- 3 tests
- `crates/plugins/releases/forgejo/src/error.rs` (lines 46-64) -- 3 tests
- `crates/plugins/infrastructure/proxmox/src/error.rs` (lines 44-54) -- 2 tests

Per the testing philosophy, these tests should be removed because they test upstream crate
behavior (`thiserror` formatting), not application logic. Tests for custom `Display`
implementations (where `Display` delegates to hand-written `as_str()` matches) are internal
logic tests and correctly remain. (Confirmed by Tests parallel review, finding 2.1.)

---

## Cross-Cutting HTTP Reliability

### Issues

**[MEDIUM]** `crates/plugins/package-managers/npm/src/plugin.rs:411-421` -- The npm plugin
has no retry logic for transient HTTP failures (network timeout, 429 rate limit, 5xx server
error). A single failed request to the npm registry causes the entire release fetch to fail.
The same gap exists at [LOW] severity in the release plugins: `releases/github`, `releases/forgejo`,
and `releases/gitlab` all make HTTP calls with no retry on `is_connect()` or `is_timeout()`
errors. The workspace `uptrakit-backoff` crate provides the `Backoff` primitive needed for
a consistent retry implementation. Per-crate details are in the respective `CODEREVIEW.md`
files (`npm`: [MEDIUM], `github`/`forgejo`/`gitlab`: [LOW]).

**[MEDIUM]** `crates/plugins/package-managers/npm/src/plugin.rs:134-142` -- The npm plugin
hardcodes the registry URL to `https://registry.npmjs.org`, preventing use with private
registries (Verdaccio, GitHub Packages, Artifactory). The URL should be an optional
`NpmConfig` field defaulting to the public registry. Per-crate detail in
`crates/plugins/package-managers/npm/CODEREVIEW.md`.

---

## Cross-Cutting Plugin Findings

The following findings apply across the plugin subsystem and are documented here for reference.
Individual crate reviews contain the crate-specific details.

### Plugin Extension Checklist

When adding a new plugin, the following steps are required:

| Step | Location | Status |
| ------ | ---------- | -------- |
| New crate with `Plugin` + config struct | `crates/plugins/<name>/` | Clean |
| Implement `SecretMasking` | plugin config struct | Clean |
| One line in `register_plugins!` | `registry.rs` | Clean |
| Dependency in registry `Cargo.toml` | `registry/Cargo.toml` | Clean |
| New variant in `PluginType` | `shared/types/src/plugin_types.rs` | Clean |
| `as_str()`, `FromStr`, `Display` for new variant | `shared/types/src/plugin_types.rs` | Clean |
| **If discovery-capable:** include `PluginCapability::DiscoverLocalSoftware` in `capabilities()` | plugin crate | Clean |
| **If special identifier rules:** implement `validate_package_identifier` | plugin crate | Clean |

All previously "Manual" steps have been eliminated -- discovery support is now fully auto-derived
from the `register_plugins!` macro and the plugin's `capabilities()` method.
