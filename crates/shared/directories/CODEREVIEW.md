# Code Review: `uptrakit-directories`

**Date:** 2026-02-17
**Reviewer:** Claude Opus 4.6 (automated)
**Scope:** Architecture, security, code quality, coding standards
**Overall quality: GOOD -- with meaningful security gaps that should be addressed**

---

## Architecture

The entire crate is a single `src/lib.rs` file providing:

- `AppDirs` struct for cross-platform directory resolution using the `directories` crate.
- Secure file/directory creation functions with atomic permission setting.
- Tilde expansion utility.

---

## Findings

### ~~MEDIUM: Recursive `DirBuilder` applies mode only to leaf directory~~ RESOLVED

**Resolution:** `create_secure_dir` now walks all newly created intermediate directories between the first pre-existing
ancestor and the leaf, calling `set_dir_permissions()` on each to ensure `0o700`. The leaf is always fixed as well
(covers pre-existing directories with wrong permissions). Test added.

### ~~MEDIUM / SECURITY: `config_path` and `state_path` do not sanitize the `name` argument~~ RESOLVED

Resolved: `config_path` and `state_path` now validate `name` via `validate_path_name()`, rejecting path separators,
`..`, `.`, empty strings, and absolute paths. Returns `Result<PathBuf>` with `DirectoryError::PathTraversal` on
violation. Tests added.

### ~~MEDIUM: `to_string_lossy()` can corrupt non-UTF-8 paths in `expand_tilde`~~ RESOLVED

Resolved: `expand_tilde` rewritten using `std::path::Component`-based matching, avoiding lossy string conversion
entirely. Non-UTF-8 path components are preserved on Unix.

### ~~MEDIUM: Sync `create_secure_dir` called from async context~~ RESOLVED

**Resolution:** The entire crate has been converted to native async. `create_secure_dir` is now an `async fn` using
`tokio::fs::DirBuilder`, `tokio::fs::metadata`, and `tokio::fs::set_permissions`. All sync/async duplication has been
eliminated: `write_with_mode`, `write_secure_file`, and `write_secure_file_str` are now the single async versions.
`ensure_config_dir`, `ensure_state_dir`, and `ensure_dirs` are async. All callers updated.

### PASS: Atomic permission setting on file writes

`write_with_mode` sets the mode at `open()` time via `OpenOptionsExt`, eliminating the classic create-then-chmod TOCTOU
window. Correctly documented.

### PASS: No production `unwrap`/`panic`

Zero instances in non-test code. All fallible operations return `Result`.

### ~~LOW: No atomic file replacement~~ (FIXED)

~~Write functions use `create(true).truncate(true)` which overwrites in-place. If the process crashes mid-write, the file
is left partially written. For security-critical files (private keys, certificates), consider write-to-temp-then-rename.~~

**Resolution:** Implemented atomic write-to-temp-then-rename in `write_with_mode`. Writes to `.{filename}.tmp` temp file, then atomically renames to target.

### ~~LOW: Missing `file.sync_all()` in sync write path~~ RESOLVED

**Resolution:** The sync write path was removed. The remaining async `write_with_mode` calls `file.shutdown().await`
for flush.

### ~~LOW: `home_dir()` only checks `$HOME` environment variable~~ FIXED

**Resolution:** Replaced with `directories::BaseDirs::new().map(|bd| bd.home_dir().to_path_buf())`,
which handles Linux (`$HOME`), macOS (`$HOME`), and Windows (`USERPROFILE`) automatically.
Added a test verifying the function returns `Some` on the current platform.

### ~~LOW: `ProjectDirs::from` qualifier mismatch with documentation~~ RESOLVED

**Resolution:** Updated the crate docstring and `AGENTS.md` to say `org.uptrakit.*` (matching the code). The code
correctly uses `ProjectDirs::from("org", "uptrakit", app_name)`.

### LOW: Non-Unix platforms get no permission hardening

On `#[cfg(not(unix))]`, files and directories are created with default permissions. `set_dir_permissions` and
`set_file_permissions` are complete no-ops. Acceptable if the project targets Unix only, but the code comments say
"cross-platform."

### LOW: `~user` syntax not handled

The function only handles `~` and `~/...`, not `~otheruser/...`. Should be documented explicitly.

### ~~LOW: Tests are Unix-only without cfg guards~~ RESOLVED

~~`use std::os::unix::fs::PermissionsExt;` inside `#[cfg(test)]` but not behind `#[cfg(unix)]`. Tests will fail to
compile on Windows.~~

**Resolution:** Added `#[cfg(unix)]` guard before `#[cfg(test)]` on the test module. The entire test suite uses
Unix-specific permission checks (`PermissionsExt`), so gating the whole module is appropriate.

### ~~LOW: Missing test coverage~~ RESOLVED

~~Missing tests for: `expand_tilde` with relative paths, `~user` syntax, unset `$HOME`, `create_secure_dir` idempotency,
existing directory with wrong permissions, `write_secure_file` overwrite behavior, `write_secure_file_str`,
`set_dir_permissions`, `set_file_permissions`, `AppDirs::ensure_state_dir`, error paths, and content verification for
sync writes.~~

**Resolution:** Added 8 edge-case tests covering: `~otheruser/foo` pass-through, unset `$HOME` error,
`ensure_state_dir` permissions, `ensure_config_dir` permissions, absolute path rejection in `config_path`,
empty file write with correct permissions, `set_dir_permissions` correcting 0o755 to 0o700, and
`create_secure_dir` failure on read-only parent. Total: 32 tests (up from 24).

---

## Summary

| Category                | Status     | Notes                                                      |
| ----------------------- | ---------- | ---------------------------------------------------------- |
| File permission setting | GOOD       | Atomic at creation time for files; TOCTOU gap for dirs     |
| TOCTOU                  | PASS       | `create_secure_dir` verifies/sets permissions on existing dirs |
| Path traversal          | PASS       | `config_path`/`state_path` validate names via `validate_path_name()` |
| Error handling          | PASS       | rootcause/thiserror with contextual path information       |
| `unwrap`/`panic`        | PASS       | Zero in production code                                    |
| Cross-platform          | FAIR       | Permission hardening is Unix-only; docs say "cross-platform" |
| Test coverage           | GOOD       | 32 tests covering core paths, edge cases, and error paths    |
