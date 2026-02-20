# Test Coverage: uptrakit-directories

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 87.4% (396 / 453) |
| Function coverage | 83.1% (59 / 71) |
| Test count | 23 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| lib.rs | 87.4% | 396/453 | 83.1% | 59/71 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Non-Unix platform code** (`lib.rs`): Remaining uncovered lines are in `#[cfg(not(unix))]` platform-specific blocks
  that cannot be exercised on macOS.
- **Error handling in `ensure_dirs()`** (`lib.rs`): Some error handling paths in `ensure_dirs()` remain uncovered,
  such as when parent directories are read-only or paths are invalid.

## Test Recommendations

1. **Ensure dirs error handling tests** — Test `ensure_dirs()` when parent directory is read-only or path is invalid. Covers
   error paths in `AppDirs` (Tier 2). Use temp directories with restricted permissions.
2. **Cross-platform CI** — Run tests on Windows to cover `#[cfg(not(unix))]` blocks that cannot be exercised on macOS.
   This would cover the remaining platform-specific code paths (Tier 2).
