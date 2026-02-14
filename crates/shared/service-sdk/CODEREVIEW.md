# Code Review: `uptrakit-service-sdk`

**Rating:** EXCELLENT — Exemplary; optional base64 cleanup.

The service-sdk crate provides a clean, unified abstraction for both agents and MQTT services: identity
management, TLS configuration, CA bootstrap, enrollment WebSocket flows, and the `ControllerConnection`
wrapper with sequence-validated messaging. File permissions are correctly set to 0o600/0o700. The enrollment
secret is properly cleared after certificate issuance. Test coverage is thorough, including permission
validation on Unix.

## Extensibility Assessment

The service-sdk is the primary entry point for external developers building new services. While it provides
all the building blocks (enrollment, identity, TLS, WebSocket), it has significant extensibility gaps:

1. **No `ServiceHandler` trait or callback mechanism**: Building a new service requires deep knowledge of the
   enrollment flow, the `ControllerConnection::recv()` loop, which `ServiceMessage` variants to send and
   when, and which `ControllerMessage` variants to handle. A trait like:

   ```rust
   pub trait ServiceHandler {
       async fn on_connected(&mut self, conn: &mut ControllerConnection) -> Result<()>;
       async fn on_message(&mut self, msg: ControllerMessage, conn: &mut ControllerConnection) -> Result<()>;
       async fn on_disconnected(&mut self, reason: Option<&str>) -> Result<()>;
   }
   ```

   with a default `run()` function handling the enrollment + reconnection + message loop would dramatically
   improve external extensibility.

2. **Enrollment boilerplate duplicated between agent and MQTT**: ~200 lines of nearly identical code
   (`do_enrollment()`, `run_authenticated_with_reconnect()`, URL parsing, directory resolution, identity
   loading, CA bootstrap, certificate expiry checking) are duplicated in both binaries. This is the strongest
   signal that the SDK should provide a higher-level lifecycle abstraction.

3. **`EnrollmentError` is overloaded**: 19 variants spanning I/O, TLS, key generation, WebSocket, enrollment
   protocol, identity state, and CA fetch. External consumers cannot distinguish "network is down" from
   "certificate expired" from "enrollment rejected" without matching individual variants. Consider splitting
   into sub-error enums.

4. **`WsStream` type alias leaks implementation details**: Exposes `tokio_tungstenite::WebSocketStream<
   tokio_rustls::client::TlsStream<tokio::net::TcpStream>>`. Switching TLS libraries or transport would
   break all consumers.

5. **Timeout constants are hardcoded**: `CONNECT_TIMEOUT` (30s), `RESPONSE_TIMEOUT` (60s),
   `APPROVAL_TIMEOUT` (30min) cannot be overridden by external consumers or CLI flags.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| [SDK-01](#sdk-01) | Custom hand-rolled base64 decoder | Low | Actionable | `src/identity.rs` |
| [SDK-02](#sdk-02) | No `ServiceHandler` trait for external service development | High | Actionable | `src/` |
| [SDK-03](#sdk-03) | `EnrollmentError` overloaded with 19 variants spanning 6 concerns | Medium | Actionable | `src/error.rs` |
| [SDK-04](#sdk-04) | `WsStream` type alias leaks implementation details | Medium | Informational | `src/connection.rs` |
| [SDK-05](#sdk-05) | Timeout constants hardcoded with no override mechanism | Low | Informational | `src/ws.rs` |

## Details

### SDK-01

**Custom hand-rolled base64 decoder**

- **Severity:** Low
- **Type:** Actionable
- **File:** `src/identity.rs:529-561`

**Description:** The `pem_to_der()` function uses a hand-rolled base64 decoder (`base64_decode()`) to
extract DER bytes from PEM certificates. While the implementation is correct and tested, the crate already
depends on `x509-parser` (which brings in PEM parsing capabilities) and `rcgen`. Using an established base64
or PEM parsing library would reduce maintenance surface.

**Code evidence:**

```rust
// src/identity.rs:529-561
/// Minimal base64 decoder (standard alphabet, no padding required).
fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    fn val(c: u8) -> Option<u8> {
        TABLE.iter().position(|&b| b == c).map(|p| p as u8)
    }

    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let mut buf: u32 = 0;
        for (i, &b) in chunk.iter().enumerate() {
            buf |= (val(b)? as u32) << (18 - 6 * i);
        }
        match chunk.len() {
            4 => {
                out.push((buf >> 16) as u8);
                out.push((buf >> 8) as u8);
                out.push(buf as u8);
            }
            3 => {
                out.push((buf >> 16) as u8);
                out.push((buf >> 8) as u8);
            }
            2 => {
                out.push((buf >> 16) as u8);
            }
            _ => return None,
        }
    }
    Some(out)
}
```

The `pem_to_der()` function that calls this also manually finds `BEGIN`/`END` markers:

```rust
// src/identity.rs:513-526
fn pem_to_der(pem: &str) -> Option<Vec<u8>> {
    let start_marker = "-----BEGIN CERTIFICATE-----";
    let end_marker = "-----END CERTIFICATE-----";
    let start = pem.find(start_marker)? + start_marker.len();
    let end = pem[start..].find(end_marker)? + start;
    let b64: String = pem[start..end]
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();

    base64_decode(&b64)
}
```

**Recommendation:** Replace `pem_to_der()` and `base64_decode()` with the `pem` crate (already an indirect
dependency via `x509-parser` and `rcgen`) or `x509-parser`'s own PEM parsing:

```rust
fn pem_to_der(pem_str: &str) -> Option<Vec<u8>> {
    let (_, pem) = x509_parser::pem::parse_x509_pem(pem_str.as_bytes()).ok()?;
    Some(pem.contents)
}
```

This eliminates ~50 lines of hand-rolled parsing with no new dependencies. The existing `pem_to_der_basic`
test should continue to pass with the replacement.

### SDK-02

**No `ServiceHandler` trait for external service development**

- **Severity:** High
- **Type:** Actionable
- **File:** (crate-wide)

**Description:** The SDK provides all building blocks (enrollment, identity, TLS, WebSocket) but no
trait or framework to compose them. An external developer building a new service must study the agent
or MQTT source code to understand the enrollment + reconnect + message loop pattern, then manually
replicate ~200 lines of boilerplate. This is the primary barrier to external service development.

The agent and MQTT service have ~80% identical code for: URL parsing, directory resolution, identity
loading, force-enroll check, CA bootstrap, certificate expiry check, enrollment with backoff, and
reconnection loop with backoff.

**Recommendation:** Introduce a `ServiceHandler` trait and a `run_service_lifecycle()` function:

```rust
pub trait ServiceHandler: Send + Sync {
    fn service_type(&self) -> ServiceType;
    async fn on_connected(&mut self, conn: &mut ControllerConnection) -> Result<()>;
    async fn on_message(&mut self, msg: ControllerMessage, conn: &mut ControllerConnection) -> Result<()>;
    async fn on_disconnected(&mut self, reason: Option<&str>) -> ReconnectDecision;
}

pub async fn run_service_lifecycle(
    args: CommonServiceArgs,
    handler: impl ServiceHandler,
) -> Result<()> { ... }
```

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
