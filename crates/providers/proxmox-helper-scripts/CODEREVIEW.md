# Code Review: uptrakit-provider-proxmox-helper-scripts

## Summary

Proxmox VE helper scripts provider crate (~150 lines across 2 source files) implementing the `Provider` trait for executing Proxmox helper script updates via `curl | bash`. Simplest provider in the system -- zero-config unit struct with `Default`.

## Architecture

- **Module structure**: `lib.rs` re-exports from `provider.rs`.
- **Public API surface**: `ProxmoxHelperScriptsProvider`.
- **Dependency choices**: `uptrakit-provider-core` (sole uptrakit dependency), `rootcause`, `async-trait`, `tokio`, `serde_json` -- minimal.
- **Layering**: Leaf provider crate. Depends only on `uptrakit-provider-core`.

## Security and Safety

- **Injection-safe script execution**: The `execute_update` method passes `script_url` as a positional argument (`$1`) to bash rather than interpolating it into the command string. The command uses `set -euo pipefail` with `curl -fsSL -- "$1" | bash -s -- --update`.
- **`curl | bash` risk**: The pattern itself runs arbitrary remote code. This is inherent to the Proxmox helper scripts ecosystem and should be documented as a security consideration.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: Uses `ProviderError` directly (no crate-specific error module).
- **Test coverage**: Minimal -- relies on integration testing at the registry level.

## Coding Standards Compliance

- Uses provider-core error types directly -- acceptable for a minimal provider.
- No `#[allow()]` directives.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| PHS-01 | Major | Correctness | Claims `DiscoverLocalSoftware` capability but does not implement `discover_software()`. The default implementation returns an error. Callers checking `has_capability(DiscoverLocalSoftware)` would expect `discover_software()` to succeed, violating the capability contract. Either implement discovery or remove the capability. | `src/provider.rs:30` |
| ~~PHS-02~~ | ~~Minor~~ | ~~Code Quality~~ | ~~No configuration type or validation.~~ **FIXED.** `ProxmoxHelperScriptsConfig` struct created with `script_url: String` and `validate()` method. Provider now requires config at construction time. Registry deserializes and validates config before creating provider. | `src/config.rs`, `src/provider.rs` |
| PHS-03 | Info | Security | The `curl | bash` execution pattern runs arbitrary remote code. While the quoting prevents injection, the pattern itself is inherently risky. Document the trust model: the user must trust the script URL source. | `src/provider.rs` |

## Verdict

**Conditional pass.** The capability contract violation (PHS-01) is a real correctness issue -- callers that trust `has_capability()` will encounter runtime errors. This should be fixed before relying on capability-based dispatch. The missing config validation (PHS-02) is a secondary concern.
