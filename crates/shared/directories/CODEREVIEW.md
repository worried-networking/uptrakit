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

### PASS: Atomic permission setting on file writes

`write_with_mode` sets the mode at `open()` time via `OpenOptionsExt`, eliminating the classic create-then-chmod TOCTOU
window. Correctly documented.

### PASS: No production `unwrap`/`panic`

Zero instances in non-test code. All fallible operations return `Result`.

### LOW: Non-Unix platforms get no permission hardening

On `#[cfg(not(unix))]`, files and directories are created with default permissions. `set_dir_permissions` and
`set_file_permissions` are complete no-ops. Acceptable if the project targets Unix only, but the code comments say
"cross-platform."

### LOW: `~user` syntax not handled

The function only handles `~` and `~/...`, not `~otheruser/...`. Should be documented explicitly.

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
