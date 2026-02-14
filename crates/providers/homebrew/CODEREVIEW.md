# Code Review: uptrakit-provider-homebrew

## Summary

Homebrew formulae/cask provider crate (~700 lines across 5 source files) implementing the `Provider` trait for tracking and managing Homebrew packages. Supports both formula and cask package types, local software discovery, package index refresh, and direct-exec command execution.

## Architecture

- **Module structure**: `lib.rs` re-exports from `provider.rs`, `config.rs`, `types.rs`, `error.rs`.
- **Public API surface**: `HomebrewProvider`, `HomebrewConfig`, `HomebrewError`.
- **Dependency choices**: `uptrakit-provider-core` (sole uptrakit dependency -- correct), `serde`/`serde_json`, `rootcause`/`thiserror`. No HTTP client or regex needed -- lightest provider.
- **Layering**: Leaf provider crate. Depends only on `uptrakit-provider-core`.

## Security and Safety

- **Input validation**: `require_package_identifier()` prevents empty identifiers from reaching shell commands.
- **Injection prevention**: All `brew` commands use `run_command_exec` (direct exec, no shell).
- **"latest" version filtering**: Correctly filters out the Homebrew `"latest"` sentinel from cask versions.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: `HomebrewError` enum. Uses `rootcause::Report` wrapper.
- **Test coverage**: Tests cover JSON parsing for installed versions, latest versions, package discovery, configuration, and `HomebrewPackageType` serde.
- **Dual capability**: Only provider that declares both `DiscoverLocalSoftware` and `RefreshPackageIndex` capabilities.

## Coding Standards Compliance

- Typed error enum with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined.
- No `#[allow()]` directives.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| BREW-01 | Minor | Code Quality | `execute_update` ignores `_to_version` and `_release_info` because `brew upgrade` always goes to latest. This is correct behavior but should have a doc comment explaining the semantics. | `src/provider.rs:319-324` |
| BREW-02 | Minor | Code Quality | `execute_update` discards the exit code from `run_command_exec`: `let (cmd_output, _exit_code) = ...`. A non-zero exit code from `brew upgrade` (e.g., permission denied) should be checked and surfaced. | `src/provider.rs:358` |
| BREW-03 | Minor | Code Quality | `discover_software` and `refresh_package_index` create a `mpsc::channel(1)` and immediately drop the receiver. If the channel fills before the command completes, send attempts may block. Use a larger buffer or drain in a background task. | `src/provider.rs:167,188` |
| BREW-04 | Minor | Code Quality | `HomebrewConfig::validate()` returns `std::result::Result<(), String>` instead of the crate's own `Result` type. This is inconsistent with how GitHub and Docker config validation works. | `src/config.rs:30` |

## Verdict

**Pass.** Clean, lightweight provider with correct Homebrew integration. The discarded exit code (BREW-02) is the most impactful finding -- silent failures during `brew upgrade` could confuse users. The other findings are minor consistency improvements.
