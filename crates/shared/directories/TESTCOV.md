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

- **Non-Unix platform code** (`lib.rs`): Remaining uncovered lines are in `#[cfg(not(unix))]` blocks which cannot be
  exercised on the current platform. The `write_secure_file_str`, `write_secure_file`, `set_file_permissions`, and
  tilde expansion functions are now tested.

## Test Recommendations

1. **Ensure dirs error handling tests** — Test `ensure_dirs()` when parent directory is read-only or path is invalid. Covers
   error paths in `AppDirs` (Tier 2). Use temp directories with restricted permissions.
2. **Tilde expansion edge cases** — Test `~` expansion with unset HOME and paths like `~/../../etc`. Covers remaining
   tilde expansion gaps (Tier 2). Temporarily modify environment variables.
