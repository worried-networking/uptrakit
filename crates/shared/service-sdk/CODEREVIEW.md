# Code Review: `uptrakit-service-sdk`

**Rating:** EXCELLENT

The service-sdk crate provides a clean, unified abstraction for both agents and MQTT services: identity
management, TLS configuration, CA bootstrap, enrollment WebSocket flows, the `ControllerConnection`
wrapper with sequence-validated messaging, and a `ServiceHandler` trait with `run_service_lifecycle()`
for building new services with minimal boilerplate. File permissions are correctly set to 0o600/0o700.
The enrollment secret is properly cleared after certificate issuance. Test coverage is thorough,
including permission validation on Unix.

## Extensibility Assessment

The service-sdk is the primary entry point for external developers building new services. The
`ServiceHandler` trait and `run_service_lifecycle()` function encapsulate the entire
bootstrap-enrollment-reconnect flow, so new services only need to implement three methods:
`config()`, `enrollment_info()`, and `run_authenticated_loop()`. See
[Service Lifecycle](../../../docs/development/service-lifecycle.md) for the developer guide.

Remaining extensibility gaps:

1. ~~**`EnrollmentError` is overloaded**~~ **FIXED** — restructured into 4 domain sub-enums (`TlsError`,
   `IdentityError`, `ProtocolError`, `CaError`) plus 5 top-level variants.

2. **`WsStream` type alias leaks implementation details**: Exposes `tokio_tungstenite::WebSocketStream<
   tokio_rustls::client::TlsStream<tokio::net::TcpStream>>`. Switching TLS libraries or transport would
   break all consumers.

3. **Timeout constants are hardcoded**: `CONNECT_TIMEOUT` (30s), `RESPONSE_TIMEOUT` (60s),
   `APPROVAL_TIMEOUT` (30min) cannot be overridden by external consumers or CLI flags.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| [SDK-03](#sdk-03) | `EnrollmentError` overloaded with 19 variants spanning 6 concerns | Medium | Actionable | `src/error.rs` |
| [SDK-04](#sdk-04) | `WsStream` type alias leaks implementation details | Medium | Informational | `src/connection.rs` |
| [SDK-05](#sdk-05) | Timeout constants hardcoded with no override mechanism | Low | Informational | `src/ws.rs` |

## Details

### ~~SDK-03~~ **FIXED**

**~~`EnrollmentError` overloaded with 19 variants spanning 6 concerns~~**

**Status:** Resolved. `EnrollmentError` restructured from 19 flat variants into 4 domain sub-enums
(`TlsError`, `IdentityError`, `ProtocolError`, `CaError`) plus 5 top-level variants (`Io`, `Json`,
`WebSocket`, `HttpUri`, `Directory`). Sub-enums use `#[from]` for composition. `impl_report_conversion!`
updated with closure-based forms for leaf-type-to-top-level conversions. All construction sites (~60)
updated across 8 files. Helper methods (`is_receive_closed()`, `is_cert_expired()`) updated to match
new paths.

### SDK-04

**`WsStream` type alias leaks implementation details**

- **Severity:** Medium
- **Type:** Informational
- **File:** `src/connection.rs`

**Description:** The public type alias `WsStream` exposes the exact TLS and transport stack:
`tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>`.
Switching TLS libraries or adding transport options (e.g., Unix sockets for testing) would break all
consumers.

### SDK-05

**Timeout constants hardcoded with no override mechanism**

- **Severity:** Low
- **Type:** Informational
- **File:** `src/ws.rs`

**Description:** `CONNECT_TIMEOUT` (30s), `RESPONSE_TIMEOUT` (60s), and `APPROVAL_TIMEOUT` (30min)
are reasonable defaults but cannot be overridden by external consumers or CLI flags. Environments with
slow networks or manual approval workflows may need different values.
