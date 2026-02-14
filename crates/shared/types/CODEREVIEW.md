# Code Review: uptrakit-shared-types

## Summary

Shared value-type crate (~400 lines across 4 source files) providing `ProviderType` enum, `ReleaseAsset`/`ReleaseInfo` structs, `SecretString` wrapper for redacted secret handling, and zero-dependency `hex` encode/decode helpers. Only dependency is `serde`.

## Architecture

- **Module structure**: `lib.rs` re-exports from `provider_types.rs`, `secret_string.rs`, and exposes `pub mod hex`.
- **Public API surface**:
  - `ProviderType` enum (`GithubReleases`, `ProxmoxHelperScripts`, `DockerRegistry`, `Homebrew`)
  - `ReleaseAsset`, `ReleaseInfo` structs for update metadata
  - `SecretString` wrapper with redacted `Debug`/`Display`
  - `hex::encode()`, `hex::decode()`, `hex::DecodeError`
- **Dependency choices**: `serde` only (workspace). `serde_json` as dev-dependency for tests. Zero-dependency hex implementation replaces the external `hex` crate.
- **Layering**: Foundation crate used by `db` (crypto), `web-api-types`, `provider-core`, `wire`, and others.

## Security & Safety

- **SecretString redaction**: `Debug` outputs `"SecretString(***)"` (`src/secret_string.rs:33-35`), `Display` outputs `"***REDACTED***"` (`src/secret_string.rs:39-41`). Prevents accidental secret logging.
- **No `Zeroize` implementation**: `SecretString` wraps a standard `String` that remains in memory until dropped. The inner value is not zeroed on drop. The workspace declares `zeroize` as a dependency, but this crate does not use it.
- **Transparent serialization**: `SecretString` uses `#[serde(transparent)]` so JSON wire format is unchanged. This means the secret appears in plaintext in serialized output -- correct for wire protocol but consumers must ensure logging/tracing does not serialize `SecretString`-containing structs.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.
- `hex::encode()` uses `let _ = write!(...)` for infallible `String` writes.

## Code Quality

- **Error handling**: `hex::DecodeError` enum with `OddLength` and `InvalidChar` variants, implementing `Display` and `std::error::Error`. Clean and minimal.
- **Test coverage**:
  - `secret_string.rs`: 9 tests (debug redaction, display redaction, expose_secret, into_inner, serde roundtrip, Option serde, equality, hash consistency, clone).
  - `provider_types.rs`: 10 tests (serde roundtrip for all 4 variants, display, release asset roundtrip, optional field omission, release info roundtrip, empty assets omission).
  - `hex.rs`: 7 tests (encode empty, encode bytes, leading zeros, decode empty, decode valid, decode uppercase, decode odd length, decode invalid char, roundtrip).
  - Total: 26 tests -- comprehensive.
- **Consistency**: All public types derive appropriate traits (`Clone`, `Debug`, `PartialEq`, `Eq`, `Serialize`, `Deserialize`).

## Coding Standards Compliance

- No `rootcause`/`thiserror` needed at this layer (leaf types, no complex error chains).
- `hex::DecodeError` implements `std::error::Error` manually -- acceptable for a simple two-variant error.
- No `#[allow()]` directives.

## Extensibility Assessment

This crate is the foundation for external extensibility. Two critical issues limit third-party extension:

1. **`ProviderType` is a closed enum** with exactly four variants and no `#[non_exhaustive]`. An external developer building a new provider (e.g., APT, Flatpak) cannot add variants without forking. Adding a variant is also a semver-breaking change for downstream exhaustive matchers.

2. **`ProviderType` lacks `FromStr`**. Unlike many other enums in the codebase, `ProviderType` has `Display` but no `FromStr`. External consumers parsing provider types from user input or config files must manually match against serialized strings.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| TYP-01 | Info | Security | `SecretString` does not implement `Zeroize`. The inner `String` stays in memory until dropped. The workspace has `zeroize` as a dependency. Adding `#[derive(Zeroize, ZeroizeOnDrop)]` or a manual `Drop` impl would provide defense-in-depth against memory scraping. | `src/secret_string.rs:11-13` |
| TYP-02 | Info | Code Quality | `ProviderType::Display` implementation manually matches variants to snake_case strings. Using `strum::Display` with `#[strum(serialize_all = "snake_case")]` would reduce duplication with the serde `rename_all`, but the manual impl is correct and tested. | `src/provider_types.rs:15-24` |
| TYP-03 | Major | Extensibility | `ProviderType` is a closed enum without `#[non_exhaustive]`. External developers cannot add provider types without forking. Adding a variant is a semver-breaking change. At minimum add `#[non_exhaustive]`; consider a string-based newtype or `Other(String)` variant for full extensibility. | `src/provider_types.rs:7-13` |
| ~~TYP-04~~ | ~~Minor~~ | ~~Extensibility~~ | ~~`ProviderType` lacks `FromStr` implementation.~~ **FIXED.** `FromStr` implemented with typed `ParseProviderTypeError`, matching the `Permission`/`AgentStatus` pattern. | `src/provider_types.rs` |

## Verdict

**Pass.** Well-structured value-type crate with thorough test coverage. The `SecretString` redaction works correctly. The closed `ProviderType` enum (TYP-03) is the primary extensibility concern -- it is the root cause of the provider system being closed to external contributions. No action required for existing functionality, but TYP-03 should be addressed for external extensibility.
