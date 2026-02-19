# Test Coverage: uptrakit-internal-wire

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 98.9% (1,301 / 1,316) |
| Function coverage | 98.4% (124 / 126) |
| Test count | 107 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| close_reason.rs | 100.0% | 80/80 | 100.0% | 11/11 |
| lib.rs | 98.8% | 1,221/1,236 | 98.3% | 113/115 |

## Uncovered Critical Paths

### Tier 3 — Supporting

- **Wire protocol edge cases** (`lib.rs`, 98.8% coverage): 15 uncovered lines include edge cases in message deserialization for
  unusual field combinations and protocol version mismatch handling. Risk: minimal — the wire protocol is extensively tested with
  107 tests covering all message types.

## Test Recommendations

1. **Protocol version mismatch test** — Test behavior when receiving a message with an unsupported protocol version. Covers
   `lib.rs` gaps (Tier 3). Simple deserialization test with modified version field.
2. **Malformed message deserialization tests** — Test handling of truncated, corrupted, or unexpected JSON payloads. Covers
   `lib.rs` remaining gaps (Tier 3). Fuzz-style tests with invalid input.
