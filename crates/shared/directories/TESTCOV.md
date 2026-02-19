# Test Coverage: uptrakit-directories

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 83.6% (326 / 390) |
| Function coverage | 77.4% (48 / 62) |
| Test count | 18 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| lib.rs | 83.6% | 326/390 | 77.4% | 48/62 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Secure file operations** (`lib.rs`, 83.6% coverage, 390 lines): 64 uncovered lines include `write_secure_file` and
  `write_secure_file_str` atomic write implementations, `create_secure_dir` permission enforcement on non-Unix platforms,
  `AppDirs::ensure_dirs()` error handling, and tilde expansion edge cases. Risk: untested secure file operations could create
  files with incorrect permissions, exposing sensitive data (private keys, certificates).
- **Path traversal validation** (`lib.rs`): `config_path` and `state_path` methods validate against path traversal attacks
  (rejecting `..`, `/`, empty strings). While the validation logic is tested, error message formatting paths are not.

## Test Recommendations

1. **Secure file write permission tests** — Test that `write_secure_file` creates files with 0o600 permissions and
   `create_secure_dir` creates directories with 0o700. Covers atomic permission paths (Tier 2). Use `tempfile` and verify
   permissions with `std::fs::metadata`.
2. **Ensure dirs error handling tests** — Test `ensure_dirs()` when parent directory is read-only or path is invalid. Covers
   error paths in `AppDirs` (Tier 2). Use temp directories with restricted permissions.
3. **Tilde expansion edge cases** — Test `~` expansion with unset HOME, empty HOME, and paths like `~/../../etc`. Covers
   tilde expansion gaps (Tier 2). Temporarily modify environment variables.
4. **Platform-specific directory resolution tests** — Test `AppDirs::resolve` on the current platform with custom overrides.
   Covers platform-specific code paths (Tier 2). Verify expected directory structure.
