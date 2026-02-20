# Test Coverage: uptrakit-command

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 67.2% (531 / 790) |
| Function coverage | 70.6% (89 / 126) |
| Test count | 40 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| executor.rs | 75.1% | 130/173 | 75.9% | 22/29 |
| command.rs | 65.0% | 401/617 | 69.1% | 67/97 |
| types.rs | — | — | — | — |

> **Note:** Previous report showed 92.2% (463/502) totals because 0%-coverage entries from other
> compilation units were excluded. The actual measured codebase is larger than previously reported.
> types.rs was not measured in this run.

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **command.rs remaining gaps (65.0%)**: Output truncation at the 10 MB limit, signal propagation,
  task-join failure fallback, and some streaming output paths via mpsc channels remain uncovered.
  New tests added for working directory errors, stderr capture, and nonexistent program handling.
- **executor.rs remaining gaps (75.1%)**: Some executor paths are not yet exercised by the test
  suite.

## Test Recommendations

1. **Large output truncation tests** — Test output exceeding the 10 MB buffer limit. Covers `command.rs` truncation paths
   (Tier 2). Use `yes | head -c 11000000` or similar.
2. **Signal propagation tests** — Test that child processes receive SIGTERM on cancellation. Covers `command.rs` gaps (Tier 2).
   Use a command that traps signals.
3. **Streaming output paths** — Exercise the mpsc-channel-based streaming output collection to cover remaining `command.rs`
   branches.
4. **Executor edge cases** — Add tests targeting the uncovered executor.rs paths to improve coverage from 75.1%.
