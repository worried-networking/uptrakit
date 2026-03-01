# Code Review: uptrakit-plugin-discovery-proxmox-helper-scripts

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-plugin-discovery-proxmox-helper-scripts` (~2,190 LoC across 4 source files) provides
Proxmox VE helper script discovery, version checking, and update execution. It supports multiple
GitHub release sources, path-traversal validation on owner/repo fields, and a two-context config
design that allows discovery with minimal configuration.

The main concerns are the `script_url` empty-string sentinel pattern (type-unsafe) and the
platform-specific nature of the plugin being compiled unconditionally.

## Architecture

### Strengths

- `src/config.rs:61-118` -- Two-context design: `script_url` defaults to empty string via
  `#[serde(default)]`, allowing `{}` to deserialize for discovery. `validate()` rejects empty
  URL for version-check/update contexts. Comments at lines 63-66 document the invariant.
- `src/discovery.rs` -- Comprehensive Proxmox helper script discovery with parsing of installed
  scripts and their metadata.

### Issues

No architectural issues found.

## Security and Safety

### Strengths

- `src/config.rs:47-55` -- Path traversal validation explicitly rejects `owner` and `repo`
  values containing `/` or `..`, defending against URL path traversal in GitHub API URLs.
- `SecretString` used for `GitHubReleaseSource.auth_token`.
- No `unsafe` blocks.

### Issues

No security issues found.

## Code Quality

### Strengths

- `src/config.rs:249-276` -- Explicit tests for `owner` containing `/` and `repo` containing
  `..`, covering both path-traversal vectors in `GitHubReleaseSource.validate()`.
- `src/discovery.rs` -- Comprehensive parsing tests for Proxmox helper script output.

### Issues

No code quality issues found.

## High Availability

### Strengths

- Command execution is stateless. Each discovery/check/update invocation is independent.

### Issues

No high availability issues found.

## Coding Standards

### Strengths

- Consistent `bail!` and `report!` usage.
- `skip_serializing_if = "Option::is_none"` on optional fields.
- Zero `#[allow(clippy::...)]` suppressions.

### Issues

No coding standards issues found.

## Extensibility

### Strengths

- Multiple `GitHubReleaseSource` instances per config, allowing tracking of scripts from
  different repositories.
- `CommandExecutor` DI enables testing without Proxmox environment.

### Issues

**[LOW]** `src/config.rs:67` -- `script_url` empty-string default is a semantic workaround.
The `String` type with empty-string sentinel conflates "not provided" with "explicitly set to
empty". Using `Option<String>` for `script_url` with `#[serde(default)]` would make the
distinction type-safe: `None` means "not provided" (valid for discovery), `Some("")` would
remain an error from `validate()`.

**[LOW]** No shared abstraction for "config valid for discovery but not for update". A future
plugin with a similar split will face the same design challenge and may solve it differently.

## Tests

### Strengths

- `src/discovery.rs:836-1003+` -- 15+ tests cover the full discovery parsing pipeline: empty
  output, single script, multiple scripts, script with missing fields, duplicate URL
  deduplication, output with comment lines, malformed JSON lines, and target synthesis
  (converting a discovered script into a `DiscoveryTarget` with the correct `package_identifier`
  and metadata).
- `src/config.rs:49-90` -- Five tests cover `GitHubReleaseSource` validation: valid source,
  missing owner, missing repo, owner containing `/`, and repo containing `..`. Path traversal
  validation is tested explicitly.
- Tests for parsing are pure synchronous functions requiring no async runtime or subprocess,
  making the suite fast and fully deterministic.

### Issues

**[MEDIUM]** `src/plugin.rs` -- `fetch_releases`, `detect_installed_version`, and
`execute_update` have no tests. These are the primary update-path operations and each
involves HTTP requests (for release fetching via GitHub API). No mock executor or mock HTTP
server exercises the command-construction logic or HTTP interaction paths. The Proxmox
plugin shares the GitHub release-fetching pattern with the GitHub plugin, which also lacks
HTTP mock tests. At minimum, a mock executor test for `detect_installed_version` (runs a
shell command, parses the version from stdout) would verify the command construction and
output parsing without requiring a network connection.
