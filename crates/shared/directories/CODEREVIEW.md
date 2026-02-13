# Code Review: uptrakit-directories

## Summary

Cross-platform directory management crate (~352 lines, single source file) providing `AppDirs` resolution, secure directory/file creation with Unix permissions (0o700 for dirs, 0o600 for files), and tilde expansion. Uses the `directories` crate for platform-standard path resolution.

## Architecture

- **Module structure**: Single `lib.rs` with `AppDirs` struct, free functions for secure I/O, and tilde expansion.
- **Public API surface**: `AppDirs::resolve()`, `AppDirs::ensure_dirs()`, `AppDirs::config_dir()`, `AppDirs::state_dir()`, `AppDirs::config_path()`, `AppDirs::state_path()`, `expand_tilde()`, `create_secure_dir()`, `write_secure_file()`, `write_secure_file_str()`, `set_dir_permissions()`, `set_file_permissions()`.
- **Dependency choices**: `directories` (platform path resolution), `rootcause`/`thiserror` (error handling) -- minimal and appropriate.
- **Layering**: Used by all binaries (controller, agent, MQTT) for directory setup.

## Security & Safety

- **Permissions model**: 0o700 for directories, 0o600 for files. Unix-only via `#[cfg(unix)]`; no-op on Windows.
- **TOCTOU window in `write_secure_file()`**: File is created with `fs::write()` (default permissions ~0o644 depending on umask) at line 192, then permissions are restricted to 0o600 at line 199. There is a brief window where the file may be readable by other users. Using `OpenOptions` with `.mode(0o600)` on Unix would eliminate this.
- **Tilde expansion**: Uses `$HOME` environment variable via `std::env::var_os("HOME")`. Standard practice; could be spoofed in adversarial environments but acceptable for the intended use case.
- No `unsafe` code.
- No `unwrap`/`panic` in non-test code.

## Code Quality

- **Error handling**: `DirectoryError` enum with `NoProjectDirs`, `CreateDir`, `WriteFile`, `SetPermissions`, `NoHomeDir` variants. Uses `rootcause::Report` wrapper. All error paths provide path context.
- **Documentation**: Module-level doc comment and function-level doc comments present.
- **Test coverage**: 10 tests covering default resolution, overrides, tilde expansion (3 variants), directory permissions, file permissions, parent dir creation, `ensure_dirs()`, and path helpers.
- **Platform handling**: Linux uses `state_dir()` (XDG_STATE_HOME), other platforms use `data_local_dir()`. Conditional compilation is clean.

## Coding Standards Compliance

- Typed error enum with `thiserror` + `rootcause::Report` -- compliant.
- `Result<T>` type alias defined (`src/lib.rs:56`).
- No `#[allow()]` directives.

## Findings

| ID | Severity | Category | Description | File:Line |
| --- | --- | --- | --- | --- |
| DIR-01 | Medium | Security | TOCTOU permission window in `write_secure_file()`. File is created with `fs::write()` (default umask permissions), then restricted to 0o600 via `set_file_permissions()`. Brief window where sensitive files (keys, certificates) may be world-readable. Fix: use `std::fs::OpenOptions` with `std::os::unix::fs::OpenOptionsExt::mode(0o600)` on Unix. | `src/lib.rs:192-199` |
| DIR-02 | Low | Code Quality | Doc comment inaccuracy: `create_secure_dir()` doc says "Creates parent directories as needed, also with 700 permissions" but `set_dir_permissions()` is only called on the leaf directory. Intermediate directories created by `create_dir_all()` retain default umask permissions. | `src/lib.rs:165-167` |
| DIR-03 | Info | Code Quality | `create_secure_dir()` has a TOCTOU race on the `path.exists()` check at line 169. Another process could remove the directory between the check and `create_dir_all()`. The `create_dir_all()` call handles this gracefully (succeeds if dir exists), so the impact is cosmetic (the early return skips re-applying permissions to an existing dir). | `src/lib.rs:169-171` |
| DIR-04 | Info | Code Quality | Windows support is minimal: permission-setting functions are no-ops on non-Unix platforms. Acceptable given the project targets Linux primarily, but not documented in the public API. | `src/lib.rs:220-223`, `src/lib.rs:239-242` |

## Verdict

**Conditional pass.** The TOCTOU window in `write_secure_file()` (DIR-01) is a real security concern for sensitive files. The doc comment inaccuracy (DIR-02) should be corrected to avoid misleading consumers. The crate is otherwise well-structured with good error handling and test coverage.
