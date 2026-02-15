# Code Review: uptrakit-cli

## Summary

Command-line interface binary crate (~600 lines across 7 source files) providing authentication (via device authorization flow), API token management, and a generic `api` subcommand for raw HTTP requests. Serves as proof that `web-api-types` is sufficient for building an independent API client.

## Architecture

- **Module structure**: `main.rs` (CLI dispatch), `client.rs` (HTTP client wrapper), `commands/auth.rs` (auth logic), `commands/api.rs` (raw API passthrough), `config.rs` (credential storage), `error.rs` (error types), `output.rs` (multi-format output).
- **Public API surface**: Binary only -- no library exports.
- **Dependency choices**: `uptrakit-web-api-types` (shared DTOs), `uptrakit-shared-macros` (error handling), `uptrakit-build-info` (version display), `clap` (CLI parsing), `reqwest` (HTTP), `serde_yaml_ng` (YAML output). Minimal and correct.
- **Layering**: Depends only on `web-api-types` and two build-support crates. Does NOT depend on `web-api`, `shared-db`, or any heavyweight crate. This is the ideal dependency pattern for an API client.

## Security and Safety

- **Credential storage**: `save_credentials()` sets Unix file permissions to `0o600`, preventing other users from reading stored API tokens.
- **Device authorization flow**: Correctly implements the device code grant with exponential backoff respecting `interval` and `expires_in`.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code (except `unwrap_or("")` on JSON field access in token commands).

## Code Quality

- **Error handling**: `CliError` enum with 7 variants. Uses `rootcause::Report` wrapper.
- **Multi-format output**: `print_output()` and `print_value()` support human-readable, JSON, and YAML formats.
- **Type-safe deserialization**: Auth status uses `web-api-types::auth::UserResponse` for compile-time contract validation.

## Coding Standards Compliance

- Typed error enum with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined.
- `impl_report_conversion!` used for cross-boundary errors.
- No `#[allow()]` directives.

## Extensibility Assessment

**The CLI proves that `web-api-types` is sufficient for building an independent API client.** An external developer building a TUI, mobile client, or automation tool can follow this exact pattern: depend on `web-api-types`, make HTTP requests, deserialize into shared types.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| ~~CLI-01~~ | ~~Minor~~ | ~~Code Quality~~ | ~~Token commands manually extract fields from `serde_json::Value`.~~ **FIXED.** `token_create()` and `token_list()` now use typed `CreateApiTokenResponse` and `ApiTokenListResponse` deserialization, matching the `UserResponse` pattern. | `src/commands/auth.rs` |
| ~~CLI-02~~ | ~~Minor~~ | ~~Code Quality~~ | ~~`chrono_date()` hand-rolls epoch-to-date calculation with `is_leap()`.~~ **FIXED.** Replaced with `time::OffsetDateTime::now_utc()`. `is_leap()` deleted. | `src/commands/auth.rs` |
| CLI-03 | Minor | Code Quality | `ApiClient::request()` returns `(u16, serde_json::Value)` tuple. A typed wrapper (e.g., `fn get<T: DeserializeOwned>(&self, path) -> Result<T>`) would reduce boilerplate at call sites. | `src/client.rs` |
| ~~CLI-04~~ | ~~Minor~~ | ~~Consistency~~ | ~~`config_dir()` hardcodes `$HOME/.config/uptrakit/`.~~ **FIXED.** Now uses `uptrakit-directories` crate with `AppDirs::resolve("cli", None, None)` for platform-appropriate paths. Credentials use `write_secure_file_str` (atomic 0o600). Manual `#[cfg(unix)]` permission block removed. | `src/config.rs` |
| CLI-05 | Info | UX | No `--quiet` flag for script-friendly usage. For automation scenarios, suppressing text output and relying on exit codes would improve integration. | `src/main.rs` |
| CLI-06 | Info | UX | No shell completion support. Adding `clap_complete` would improve developer experience with tab completion. | `src/main.rs` |

## Verdict

**Pass.** Clean, minimal API client with correct dependency layering. The CLI successfully demonstrates that external developers can build independent clients using only `web-api-types`. The hand-rolled date formatting (CLI-02) and untyped JSON extraction (CLI-01) are the most actionable findings.
