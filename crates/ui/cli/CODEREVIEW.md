# Code Review: uptrakit-cli

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-cli` is the operator-facing CLI binary. It depends on `uptrakit-openapi-client`
(not `uptrakit-web-api`) for a clean dependency boundary. All eleven command namespaces are
implemented with typed returns via the `HumanOutput` trait. The crate demonstrates good design
decisions: `SecretString` at credential boundaries, URL scheme validation before browser open,
`lib` + `bin` split for testability, and comprehensive CLI argument tests.

The main concerns are the plaintext API token storage, `main.rs` at 3,870 lines (including
~1,500 lines of tests) needing splitting, and incomplete integration test coverage (only 3 of
12 namespaces covered by `MockApiServer` tests).

## Architecture

### Strengths

- `Cargo.toml:29-34` -- Depends on `openapi-client`, not `web-api`. Clean layer boundary.
- Typed command returns with `HumanOutput` trait. `main.rs` dispatches uniformly via
  `output::print_output(format, &resp)`. Trait design documented at `output.rs:7-15`
  including deliberate omission of `#[non_exhaustive]` from `OutputFormat`.
- `src/lib.rs` -- `lib` + `bin` split. The lib target re-exports command modules so `tests/`
  can import them without going through `main`.
- `src/client.rs` -- `authenticated_client` centralizes credential resolution and client
  construction. Every command calls it as its first step.
- Param structs (`*Params<'a>`) for complex arities keep call sites readable (e.g.
  `hosts::ListParams`, `scheduler::TriggerParams`).
- `SecretString` at credential boundaries. OIDC `client_secret` and MQTT `password`/`ca_pem`
  wrapped in `SecretString::new(...)` before API client calls (`settings.rs:576-577`,
  `settings.rs:695`). Device-flow token stored via `.expose_secret()` only at disk write
  point (`auth.rs:250`).
- `src/commands/auth.rs:393-403` -- URL scheme validation before browser open. Rejects
  `file://`, `javascript:`, `data:`, and bare paths. URL still printed to stderr for manual
  opening.
- `main.rs:866-868` -- `--insecure` flag emits visible `WARNING:` to stderr whenever TLS
  verification disabled.
- `src/error.rs:43` -- `serde_yaml_ng` error wrapped via `impl_report_conversion!` rather than
  leaking third-party error type.

### Issues

**[LOW]** `src/commands/services.rs` and `src/commands/settings.rs` -- Inconsistent
parameter-passing: some commands use `*Params<'a>` structs, others use naked positional
arguments. No documented rationale for the split.

**[LOW]** `src/main.rs` (dispatch section, ~834-1300 LoC) -- Monolithic dispatch block. The
`run()` function's `match command { ... }` handles all eleven namespaces in a single function.
Splitting into per-namespace dispatch helpers would reduce navigation complexity.

## Security and Safety

### Strengths

- `src/commands/auth.rs:393-403` -- URL scheme validation before `open`/`xdg-open`/`start`.
  Rejects dangerous schemes (`file://`, `javascript:`, `data:`, `ftp://`). Eight tests cover
  all scheme categories (`auth.rs:581-606`).
- API token written to disk via `write_secure_file_str` (0o600 permissions) and never held
  in a heap variable that outlives the write path (`auth.rs:244-253`).
- Zero `unsafe` blocks.
- No credential values in `Debug` output. `Config` and `Credentials` derive `Debug` but
  neither holds a live secret at runtime.

### Issues

**[HIGH]** `src/config.rs:43-50` -- API token stored in plaintext JSON in state directory.
On shared workstations or CI pipelines, the token grants full API access with no expiry. The
`0o600` permissions mitigate Unix multi-user scenarios but not the running user's own processes
or root.

**[MEDIUM]** `src/commands/settings.rs:596-598` -- MQTT password and CA PEM passed as plain
`serde_json::Value::String` in `mqtt_update`, bypassing `SecretString` wrapping used in
`mqtt_create` (line 576-577). Inconsistent secret handling.

**[LOW]** `src/commands/api.rs:83-84` -- `StatusCode.as_u16()` in serialization helper outside
an approved site. Function lacks inline comment justifying the exception.

## Code Quality

### Strengths

- `HumanOutput` implementations consistent and complete for all response types. Uniform
  pattern: empty-list guard returns early with "No X found.", non-empty renders header + data.
- `CliError` comprehensive with seven variants covering all failure modes, mapped via
  `impl_report_conversion!`. `ClientError` -> `CliError` mapping handles `RateLimited`,
  `NotFound`, `NotAuthenticated`, `InvalidMethod` explicitly.
- `src/commands/check.rs:43-69` -- `check::all` uses scheduler task list rather than a
  dedicated endpoint, avoiding coupling to a redundant API endpoint.
- `main.rs:820-832` -- `resolve_ca_pem` handles mutual exclusion cleanly with exhaustive
  `match`.
- `main.rs:850-863` -- `tracing_subscriber` initialization is conditional (only with `-v`),
  routes to stderr to avoid contaminating stdout command output.
- Integration tests in `tests/command_execution.rs` use `MockApiServer` with typed endpoint
  builders. Mock behind `[dev-dependencies]` feature gate.
- Error-path coverage for HTTP error codes: 401 Unauthorized, 429 Too Many Requests, 500
  Internal Server Error, 404 Not Found. The 401 -> `CliError::NotLoggedIn` mapping asserted.
- Unit tests co-located with implementations. Every command file has `#[cfg(test)]` module
  testing `HumanOutput` implementations.
- `validate_url_scheme` exhaustively tested. Eight unit tests cover every scheme category.
- `chrono_date` format validated structurally (`auth.rs:564-578`).
- 84 unit tests covering all CLI subcommand parsing variations.

### Issues

**[HIGH]** `src/main.rs` -- At 3,870 lines (including ~1,500 lines of tests), this is the
largest single file. Command enum definitions (~900 lines), main dispatch, and tests should
be split into separate modules.

**[MEDIUM]** `tests/command_execution.rs` -- Only `hosts`, `services`, and `software_items`
namespaces covered by integration tests. Nine of twelve command namespaces have zero integration
coverage against `MockApiServer`: `auth`, `scheduler`, `settings`, `plugin_configs`,
`autodiscovery`, `check`, `update`, `history`, and `api`. The `check::all` two-step interaction
(list tasks, trigger by ID) would benefit particularly from a mock test.

**[MEDIUM]** `tests/command_execution.rs:195-210` -- `hosts_list_json_format` test does not
assert JSON output. Asserts only `result.is_ok()`. The test is a duplicate of
`hosts_list_success` under a misleading name.

**[MEDIUM]** `src/commands/check.rs:53` -- Task type matched by raw string literal
`"version_check"`. Should use a typed constant.

**[MEDIUM]** `src/commands/update.rs:50` -- `unwrap_or("")` on `release_url` sends empty string
to server when only `--release-tag` provided. Server must distinguish "provided and empty" from
"not provided".

**[LOW]** `src/commands/auth.rs:155-156` -- `chrono_date()` function name does not match its
implementation (uses `time::OffsetDateTime`, not `chrono`).

**[LOW]** `src/commands/auth.rs` -- `login` async function (130 lines, device-flow polling,
rate-limit backoff, timeout handling, file I/O, browser launch) has zero test coverage. The
polling loop logic should be extracted into a testable `poll_until_authorized` function.

**[LOW]** `src/output.rs:84` -- `unwrap_or_else` fallback in `print_value` for human format
silently swallows JSON error. `context_to()?` would be consistent.

**[LOW]** `src/commands/plugin_configs.rs:48` -- Raw `self.config` printed without
pretty-printing. Uses compact JSON inline instead of `serde_json::to_string_pretty`.

**[LOW]** `HumanOutput` unit tests do not verify `--output yaml` path. No test calls
`print_output(OutputFormat::Yaml, &val)`. A `serde_yaml_ng` regression would not be caught.

## High Availability

### Strengths

N/A -- Short-lived CLI process. No persistent connections or background tasks.

### Issues

N/A.

## Coding Standards

### Strengths

- `edition = "2024"` with workspace field inheritance.
- `Result<T>` alias and `bail!` / `report!` patterns used uniformly.
- Zero `#[allow(clippy::...)]` suppressions.
- No `StatusCode` numeric literals.
- `--output` default documented and tested (`default_format_is_human`).

### Issues

**[LOW]** `Cargo.toml` -- No `publish = false` declaration. Should not be accidentally
published to crates.io.

**[LOW]** `src/commands/settings.rs:701` -- `role_mapping: HashMap::new()` hardcoded in
`oidc_create`. No `--role-mapping` CLI argument.

## Extensibility

### Strengths

- Adding a new command namespace requires four localized changes: module, pub mod, enum variant,
  dispatch arm.
- `HumanOutput` is open for extension. Three output formats (Human/JSON/YAML) require no
  per-command work beyond `Serialize + HumanOutput`.
- `CliError::Other(String)` catch-all allows surfacing domain-specific errors without new
  variants.

### Issues

**[MEDIUM]** `src/output.rs:57-72` -- `print_output` prints to stdout directly with no way
for callers to capture output. Accepting `&mut dyn Write` would enable test assertions.

**[LOW]** `src/commands/check.rs:43-69` -- `check::all` makes two sequential API calls where
one would suffice with a dedicated endpoint. TOCTOU window between list and trigger.
