# Code Review: `uptrakit-cli` Crate

**Reviewer:** Claude Opus 4.6
**Date:** 2026-02-17
**Scope:** Full review of `crates/ui/cli/` - architecture, security & safety, code quality,
high availability, and coding standards compliance.
**Crate version:** workspace (edition 2024)
**Files reviewed:** 17 Rust source files + `Cargo.toml` (build.rs, main.rs, client.rs,
config.rs, error.rs, output.rs, commands/{mod,api,auth,check,history,hosts,scheduler,services,settings,software_items,update}.rs)

---

## Summary

The `uptrakit-cli` crate is a well-structured CLI application built on `clap` that provides
comprehensive coverage of the controller's REST API via the `uptrakit-openapi-client`. The
codebase is clean, follows project error-handling conventions, and has good test coverage for
argument parsing. However, there are security concerns around credential storage, several
code quality inconsistencies, and a semantic bug in the `check` command.

### Severity Legend

| Severity | Meaning |
| --- | --- |
| **CRITICAL** | Security vulnerability or data loss risk; must fix before merge |
| **HIGH** | Significant bug or design flaw; should fix before merge |
| **MEDIUM** | Code quality, consistency, or correctness issue; fix soon |
| **LOW** | Minor improvement or style inconsistency; fix at convenience |
| **INFO** | Observation or suggestion; no action required |

---

## Extensibility Review

### Clean dependency chain

The CLI depends only on `uptrakit-openapi-client`, `uptrakit-build-info`, and
`uptrakit-directories`. No server, database, wire, or provider dependencies.

### Extensibility positives

- **Demonstrates the intended external developer experience** -- if the CLI can do everything
  through `openapi-client`, so can any external application.
- **Uses `openapi-client::types::*`** for all request/response types, validating the re-export
  strategy.
- **Device flow authentication** demonstrates the headless auth pattern for external tools.
- Serves as a living integration test for the `openapi-client` API surface.

---

## 1. Security & Safety

### S-1: Credentials stored in config directory instead of state directory [CRITICAL]

**File:** `src/config.rs:41-54`

Both `load_credentials()` and `save_credentials()` store `credentials.json` under
`dirs.config_path()`. Per the `uptrakit-directories` documentation and the project's own
directory management guidelines (AGENTS.md "Config vs state separation" table), secrets
(tokens, private keys) belong in the **state** directory, not config.

Config directories are conceptually "shareable" - they may be synced across machines, backed
up to cloud storage, or included in dotfile repositories. Storing API tokens there increases
the risk of accidental credential exposure.

**Current:**

```rust
pub fn load_credentials() -> Result<Credentials> {
    let dirs = app_dirs()?;
    let path = dirs.config_path("credentials.json"); // WRONG: secret in config dir
    // ...
}
```

**Should be:**

```rust
pub fn load_credentials() -> Result<Credentials> {
    let dirs = app_dirs()?;
    let path = dirs.state_path("credentials.json"); // secrets belong in state dir
    // ...
}
```

Both `load_credentials` and `save_credentials` need this change. The config directory
(`config.json` with server URL) is correctly placed.

---

### S-2: API token visible in process listing via `--token` flag [MEDIUM]

**File:** `src/main.rs:26`

The `--token` CLI flag passes the API token as a command-line argument. This exposes the
token in:

- Process listings (`ps aux`, `/proc/*/cmdline`)
- Shell history files (`~/.bash_history`, `~/.zsh_history`)
- System audit logs

**Recommendation:** Add support for the `UPTRAKIT_TOKEN` environment variable as an
alternative (higher-priority than stored credentials, lower than `--token`). Document that
`--token` is for development/scripting only and environment variables are preferred.

The `--server` flag has the same concern to a lesser degree (add `UPTRAKIT_SERVER`).

---

### S-3: No URL validation before opening in browser [MEDIUM]

**File:** `src/commands/auth.rs:368-384`

The `open_url()` function passes the `verification_url` directly to OS shell commands
(`open`, `xdg-open`, `cmd /C start`). While this URL comes from the server's device auth
response, a compromised server could return a malicious URL (e.g., `file:///etc/passwd` or
a `javascript:` scheme).

**Recommendation:** Validate that the URL starts with `https://` (or `http://` in insecure
mode) before passing to the OS command.

---

### S-4: No warning when `--insecure` flag is used [LOW]

**File:** `src/main.rs:29`

When `--insecure` is passed, TLS certificate verification is silently disabled. Other CLI
tools (e.g., `curl -k`, `wget --no-check-certificate`) print warnings when operating
insecurely.

**Recommendation:** Print a warning to stderr when `--insecure` is active, e.g.:
`WARNING: TLS certificate verification is disabled. Connection is insecure.`

---

## 2. Architecture

### A-1: `check::installed` and `check::available` are functionally identical [HIGH]

**File:** `src/commands/check.rs:44-66`

Both `installed()` and `available()` delegate to `check_item()` with identical arguments
and no distinguishing logic. The two subcommands appear to exist for semantic UX purposes
but call the exact same API endpoint, making the distinction misleading to users.

```rust
pub async fn installed(...) -> Result<()> {
    check_item(item_id, host_id, server, token, format, insecure).await
}

pub async fn available(...) -> Result<()> {
    check_item(item_id, host_id, server, token, format, insecure).await
}
```

**Impact:** A user running `uptrakit check installed <id>` expects only an installed-version
check, but gets the same behavior as `check available`. This is either:

- A **missing feature**: the API endpoint should accept a `check_type` parameter, or
  separate endpoints should exist.
- **Dead differentiation**: if the API intentionally handles both in one call, the CLI
  should expose a single `check item` subcommand instead of two aliases.

**Recommendation:** Either differentiate the behavior (pass a check-type parameter to the
API) or collapse into a single `check item` subcommand.

---

### A-2: Duplicated auth resolution logic [MEDIUM]

**File:** `src/commands/auth.rs:330-348` vs `src/client.rs:8-27`

The `resolve_auth()` function in `auth.rs` duplicates the server/token resolution logic
that already exists in `client.rs::authenticated_client()`. Both load config and credentials,
apply overrides, and fail with `CliError::NotLoggedIn`.

**Recommendation:** Extract a `resolve_server_and_token()` function in `config.rs` or
`client.rs` that returns `(String, String)`, and have both `authenticated_client()` and
`auth::resolve_auth()` use it.

---

### A-3: Inconsistent parameter passing patterns across commands [MEDIUM]

Some commands use parameter structs while others use 5-7 loose parameters:

| Pattern | Commands |
| --- | --- |
| **Parameter struct** | `services::list`, `history::list`, `update::trigger`, `settings::registration_update`, `settings::network_update`, `settings::mqtt_create`, `settings::mqtt_update`, `settings::oidc_create`, `settings::oidc_update` |
| **Loose parameters** | `hosts::list` (6 params), `hosts::show` (5), `software_items::list` (6), `software_items::show` (5), `scheduler::show` (5), `scheduler::trigger` (5), `check::installed` (6), `check::available` (6), most settings show/simple commands |

**Recommendation:** Adopt a consistent convention. The simplest rule: use a parameter struct
when a function takes more than 4 parameters.

---

### A-4: `api::execute` calls `std::process::exit(1)` inside a `Result`-returning function [MEDIUM]

**File:** `src/commands/api.rs:57-59`

```rust
if resp.status >= 400 {
    std::process::exit(1);
}
```

This bypasses Rust's normal cleanup (Drop implementations, buffered I/O flush) and is
inconsistent with the rest of the crate which propagates errors via `Result<()>`.

**Recommendation:** Return a `CliError` instead.

---

## 3. Code Quality

### Q-1: Human output formatting has inconsistent spacing after colons [MEDIUM]

Several "show" commands have misaligned label-value pairs where the space after the colon is
missing:

| File | Line | Current | Expected |
| --- | --- | --- | --- |
| `hosts.rs` | 61 | `"Friendly Name:{}\n"` | `"Friendly Name: {}\n"` |
| `services.rs` | 80 | `"Client Version:{}\n"` | `"Client Version: {}\n"` |
| `settings.rs` | 249 | `"Fwd Cert Info Header:{}\n"` | `"Fwd Cert Info Header: {}\n"` |
| `settings.rs` | 309 | `"Fwd Cert Info Header:{}\n"` | `"Fwd Cert Info Header: {}\n"` |
| `settings.rs` | 628 | `"Auto Create Users:{}\n"` | `"Auto Create Users: {}\n"` |

### Q-2: Auth output types use `String` instead of `Uuid` for IDs [MEDIUM]

**File:** `src/commands/auth.rs:14-51`

Per AGENTS.md rule 16: "All entity ID parameters must use `&Uuid` (not `&str`), and all
response ID fields must be `Uuid` (not `String`)."

### Q-3: `update::trigger` human output uses `{:?}` (Debug format) for status [MEDIUM]

**File:** `src/commands/update.rs:46-47`

The `{:?}` format will print the Rust Debug representation instead of human-readable output.

### Q-4: `status_text()` in `api.rs` has limited coverage [LOW]

The function only maps 9 HTTP status codes and returns `""` for anything else.

### Q-5: `api.rs` mixes stderr and stdout for Human format [LOW]

Status line goes to stderr while body goes to stdout. Should be documented or made consistent.

### Q-6: `TokenEntry::created_at` and `TokenEntry::status` use raw strings [LOW]

Could use typed datetime and enum respectively.

---

## 4. High Availability

### H-1: No timeout configuration for API calls [LOW]

The CLI inherits reqwest's default timeouts. Consider adding a `--timeout` global flag.

### H-2: No retry logic for transient API failures [INFO]

Acceptable for a CLI tool; user can retry manually.

---

## 5. Coding Standards Compliance

### C-1: `ensure_dirs()` not called before config/credential file operations [MEDIUM]

**File:** `src/config.rs:22-54`

Not a bug but inconsistent with other binaries that explicitly call `ensure_dirs()`.

### C-2: Missing test coverage for command execution logic [MEDIUM]

60+ tests for CLI argument parsing, but no tests for error paths, config loading, or
formatting logic.

### C-3: No `#[non_exhaustive]` on public output structs [LOW]

Low priority since types are only used within the crate.

---

## 6. Positive Observations

- **Error handling** follows project conventions perfectly: typed `CliError` with `thiserror`,
  `Result<T>` alias with `rootcause::Report`, and `impl_report_conversion!`.
- **Secure file operations** use `uptrakit_directories::write_secure_file_str`.
- **Token secrecy** is well maintained: tokens never logged, shown once with "store securely" warning.
- **Build info** follows the unified version/build metadata contract.
- **Device auth flow** correctly handles polling with rate limiting, timeout, and expiry.
- **CLI parsing tests** are comprehensive with 60+ test cases.
- **Output format support** (Human/JSON/YAML) is consistent across all commands.

---

## Fix Priority

| Priority | ID | Description |
| --- | --- | --- |
| 1 | S-1 | Move credentials to state directory |
| 2 | A-1 | Fix or collapse `check installed`/`check available` |
| 3 | A-4 | Remove `process::exit(1)` from api.rs |
| 4 | Q-1 | Fix human output spacing inconsistencies |
| 5 | Q-2 | Use `Uuid` for auth output ID fields |
| 6 | Q-3 | Fix Debug format in update trigger output |
| 7 | A-2 | Extract shared auth resolution logic |
| 8 | S-2 | Add environment variable support for token/server |
| 9 | S-3 | Validate URL before opening in browser |
| 10 | A-3 | Standardize parameter passing (struct vs loose) |
| 11 | C-1 | Call `ensure_dirs()` in config operations |
| 12 | C-2 | Add tests for formatting and error paths |
| 13 | S-4 | Warn on `--insecure` usage |
| 14 | Q-4 | Expand status_text coverage |
| 15 | Q-5 | Document or fix stderr/stdout mixing in api command |
