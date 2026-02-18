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

### MEDIUM: Sync `create_secure_dir` called from async context

**File:** `src/lib.rs`, lines 233-237

`write_secure_file_async` calls sync `create_secure_dir`. The comment acknowledges this. On network filesystems, `mkdir`
can block for seconds, blocking the tokio runtime thread.

**Recommendation:** Wrap in `tokio::task::spawn_blocking` or provide an async `create_secure_dir_async` variant.

### PASS: Atomic permission setting on file writes

`write_with_mode` and `write_with_mode_async` both set the mode at `open()` time via `OpenOptionsExt`, eliminating the
classic create-then-chmod TOCTOU window. Correctly documented.

### PASS: No production `unwrap`/`panic`

Zero instances in non-test code. All fallible operations return `Result`.

### LOW: No atomic file replacement

Write functions use `create(true).truncate(true)` which overwrites in-place. If the process crashes mid-write, the file
is left partially written. For security-critical files (private keys, certificates), consider write-to-temp-then-rename.

### LOW: Missing `file.sync_all()` in sync write path

The async version calls `file.shutdown().await` for flush. The sync version does not explicitly call `file.sync_all()`.
For private keys and certificates, this could mean data loss on power failure.

### LOW: `home_dir()` only checks `$HOME` environment variable

**File:** `src/lib.rs`, lines 166-168

Returns `None` on Windows where `USERPROFILE` is used. The `directories` crate (already a dependency) provides
cross-platform home directory resolution.

### LOW: `ProjectDirs::from` qualifier mismatch with documentation

`ProjectDirs::from("org", "uptrakit", app_name)` produces `org.uptrakit.*` on macOS, but documentation says
`io.uptrakit`. Either change the qualifier to `"io"` or update the docs.

### LOW: Non-Unix platforms get no permission hardening

On `#[cfg(not(unix))]`, files and directories are created with default permissions. `set_dir_permissions` and
`set_file_permissions` are complete no-ops. Acceptable if the project targets Unix only, but the code comments say
"cross-platform."

### LOW: `~user` syntax not handled

The function only handles `~` and `~/...`, not `~otheruser/...`. Should be documented explicitly.

### LOW: Tests are Unix-only without cfg guards

`use std::os::unix::fs::PermissionsExt;` inside `#[cfg(test)]` but not behind `#[cfg(unix)]`. Tests will fail to
compile on Windows.

### LOW: Missing test coverage

Missing tests for: `expand_tilde` with relative paths, `~user` syntax, unset `$HOME`, `create_secure_dir` idempotency,
existing directory with wrong permissions, `write_secure_file` overwrite behavior, `write_secure_file_str`,
`set_dir_permissions`, `set_file_permissions`, `AppDirs::ensure_state_dir`, error paths, and content verification for
sync writes.

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
| Test coverage           | FAIR       | Core paths covered; many edge cases and error paths missing  |
