# CODEREVIEW — uptrakit-cli

## Summary

`uptrakit-cli` is the operator-facing command-line interface for Uptrakit. It compiles to a single
`uptrakit` binary and depends on `uptrakit-openapi-client` (the typed HTTP client generated from
the OpenAPI spec) rather than `uptrakit-web-api` directly — a correct and clean boundary. All
eleven command namespaces (`auth`, `hosts`, `services`, `software_items`, `check`, `update`,
`history`, `scheduler`, `settings`, `plugin_configs`, `autodiscovery`) are implemented and wired
through `main.rs`.

The crate introduced two notable design decisions in the `refactor(cli)` commit: the `HumanOutput`
trait (typed rendering for the `--output human` format) and typed command-function returns (every
command handler returns a concrete `Result<SomeOutputType>` rather than printing directly). Both
decisions are well-executed. The primary concerns are a missing workspace pin for `serde_yaml_ng`,
an inconsistent parameter-passing style across command functions, incomplete integration-test
coverage for most command namespaces, one credential-persistence risk in the login flow, and a
small number of code-quality issues in `main.rs` that add friction without benefit.

---

## Architecture

### Strengths

- **Correct dependency boundary.** The CLI depends on `uptrakit-openapi-client`, not on
  `uptrakit-web-api`. This keeps the CLI portable and avoids dragging in Axum, SeaORM, and the
  full server dependency graph.

- **Typed command returns with `HumanOutput`.** Command functions return `Result<T>` where `T`
  implements both `Serialize` and `HumanOutput`. `main.rs` dispatches to
  `output::print_output(format, &resp)` uniformly. The trait design is documented correctly
  (`output.rs:7-15`) including the deliberate decision to omit `#[non_exhaustive]` from
  `OutputFormat`.

- **`lib` + `bin` split.** The crate exposes a `lib` target (`src/lib.rs`) whose sole purpose is
  to re-export command modules so that `tests/` can import them without going through `main`. This
  avoids the common anti-pattern of duplicating logic between the binary entry point and test
  helpers.

- **`authenticated_client` helper.** `src/client.rs` centralises credential resolution and client
  construction in a single two-function module. Every command calls `authenticated_client(...)` as
  its first step, making the authentication path easy to audit.

- **Param structs for complex arities.** Most command functions accept a dedicated `*Params<'a>`
  struct rather than a long bare argument list. This keeps call sites readable and avoids
  positional confusion (e.g. `hosts::ListParams`, `scheduler::TriggerParams`).

- **`SecretString` at credential boundaries.** OIDC `client_secret` and MQTT `password`/`ca_pem`
  are wrapped in `SecretString::new(...)` before being passed to the API client
  (`settings.rs:576-577`, `settings.rs:695`). The device-flow token is stored via
  `.expose_secret()` only at the point it is written to disk (`auth.rs:250`).

- **URL scheme validation before browser open.** `validate_url_scheme` in `auth.rs:393-403`
  rejects `file://`, `javascript:`, `data:`, and bare paths before the CLI opens the
  verification URL in the user's browser. This is a meaningful defense against a compromised
  server returning a malicious redirect URL.

- **`--insecure` flag warning.** `main.rs:866-868` emits a visible `WARNING:` to stderr whenever
  TLS verification is disabled.

- **`serde_yaml_ng` wrapped properly.** The YAML serialization error type (`serde_yaml_ng::Error`)
  is converted to the internal `CliError::Yaml(String)` variant via
  `impl_report_conversion!` rather than leaking the third-party error type into the public API
  (`error.rs:43`).

### Issues

**[SEVERITY: Low]** `crates/ui/cli/src/commands/services.rs` and several others — inconsistent
parameter-passing style between command functions
Commands in `hosts.rs`, `scheduler.rs`, `software_items.rs`, `update.rs`, and `check.rs` all
accept a `*Params<'a>` struct. Commands in `services.rs` (`approve`, `reject`, `remove`, `show`,
`update`, `merge`) and `settings.rs` (`mqtt_show`, `mqtt_delete`, `oidc_show`, etc.) accept
naked positional arguments (`id: &Uuid, server: Option<&str>, token: Option<&str>, ...`). The two
styles coexist without a documented rationale. The positional style becomes fragile at four or more
arguments and is inconsistent with the established `*Params` pattern.

**[SEVERITY: Low]** `crates/ui/cli/src/main.rs` (dispatch section, ~834-1300 LoC) — monolithic
dispatch block
The `run()` function's `match command { ... }` arm handles all eleven top-level namespaces and all
their subcommands in a single function. While each arm is shallow (construct params, call command
function, call `print_output`), the function is long enough that adding a new top-level command
requires navigating hundreds of lines. Splitting into per-namespace dispatch helpers (e.g.
`dispatch_hosts`, `dispatch_settings`) would reduce the file from ~1300 lines to a navigable entry
point without changing any behaviour.

---

## Security & Safety

### Strengths

- **No `unsafe` code.** Zero `unsafe` blocks in the CLI crate.

- **URL scheme validation before `open`/`xdg-open`/`start`.** `auth.rs:393-403` validates the
  scheme is `https://` (or `http://` only when `--insecure` is active) before spawning the OS
  browser command. Dangerous schemes (`file://`, `javascript:`, `data:`, `ftp://`) are rejected
  with an error message, and the URL is still printed to stderr so the user can open it manually.

- **API token exposed only once.** After a successful device-flow login, the plaintext token is
  written to disk via `write_secure_file_str` (which sets `0o600` permissions) and never held in a
  heap variable that outlives the write path (`auth.rs:244-253`).

- **`validate_url_scheme` fully unit-tested.** Eight tests in `auth.rs:581-606` cover: `https`
  allowed, `http` blocked without `--insecure`, `http` allowed with `--insecure`, `file://`
  blocked regardless, `javascript:` blocked, `ftp://` blocked, `data:` blocked, empty/relative
  paths blocked.

- **No credential values in `Debug` output.** `Config` and `Credentials` derive `Debug` but
  neither holds a live secret at runtime; the token is stored only on disk and loaded transiently.

### Issues

**[SEVERITY: High]** `crates/ui/cli/src/config.rs:43-50` — API token stored in plaintext JSON in
state directory
`credentials.json` stores `{ "token": "..." }` as unencrypted JSON. On a shared workstation or
in the presence of a malicious process with filesystem read access (common in CI pipelines), the
stored token grants full API access with no expiry (API tokens have no `expires_at` column per the
database review findings). The token should be stored encrypted at rest or, at minimum, the
documentation should prominently note the risk. The `write_secure_file_str` helper correctly sets
`0o600`, which mitigates Unix multi-user scenarios, but provides no protection against the running
user's own processes or root.

**[SEVERITY: Medium]** `crates/ui/cli/src/commands/auth.rs:155-156` — `chrono_date()` function
name does not match its implementation
`chrono_date()` uses `time::OffsetDateTime::now_utc()` — not the `chrono` crate — to format the
current date. The name is a misleading artifact: the function was likely written when `chrono` was
under consideration, and the name was not updated. This creates confusion for readers who expect
`chrono_date()` to interact with the `chrono` dependency. The `chrono` crate is not in the CLI's
`Cargo.toml`; the function should be renamed (e.g. `format_current_date()`) to match its
implementation.

**[SEVERITY: Medium]** `crates/ui/cli/src/commands/settings.rs:596-598` — MQTT password and
CA PEM passed as plain `serde_json::Value::String` in `mqtt_update`
In `mqtt_create` (line 576-577), `password` and `ca_pem` are wrapped in `SecretString::new(...)`.
In `mqtt_update` (lines 596-598), the same fields are converted to `serde_json::Value::String`
directly, bypassing `SecretString`. This is driven by a nullable-patch-update API design (the
server differentiates "not provided" from "set to null" via JSON `Value`), but the inconsistency
means the secret is not redacted in `Debug` formatting for the update path.

**[SEVERITY: Low]** `crates/ui/cli/src/commands/api.rs:83-84` — `StatusCode.as_u16()` in
serialization helper outside an approved site
`serialize_status_code` calls `status.as_u16()`. Per the coding standards review, `as_u16()` is
approved only at serialization sites, and this qualifies — but the function lacks the inline
comment that the standards review identified as necessary to justify the exception
(`openapi-client/src/mock.rs:221` was flagged for the same omission). A one-line comment
`// Approved use: serialization of HTTP status for API response envelope` would satisfy the
standard.

---

## Code Quality

### Strengths

- **`HumanOutput` implementations are consistent and complete.** Every public response type
  returned by a command handler has a `HumanOutput` implementation that renders a human-readable
  table or detail view. Implementations follow a uniform pattern: empty-list guard returns early
  with a "No X found." message, non-empty renders a fixed-width header row followed by data rows.

- **`CliError` is comprehensive and well-mapped.** `error.rs` defines seven variants covering all
  failure modes (HTTP, IO, JSON, API, NotLoggedIn, Directory, YAML, Other) and maps from all
  upstream error types via `impl_report_conversion!`. The `ClientError` → `CliError` mapping is
  complete and handles the `RateLimited`, `NotFound`, `NotAuthenticated`, and `InvalidMethod`
  variants explicitly rather than falling through to a catch-all.

- **`check::all` uses the scheduler task list.** Rather than adding a dedicated "trigger all
  version checks" API endpoint, `check::all` (`check.rs:43-69`) looks up the `version_check`
  scheduled task by type and triggers it via the general scheduler API. This avoids coupling the
  CLI to a redundant endpoint.

- **`resolve_ca_pem` handles mutual exclusion cleanly.** The helper (`main.rs:820-832`) merges
  `--ca-pem` (inline string) and `--ca-pem-file` (file path) into a single `Option<String>`.
  Clap's `conflicts_with` ensures only one is provided; the helper uses an exhaustive `match`
  rather than an `if/else` chain, making extension to a third source straightforward.

- **`tracing_subscriber` initialisation is conditional.** Unlike `uptrakit-service-sdk`
  (which unconditionally initialises the subscriber from a library — a workspace-level Medium
  issue), `main.rs:850-863` only calls `tracing_subscriber::fmt().init()` when `-v` is passed,
  and routes log output to stderr so it does not contaminate stdout command output.

### Issues

**[SEVERITY: Medium]** `crates/ui/cli/src/commands/check.rs:53` — task type matched by raw
string literal `"version_check"`
`tasks.iter().find(|t| t.task_type == "version_check")` compares against a hardcoded string.
If the server-side task type identifier is ever renamed, this silently stops finding the task and
returns a non-error `TriggerScheduledTaskResponse { triggered: false }`. The task type should
use a typed constant or the `SchedulerTaskType` enum from `uptrakit-shared-types` if it is
exported, or at minimum the string should be defined as a named constant in this module.

**[SEVERITY: Medium]** `crates/ui/cli/src/commands/update.rs:50` — `unwrap_or("")` on
`release_url` produces a structurally valid but semantically empty field
`release_url: params.release_url.unwrap_or("").to_string()` in `ReleaseInfoRequest` construction
sends an empty string to the server when `--release-url` is not provided but `--release-tag` is.
The server must then distinguish "provided and empty" from "not provided". The surrounding `if`
condition (line 47) checks `params.release_tag.is_some() || params.release_url.is_some()`, so an
empty `release_url` is sent when only `--release-tag` is given. This may cause unexpected server
behaviour if the server validates the field. The field should remain `None` (i.e. use
`params.release_url.map(|s| s.to_string()).unwrap_or_default()` only when both are absent from
the struct, or restructure the condition).

**[SEVERITY: Low]** `crates/ui/cli/src/output.rs:84` — `unwrap_or_else` fallback in
`print_value` for human format silently swallows the JSON error
`serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())` — `serde_json::Value`
serialisation to `String` is effectively infallible for a value already in memory, but the silent
fallback discards the error. Consistent with the rest of the crate, `context_to()?` should be used
instead.

**[SEVERITY: Low]** `crates/ui/cli/src/commands/plugin_configs.rs:48` — raw `self.config`
printed without pretty-printing
`PluginConfigResponse.config` is a `serde_json::Value`. The `HumanOutput` implementation
formats it as `format!("Config: {}\n", self.config)` which produces compact JSON inline. For
operator readability, it should use `serde_json::to_string_pretty(&self.config)`.

---

## Tests

### Strengths

- **`MockApiServer` used correctly.** Integration tests in `tests/command_execution.rs` use the
  `uptrakit-openapi-client` mock feature's `MockApiServer` with typed endpoint builders
  (`server.hosts().on_list().ok(...)`, `server.hosts().on_get(&id).not_found(...)`). The mock is
  behind a `[dev-dependencies]` feature gate and does not enter production builds.

- **Error-path coverage for HTTP error codes.** The integration test file covers the four most
  important error scenarios against the live mock: 401 Unauthorized (`api_401_returns_not_authenticated`),
  429 Too Many Requests (`api_429_returns_rate_limited`), 500 Internal Server Error
  (`api_500_returns_server_error`), and 404 Not Found (`hosts_show_not_found`,
  `services_approve_not_found`). The 401 → `CliError::NotLoggedIn` mapping is asserted directly.

- **Unit tests co-located with implementations.** Every command file has a `#[cfg(test)]` module
  testing its `HumanOutput` implementations (e.g. `auth.rs:424-617`, `hosts.rs:200-298`,
  `scheduler.rs:152-220`, `settings.rs:782-917`). These tests verify key field presence, empty
  list behaviour, and serialisation round-trips without requiring a live server.

- **`validate_url_scheme` exhaustively tested.** Eight unit tests in `auth.rs` cover every
  scheme category. This is the security-sensitive function in the CLI and the test coverage is
  proportionate to the risk.

- **`chrono_date` format validated structurally.** `auth.rs:564-578` asserts that the date string
  is exactly 10 characters, has three dash-separated parts, and all parts are numeric digits —
  catching format drift without hardcoding a specific date.

### Issues

**[SEVERITY: Medium]** `crates/ui/cli/tests/command_execution.rs` — only `hosts`, `services`,
and `software_items` namespaces are covered by integration tests
Nine of the twelve command namespaces have zero integration-level test coverage against
`MockApiServer`: `auth` (login, token create/list/revoke, status), `scheduler` (list, show,
trigger), `settings` (all subcommands), `plugin_configs` (list, show, create, update, delete,
discover), `autodiscovery` (ignores list/create/delete), `check` (all, item), `update` (trigger),
`history` (list, show), and `api` (raw request). The `check::all` function has a non-trivial
two-step interaction (list tasks, then trigger by ID) that would benefit particularly from a
mock test verifying the correct task ID is passed to the trigger endpoint.

**[SEVERITY: Medium]** `crates/ui/cli/tests/command_execution.rs:195-210` — `hosts_list_json_format`
test does not assert JSON output
`hosts_list_json_format` calls `hosts::list(...)` and asserts `result.is_ok()`. The test name
implies it verifies JSON output format, but since `hosts::list` returns a typed `Result<PaginatedResponse<HostResponse>>` rather than a formatted string, the format is not exercised.
The test is a duplicate of `hosts_list_success` under a misleading name. It should either be
removed or rewritten to call `output::print_output(OutputFormat::Json, &resp)` and verify the
resulting JSON string contains expected fields.

**[SEVERITY: Low]** `crates/ui/cli/src/commands/auth.rs` — `login` async function has zero test
coverage
`auth::login` is the most complex function in the crate (130 lines, device-flow polling loop,
rate-limit backoff, timeout handling, file I/O, browser launch). It cannot be tested with the
current `MockApiServer` setup because it also performs interactive stdin prompts and spawns an OS
browser process. The polling loop logic (`RateLimited` backoff, `Expired` bailout, `Authorized`
token extraction) should be extracted into a testable `poll_until_authorized` function that
accepts the client and a `max_attempts` parameter, enabling unit testing without stdin/browser
side-effects.

**[SEVERITY: Low]** `crates/ui/cli/src/commands/` — `HumanOutput` unit tests do not verify
the `--output yaml` path
All `#[cfg(test)]` modules test `to_human_string()` and JSON serialisation. No test calls
`output::print_output(OutputFormat::Yaml, &val)` or asserts YAML output structure. Since
`serde_yaml_ng` is a non-workspace dependency, a regression in its output format (e.g. changed
quoting rules, key ordering) would not be caught until a user reports it.

#### 2026-02-24 Review

##### Strengths

- **CLI error-handling tests cover 401, 429, and 500 status codes with message assertions.** `tests/command_execution.rs:298-374` — Three tests verify HTTP error status codes translate correctly.

##### Issues

---

## High Availability

### Strengths

N/A — The CLI is a short-lived process with no persistent connections, no background tasks, and no
state beyond the on-disk config and credentials files. High availability concerns do not apply.

### Issues

N/A — See above.

---

## Database

### Strengths

N/A — The CLI binary has no direct database access. All data operations are performed through the
`uptrakit-openapi-client` HTTP client against the web-api server.

### Issues

N/A — See above.

---

## Coding Standards

### Strengths

- **`edition = "2024"` and workspace field inheritance.** `Cargo.toml` inherits `license`,
  `authors`, `repository`, and `version` from the workspace. `edition = "2024"` matches every
  other crate in the workspace.

- **`Result<T>` alias used consistently.** `error.rs:34` defines `pub type Result<T> = std::result::Result<T, Report<CliError>>`. Every command function and helper returns `Result<T>`, never `Result<T, String>` or a bare `Box<dyn Error>`.

- **`bail!` / `report!` patterns used uniformly.** Guard returns throughout the command files use
  `bail!(CliError::Other("...".into()))` and inline constructions use `report!(...)`. No
  `Report::new()` anti-pattern is present.

- **No `#[allow(clippy::...)]` suppressions.** The crate contains zero `#[allow(clippy::...)`
  annotations, consistent with the workspace-wide one-suppression limit documented in
  `AGENTS.md`.

- **No `StatusCode` numeric literals.** `api.rs:68` uses `resp.status.is_client_error()` and
  `.is_server_error()`, and `error.rs:17,58` uses `StatusCode::TOO_MANY_REQUESTS` and
  `StatusCode::NOT_FOUND`. No raw integer comparisons.

- **`--output` default documented and tested.** `output.rs:19-20` sets `OutputFormat::Human` as
  the default via `#[default]` and `default_value_t`. The unit test `default_format_is_human`
  asserts this directly.

### Issues

**[SEVERITY: Medium]** `crates/ui/cli/src/main.rs:862` — `expect()` in `run()` on a tracing
filter directive
`level.parse().expect("valid level directive")` in the verbose logging setup. This call is in the
binary entry-point `run()` function, not in a library. The input (`"warn"`, `"info"`, `"debug"`,
`"trace"`) is a static string literal that cannot fail to parse, so the `expect()` is functionally
safe — but it violates the AGENTS.md rule that `expect()` is approved only for `Mutex`/`RwLock`
poison guards. The expression should use `.unwrap()` with a comment, or be restructured to use
`EnvFilter::new(level)` which accepts a `&str` without parsing.

**[SEVERITY: Low]** `crates/ui/cli/Cargo.toml` — no `publish = false` declaration
All other internal crates that are not intended for publication to crates.io should declare
`publish = false`. `uptrakit-cli` is a first-party binary and should not be accidentally
published. `crates/ui/web-api/Cargo.toml` similarly lacks this guard (noted in the architecture
review for `uptrakit-web-api-types`).

**[SEVERITY: Low]** `crates/ui/cli/src/commands/settings.rs:701` — `role_mapping: HashMap::new()`
hardcoded in `oidc_create`
The CLI `settings oidc create` command always sends an empty `role_mapping` because there is no
`--role-mapping` argument. A user who wants to create an OIDC provider with pre-configured role
mappings must use `settings oidc update` in a second step. This is a UX limitation, not a
correctness issue, but the API surface supports role mapping at creation time and the CLI silently
ignores it. Either the argument should be added or a doc-comment on the function should note the
limitation.

---

## Extensibility

### Strengths

- **Adding a new command namespace requires four localised changes.** (1) Add a module to
  `src/commands/` implementing functions with typed returns. (2) Add a `pub mod` line to
  `src/commands/mod.rs`. (3) Add a variant to the appropriate `*Commands` enum in `main.rs`. (4)
  Add a dispatch arm to `run()`. No changes are required to `output.rs`, `error.rs`, or
  `client.rs`.

- **`HumanOutput` is open for extension.** New response types from the openapi-client can
  implement `HumanOutput` in the command module that uses them, without modifying the trait
  definition. The trait has no blanket implementations that would conflict.

- **`CliError::Other(String)` catch-all.** The `Other` variant in `error.rs:30-31` allows command
  authors to surface domain-specific error messages without requiring a new variant. Combined with
  `impl_report_conversion!`, mapping from new upstream error types is a one-liner.

- **Three output formats require no per-command work.** Any type that implements `Serialize +
  HumanOutput` gets JSON and YAML output for free via `print_output`. New commands only need to
  implement `HumanOutput`; `serde(Serialize)` is already required by the typed return design.

### Issues

**[SEVERITY: Medium]** `crates/ui/cli/src/output.rs:57-72` — `print_output` prints to stdout
without giving callers visibility into what was printed
`print_output` calls `print!(...)` / `println!(...)` directly and returns `Result<()>`. There is
no way for a caller or test to capture or redirect the output without redirecting stdout at the OS
level. This makes testing the formatting path (as noted in the Tests section) structurally
difficult: integration tests can only assert `result.is_ok()`, not the actual output content.
Refactoring `print_output` to accept a `&mut dyn Write` parameter (with `io::stdout()` as the
default in `main.rs`) would allow tests to pass a `Vec<u8>` and assert content, without changing
any production behaviour.

**[SEVERITY: Low]** `crates/ui/cli/src/commands/check.rs:43-69` — `check::all` makes two
sequential API calls where one would suffice if a dedicated endpoint existed
The function lists all scheduled tasks and then triggers the `version_check` task. This introduces
a TOCTOU window: if the task is deleted or renamed between the list and the trigger, the command
silently reports `triggered: false`. A dedicated `POST /api/v1/scheduler/version-check/trigger`
endpoint would eliminate the two-hop lookup. This is an API design note for the server, not
something fixable solely in the CLI.

#### 2026-02-24 Review

##### Strengths

- **`OutputFormat` explicitly documents its intentional omission of `#[non_exhaustive]`.** `src/output.rs:7-17` — Detailed doc comment explains why. Same documentation approach should be applied elsewhere.
