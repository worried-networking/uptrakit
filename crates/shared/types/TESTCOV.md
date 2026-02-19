# Test Coverage: uptrakit-shared-types

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 96.6% (711 / 736) |
| Function coverage | 93.6% (117 / 125) |
| Test count | 77 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| provider_types.rs | 100.0% | 145/145 | 100.0% | 17/17 |
| secret_string.rs | 100.0% | 71/71 | 100.0% | 13/13 |
| hex.rs | 98.3% | 59/60 | 100.0% | 14/14 |
| mqtt_transport.rs | 96.5% | 82/85 | 94.4% | 17/18 |
| output_stream_type.rs | 96.1% | 73/76 | 90.0% | 9/10 |
| mqtt_connection_status.rs | 95.3% | 61/64 | 90.0% | 9/10 |
| device_auth_status.rs | 94.9% | 56/59 | 88.9% | 8/9 |
| service_status.rs | 94.6% | 53/56 | 87.5% | 7/8 |
| hook_shell.rs | 92.9% | 39/42 | 88.9% | 8/9 |
| session_token_type.rs | 92.3% | 36/39 | 88.9% | 8/9 |
| service_type.rs | 92.3% | 36/39 | 87.5% | 7/8 |

## Uncovered Critical Paths

### Tier 3 — Supporting

- **Enum variant coverage gaps** (across multiple files, ~25 uncovered lines total): Each enum type has 1–3 uncovered lines
  typically in `sea_orm` or `openapi` feature-gated trait implementations (`ActiveEnum`, `ToSchema`). These are compile-time
  derivations with minimal runtime logic. Risk: negligible — these are generated implementations.

## Test Recommendations

1. **Feature-gated trait implementation tests** — Test `ActiveEnum` and `ToSchema` implementations for each enum type under
   `sea-orm` and `openapi` features. Covers remaining gaps (Tier 3). Requires feature-specific test configurations.
2. **Hex decode edge cases** — Test hex decoding with mixed-case input and boundary-length strings. Covers `hex.rs` remaining
   1 line (Tier 3). Simple unit test.
