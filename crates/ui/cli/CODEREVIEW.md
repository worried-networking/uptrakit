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

### ~~S-3: No URL validation before opening in browser~~ [FIXED]

**Resolution:** Added `validate_url_scheme()` function that allows only `https://` URLs
(or `http://` when `--insecure` is active). The validation runs before `open_url()` in the
device auth login flow. Dangerous schemes (`file://`, `javascript:`, etc.) are rejected.
Tests added.

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

### ~~A-1: `check::installed` and `check::available` are functionally identical~~ [FIXED]

**Resolution:** Collapsed `check installed` and `check available` into a single
`check item` subcommand. The `installed()` and `available()` functions were removed
and replaced with `item()`. CLI tests updated to match.

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
| ~~1~~ | ~~A-1~~ | ~~Fix or collapse `check installed`/`check available`~~ (FIXED) |
| 2 | Q-1 | Fix human output spacing inconsistencies |
| 3 | Q-2 | Use `Uuid` for auth output ID fields |
| 4 | Q-3 | Fix Debug format in update trigger output |
| 5 | A-2 | Extract shared auth resolution logic |
| 6 | S-2 | Add environment variable support for token/server |
| ~~7~~ | ~~S-3~~ | ~~Validate URL before opening in browser~~ (FIXED) |
| 8 | A-3 | Standardize parameter passing (struct vs loose) |
| 9 | C-1 | Call `ensure_dirs()` in config operations |
| 10 | C-2 | Add tests for formatting and error paths |
| 11 | S-4 | Warn on `--insecure` usage |
| 12 | Q-4 | Expand status_text coverage |
| 13 | Q-5 | Document or fix stderr/stdout mixing in api command |
