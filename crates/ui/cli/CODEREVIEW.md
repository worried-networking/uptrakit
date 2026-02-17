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
|----------|---------|
| **CRITICAL** | Security vulnerability or data loss risk; must fix before merge |
| **HIGH** | Significant bug or design flaw; should fix before merge |
| **MEDIUM** | Code quality, consistency, or correctness issue; fix soon |
| **LOW** | Minor improvement or style inconsistency; fix at convenience |
| **INFO** | Observation or suggestion; no action required |

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

The auth module needs the raw `(server, token)` strings for display (e.g., in `auth status`
output), which is why it doesn't use `authenticated_client()` directly. However, the
resolution logic should be factored into a shared function that both can call.

**Recommendation:** Extract a `resolve_server_and_token()` function in `config.rs` or
`client.rs` that returns `(String, String)`, and have both `authenticated_client()` and
`auth::resolve_auth()` use it.

---

### A-3: Inconsistent parameter passing patterns across commands [MEDIUM]

Some commands use parameter structs while others use 5-7 loose parameters:

| Pattern | Commands |
|---------|----------|
| **Parameter struct** | `services::list`, `history::list`, `update::trigger`, `settings::registration_update`, `settings::network_update`, `settings::mqtt_create`, `settings::mqtt_update`, `settings::oidc_create`, `settings::oidc_update` |
| **Loose parameters** | `hosts::list` (6 params), `hosts::show` (5), `software_items::list` (6), `software_items::show` (5), `scheduler::show` (5), `scheduler::trigger` (5), `check::installed` (6), `check::available` (6), most settings show/simple commands |

**Recommendation:** Adopt a consistent convention. The simplest rule: use a parameter struct
when a function takes more than 4 parameters (the `server`, `token`, `format`, `insecure`
"context" group counts as 4 already, so any command with additional parameters should use a
struct).

---

### A-4: `api::execute` calls `std::process::exit(1)` inside a `Result`-returning function [MEDIUM]

**File:** `src/commands/api.rs:57-59`

```rust
if resp.status >= 400 {
    std::process::exit(1);
}
```

This bypasses Rust's normal cleanup (Drop implementations, buffered I/O flush) and is
inconsistent with the rest of the crate which propagates errors via `Result<()>`. The
function already returns `Ok(())` after this check, so the exit is the only way to signal
failure for HTTP errors.

**Recommendation:** Return a `CliError` instead:

```rust
if resp.status >= 400 {
    bail!(CliError::Api {
        status: resp.status,
        message: status_text(resp.status).to_string(),
    });
}
```

Note: The human/JSON output is already printed before this point, so the user sees the
response body. The error propagation would then produce the `Error:` line from main's
error handler.

---

## 3. Code Quality

### Q-1: Human output formatting has inconsistent spacing after colons [MEDIUM]

Several "show" commands have misaligned label-value pairs where the space after the colon is
missing:

| File | Line | Current | Expected |
|------|------|---------|----------|
| `hosts.rs` | 61 | `"Friendly Name:{}\n"` | `"Friendly Name: {}\n"` |
| `services.rs` | 80 | `"Client Version:{}\n"` | `"Client Version: {}\n"` |
| `settings.rs` | 249 | `"Fwd Cert Info Header:{}\n"` | `"Fwd Cert Info Header: {}\n"` |
| `settings.rs` | 309 | `"Fwd Cert Info Header:{}\n"` | `"Fwd Cert Info Header: {}\n"` |
| `settings.rs` | 628 | `"Auto Create Users:{}\n"` | `"Auto Create Users: {}\n"` |

These break the visual alignment that other fields maintain.

---

### Q-2: Auth output types use `String` instead of `Uuid` for IDs [MEDIUM]

**File:** `src/commands/auth.rs:14-51`

The serializable output structs use `String` for ID fields:

```rust
pub struct AuthStatusOutput {
    pub user_id: String,  // should be Uuid
    // ...
}
pub struct TokenCreateOutput {
    pub id: String,  // should be Uuid
    // ...
}
pub struct TokenEntry {
    pub id: String,  // should be Uuid
    // ...
}
pub struct TokenRevokeOutput {
    pub id: String,  // should be Uuid
    // ...
}
```

Per the project's coding standards (AGENTS.md rule 16): "All entity ID parameters must use
`&Uuid` (not `&str`), and all response ID fields must be `Uuid` (not `String`)."

---

### Q-3: `update::trigger` human output uses `{:?}` (Debug format) for status [MEDIUM]

**File:** `src/commands/update.rs:46-47`

```rust
let human = format!(
    "Update triggered.\n  History ID: {}\n  Status:     {:?}\n",
    resp.update_history_id, resp.status
);
```

The `{:?}` format will print the Rust Debug representation of the status enum (e.g.,
`Pending` instead of `pending`). Human output should use Display formatting.

**Recommendation:** Use `{}` or the status type's `as_str()`/`Display` implementation
for human-readable output.

---

### Q-4: `status_text()` in `api.rs` has limited coverage [LOW]

**File:** `src/commands/api.rs:71-84`

The function only maps 9 HTTP status codes and returns `""` for anything else. Common codes
missing include:

- 202 Accepted
- 301 Moved Permanently
- 302 Found
- 422 Unprocessable Entity
- 429 Too Many Requests
- 502 Bad Gateway
- 503 Service Unavailable

**Recommendation:** Either expand the list or use a well-known crate (`http::StatusCode`)
for canonical reason phrases.

---

### Q-5: `api.rs` mixes stderr and stdout for Human format [LOW]

**File:** `src/commands/api.rs:37-41`

```rust
OutputFormat::Human => {
    eprintln!("HTTP {} {}", resp.status, status_text(resp.status));
    if !resp.body.is_null() {
        print_value(format, &resp.body)?; // prints to stdout
    }
}
```

The status line goes to stderr while the body goes to stdout. While this can be useful for
piping (body only goes to stdout), it's inconsistent with other commands where all human
output goes to stdout. This should either be documented as intentional or made consistent.

---

### Q-6: `TokenEntry::created_at` and `TokenEntry::status` use raw strings [LOW]

**File:** `src/commands/auth.rs:39-44`

```rust
pub struct TokenEntry {
    pub created_at: String,  // should be typed datetime
    pub status: String,      // should be enum
}
```

The `status` field is computed from `revoked_at.is_some()` and could be a typed enum
(`Active` | `Revoked`). The `created_at` field comes from the API as a string, so keeping
it as `String` is acceptable but loses type safety.

---

## 4. High Availability

### H-1: No timeout configuration for API calls [LOW]

The CLI inherits reqwest's default timeouts. Long-running operations (large history queries,
slow networks) can cause the CLI to hang indefinitely.

**Recommendation:** Consider adding a `--timeout` global flag (default 30s) passed to the
client builder.

---

### H-2: No retry logic for transient API failures [INFO]

Regular API calls have no retry logic. The device auth polling loop handles rate limiting
correctly, but standard CRUD operations fail immediately on transient network errors (DNS
resolution, TCP timeouts, 502/503 responses).

For a CLI tool, this is acceptable (the user can retry manually), but it's worth noting for
future improvement if the CLI is used in automated scripts.

---

## 5. Coding Standards Compliance

### C-1: `ensure_dirs()` not called before config/credential file operations [MEDIUM]

**File:** `src/config.rs:22-54`

The `load_config()`, `save_config()`, `load_credentials()`, and `save_credentials()`
functions do not call `dirs.ensure_dirs()` before accessing the config/state directories.
While `write_secure_file_str` creates parent directories, `load_config/load_credentials`
rely on `path.exists()` which would return false if the directory doesn't exist. This is
functionally correct (returns default) but `save_config` could fail on a fresh install if
the directory doesn't exist yet.

**Note:** `write_secure_file_str` in `uptrakit-directories` does create parent directories,
so this is not a bug but an inconsistency with other binaries that explicitly call
`ensure_dirs()`.

---

### C-2: Missing test coverage for command execution logic [MEDIUM]

The test suite (`main.rs:1309-2346`, `output.rs:74-138`, `auth.rs:386-471`) covers:

- CLI argument parsing (extensive, 60+ tests)
- Output serialization (JSON/YAML round-trips)
- Date formatting

However, there are no tests for:

- Error path behavior (what happens when the API returns errors)
- Config/credential file loading edge cases
- Output formatting logic for human-readable output
- The `authenticated_client` construction logic

The command handlers themselves are hard to unit-test due to async API calls, but the
formatting logic (human string construction) could be tested independently.

---

### C-3: No `#[non_exhaustive]` on public output structs [LOW]

**File:** `src/commands/auth.rs:14-51`

The `AuthStatusOutput`, `TokenCreateOutput`, `TokenListOutput`, `TokenEntry`, and
`TokenRevokeOutput` structs are `pub` but not `#[non_exhaustive]`. Per the project's coding
standards, public enums should have `#[non_exhaustive]`. While the standard specifically
mentions enums, applying the same principle to public structs used as serialized output
prevents breaking changes when fields are added.

**Note:** These types are currently only used within the crate, so this is low priority.

---

## 6. Positive Observations

The following aspects are well-implemented:

- **Error handling** follows project conventions perfectly: typed `CliError` with `thiserror`,
  `Result<T>` alias with `rootcause::Report`, and `impl_report_conversion!` for all
  cross-boundary conversions.
- **Secure file operations** use `uptrakit_directories::write_secure_file_str` for all
  credential and config persistence (0o600 permissions, atomic creation).
- **Token secrecy** is well maintained: tokens are never logged, and the newly created token
  is intentionally shown once with a "store securely" warning.
- **Build info** follows the unified version/build metadata contract using
  `uptrakit_build_info`.
- **Typed API client** properly leverages `uptrakit-openapi-client` for all operations,
  avoiding raw HTTP calls (except the intentional `api` escape hatch).
- **Device auth flow** correctly handles polling with rate limiting, timeout, and expiry.
- **CLI parsing tests** are comprehensive with 60+ test cases covering all subcommands.
- **Output format support** (Human/JSON/YAML) is consistent across all commands.
- **UUID parameters** are parsed by clap at the type level, preventing invalid input.

---

## Fix Priority

| Priority | ID | Description |
|----------|----|-------------|
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
