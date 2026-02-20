# Test Coverage: uptrakit-service-sdk

> Generated: 2026-02-20 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 62.1% (1,406 / 2,264) |
| Function coverage | 67.1% (190 / 283) |
| Test count | 88 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| backoff.rs | 100.0% | 55/55 | 100.0% | 7/7 |
| error.rs | 96.8% | 90/93 | 88.9% | 16/18 |
| cli.rs | 93.8% | 213/227 | 77.4% | 24/31 |
| identity.rs | 87.0% | 529/608 | 84.1% | 69/82 |
| tls.rs | 82.5% | 217/263 | 86.1% | 31/36 |
| cert_handler.rs | 80.9% | 216/267 | 89.5% | 34/38 |
| ca.rs | 42.4% | 86/203 | 50.0% | 9/18 |
| connection.rs | 0.0% | 0/100 | 0.0% | 0/13 |
| lifecycle.rs | 0.0% | 0/175 | 0.0% | 0/14 |
| ws.rs | 0.0% | 0/273 | 0.0% | 0/26 |

## Uncovered Critical Paths

### Tier 1 — Security

- **CA certificate validation** (`ca.rs`, 42.4% coverage, 203 lines): CA certificate validation, fingerprint verification
  remaining gaps. Risk: incomplete validation could allow unauthorized certificates.

### Tier 2 — Business-Logic

- **WebSocket communication** (`ws.rs`, 0% coverage, 273 lines): WebSocket connection handling, message framing,
  reconnection logic, and graceful close handling. Risk: reconnection failures could cause silent service disconnection.
- **Service lifecycle** (`lifecycle.rs`, 0% coverage, 175 lines): Service lifecycle management, startup, enrollment,
  authenticated loop, and graceful shutdown coordination. Risk: lifecycle bugs could leave services in inconsistent states.
- **Connection management** (`connection.rs`, 0% coverage, 100 lines): Controller connection, URL resolution, connection
  pooling, and retry logic. Risk: connection management failures could prevent service registration.
- **Certificate handler gaps** (`cert_handler.rs`, 80.9% coverage): `handle_request_cert_renewal` and `handle_renewal_timer`
  methods need real connection to test. Renewal timer scheduling and handler state tests added.

### Tier 3 — Supporting

- **Error handling gaps** (`error.rs`, 96.8% coverage): Remaining 3 uncovered lines in error conversion paths.

## Test Recommendations

1. **WebSocket lifecycle tests** — Test connection establishment, message send/receive, reconnection after disconnect, and graceful
   close. Covers `ws.rs` (Tier 2). Use `tokio-tungstenite` test server.
2. **Service lifecycle integration tests** — Test enrollment, authenticated loop entry, and shutdown sequence. Covers
   `lifecycle.rs` (Tier 2). Requires mock controller endpoint.
3. **Connection retry tests** — Test connection retry with exponential backoff, URL failover, and timeout handling. Covers
   `connection.rs` (Tier 2). Unit-testable with mock HTTP client.
4. **CA validation tests** — Test CA certificate validation and fingerprint verification paths. Covers `ca.rs` (Tier 1).
   Critical for security assurance.
