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

### MEDIUM: Recursive `DirBuilder` applies mode only to leaf directory

**File:** `src/lib.rs`, lines 183-186

When `recursive(true)` is used, `DirBuilder` applies the specified `mode` only to the **leaf** directory. Intermediate
directories are created with the default mode (typically `0o777` masked by umask). This is documented Rust stdlib
behavior.

If `create_secure_dir("/a/b/c/target")` is called and `/a/b/c` does not exist, `/a/b/c` could be created with `0o755`
while only `target` gets `0o700`.

**Recommendation:** Either iterate through each path component and create them individually with `0o700`, or document
that callers must ensure parent directories exist.

### MEDIUM / SECURITY: `config_path` and `state_path` do not sanitize the `name` argument

**File:** `src/lib.rs`, lines 142-149

```rust
pub fn config_path(&self, name: &str) -> PathBuf {
    self.config.join(name)
}
```

If `name` contains `..` components or an absolute path, `PathBuf::join` will traverse outside the intended directory or
replace the base path entirely. Currently, callers use only hardcoded string literals, so this is not immediately
exploitable. As a shared library, defensive validation would be prudent.

**Recommendation:** Add validation that `name` does not contain path separators or `..` components.

### MEDIUM: `to_string_lossy()` can corrupt non-UTF-8 paths in `expand_tilde`

**File:** `src/lib.rs`, line 155

`to_string_lossy()` replaces invalid Unicode sequences with U+FFFD. On Unix, paths can contain non-UTF-8 bytes, so this
function could silently corrupt such paths.

**Recommendation:** Use `OsStr`-based operations instead of lossy string conversion:

```rust
pub fn expand_tilde(path: &Path) -> Result<PathBuf> {
    let mut components = path.components();
    if let Some(std::path::Component::Normal(first)) = components.next() {
        if first == "~" {
            let home = home_dir().ok_or_else(|| report!(DirectoryError::NoHomeDir))?;
            let rest: PathBuf = components.collect();
            return Ok(home.join(rest));
        }
    }
    Ok(path.to_path_buf())
}
```

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
| Path traversal          | **MEDIUM** | `config_path`/`state_path` accept unsanitized names        |
| Error handling          | PASS       | rootcause/thiserror with contextual path information       |
| `unwrap`/`panic`        | PASS       | Zero in production code                                    |
| Cross-platform          | FAIR       | Permission hardening is Unix-only; docs say "cross-platform" |
| Test coverage           | FAIR       | Core paths covered; many edge cases and error paths missing  |
