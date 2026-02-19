# Test Coverage: uptrakit-service-sdk

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 52.9% (1,083 / 2,046) |
| Function coverage | 56.8% (142 / 250) |
| Test count | 62 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| backoff.rs | 100.0% | 55/55 | 100.0% | 7/7 |
| error.rs | 95.8% | 92/96 | 83.3% | 15/18 |
| cli.rs | 93.8% | 213/227 | 77.4% | 24/31 |
| identity.rs | 87.0% | 529/608 | 84.1% | 69/82 |
| cert_handler.rs | 71.7% | 165/230 | 76.7% | 23/30 |
| ca.rs | 19.9% | 29/146 | 30.8% | 4/13 |
| connection.rs | 0.0% | 0/100 | 0.0% | 0/13 |
| lifecycle.rs | 0.0% | 0/175 | 0.0% | 0/14 |
| tls.rs | 0.0% | 0/136 | 0.0% | 0/16 |
| ws.rs | 0.0% | 0/273 | 0.0% | 0/26 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **WebSocket communication** (`ws.rs`, 0% coverage, 273 lines): WebSocket connection establishment, message framing,
  reconnection logic, and graceful close handling. Risk: reconnection failures could cause silent service disconnection.
- **Service lifecycle** (`lifecycle.rs`, 0% coverage, 175 lines): Service startup, enrollment, authenticated loop, and graceful
  shutdown coordination. Risk: lifecycle bugs could leave services in inconsistent states.
- **Connection management** (`connection.rs`, 0% coverage, 100 lines): Controller URL resolution, connection pooling, and
  retry logic. Risk: connection management failures could prevent service registration.
- **Identity module gaps** (`identity.rs`, 87.0% coverage): 79 uncovered lines include CSR generation edge cases, state
  persistence error handling, and concurrent enrollment scenarios.

## Test Recommendations

1. **WebSocket lifecycle tests** — Test connection establishment, message send/receive, reconnection after disconnect, and graceful
   close. Covers `ws.rs` (Tier 2). Use `tokio-tungstenite` test server.
2. **Service lifecycle integration tests** — Test enrollment, authenticated loop entry, and shutdown sequence. Covers
   `lifecycle.rs` (Tier 2). Requires mock controller endpoint.
3. **Connection retry tests** — Test connection retry with exponential backoff, URL failover, and timeout handling. Covers
   `connection.rs` (Tier 2). Unit-testable with mock HTTP client.
