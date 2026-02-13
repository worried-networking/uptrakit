# Code Review: `uptrakit-service-sdk`

**Rating:** EXCELLENT — Exemplary; optional base64 cleanup.

The service-sdk crate provides a clean, unified abstraction for both agents and MQTT services: identity
management, TLS configuration, CA bootstrap, enrollment WebSocket flows, and the `ControllerConnection`
wrapper with sequence-validated messaging. File permissions are correctly set to 0o600/0o700. The enrollment
secret is properly cleared after certificate issuance. Test coverage is thorough, including permission
validation on Unix.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| [SDK-01](#sdk-01) | Custom hand-rolled base64 decoder | Low | Actionable | `src/identity.rs` |

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
