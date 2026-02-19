# Test Coverage: uptrakit-command

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 92.2% (463 / 502) |
| Function coverage | 93.0% (80 / 86) |
| Test count | 37 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| executor.rs | 100.0% | 130/130 | 100.0% | 22/22 |
| types.rs | 100.0% | 8/8 | 100.0% | 2/2 |
| command.rs | 89.3% | 325/364 | 90.3% | 56/62 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Command execution edge cases** (`command.rs`): Remaining uncovered lines include the 10 MB output truncation path
  (requires generating >10 MB output), signal propagation, and the task-join failure fallback messages. Working directory
  errors and spawn failures are now tested.

## Test Recommendations

1. **Large output truncation tests** — Test output exceeding the 10 MB buffer limit. Covers `command.rs` truncation paths
   (Tier 2). Use `yes | head -c 11000000` or similar.
2. **Signal propagation tests** — Test that child processes receive SIGTERM on cancellation. Covers `command.rs` gaps (Tier 2).
   Use a command that traps signals.
