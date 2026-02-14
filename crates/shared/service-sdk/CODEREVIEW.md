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

1. **`EnrollmentError` is overloaded**: 19 variants spanning I/O, TLS, key generation, WebSocket, enrollment
   protocol, identity state, and CA operations in a single flat enum. External consumers cannot distinguish
   "network is down" from "certificate expired" from "enrollment rejected" without matching individual
   variants. Consider splitting into sub-error enums.

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

### SDK-03

**`EnrollmentError` overloaded with 19 variants spanning 6 concerns**

- **Severity:** Medium
- **Type:** Actionable
- **File:** `src/error.rs`

**Description:** The `EnrollmentError` enum covers I/O, TLS/certificates, key/CSR generation,
WebSocket/HTTP, enrollment protocol, identity state, and CA operations in a single flat enum. External
consumers cannot distinguish error categories (e.g., "network is down" vs "cert expired" vs "enrollment
rejected") without matching individual variants.

**Recommendation:** Consider splitting into sub-enums (`TlsError`, `IdentityError`, `ProtocolError`,
`CaError`) and using `#[from]` or `context_to()` for composition.

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
