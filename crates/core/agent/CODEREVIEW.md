# Code Review: `uptrakit-agent`

**Rating:** EXCELLENT — Production-ready; minor improvements possible.

The agent crate is well-structured with clean separation between connection management, host info collection,
and update execution. Error handling follows project conventions (rootcause + thiserror). Signal handling
covers SIGINT, SIGTERM, and SIGHUP. The reconnection logic with backoff is robust.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| [CROSS-01](#cross-01) | Magic string WebSocket close reasons | High | Actionable | `src/client.rs` |
| [CROSS-02](#cross-02) | Protocol version mismatch only warns | Medium | Informational | `src/client.rs` |
| [AGENT-01](#agent-01) | `ioreg` UUID parsing is brittle | Low | Informational | `src/host_info.rs` |

## Details

### CROSS-01

**Magic string WebSocket close reasons**

- **Severity:** High
- **Type:** Actionable
- **File:** `src/client.rs:421-438`
- **Also affects:** `crates/core/mqtt/src/main.rs:420-437`, `crates/ui/web-api/src/routes/service_ws.rs`,
  `crates/ui/web-api/src/routes/agent_ws.rs`, `crates/ui/web-api/src/routes/mqtt_ws.rs`
- **Related finding:** [WIRE-01](../../shared/wire/CODEREVIEW.md#wire-01)

**Description:** Close reason strings like `"certificate rotated"` and `"certificate revoked"` are used as
bare string literals in pattern matches. The controller sends these strings from `close_with_reason()` in
`service_ws.rs`. Any typo on either side silently degrades to the catch-all arm.

**Code evidence:**

```rust
// crates/core/agent/src/client.rs:421-438
match conn.close_reason() {
    Some("certificate rotated") => {
        tracing::info!("connection closed: certificate rotated");
        break LoopOutcome::Reconnect;
    }
    Some("certificate revoked") => {
        tracing::warn!("connection closed: certificate revoked");
        break LoopOutcome::Disconnected;
    }
    // ...
}
```

The controller side uses the same bare strings:

```rust
// crates/ui/web-api/src/routes/agent_ws.rs:281
let _ = close_with_reason(sink, "certificate rotated").await;
// crates/ui/web-api/src/routes/service_ws.rs:484
let _ = close_with_reason(&mut sink, "certificate revoked").await;
```

At least 12 distinct close reasons exist across all controller handlers: `"certificate rotated"`,
`"certificate revoked"`, `"no valid certificate"`, `"internal error"`, `"certificate not recognized"`,
`"service deactivated"`, `"service not approved"`, `"service not found"`, `"enrollment timeout"`,
`"agent version too old"`, `"superseded by new connection"`, `"rate limit exceeded"`.

**Recommendation:** Define typed constants in the wire crate (see [WIRE-01](../../shared/wire/CODEREVIEW.md#wire-01))
and use them on both sender and receiver sides:

```rust
// In uptrakit-internal-wire:
pub mod close_reason {
    pub const CERTIFICATE_ROTATED: &str = "certificate rotated";
    pub const CERTIFICATE_REVOKED: &str = "certificate revoked";
    // ...
}
```

### CROSS-02

**Protocol version mismatch only warns**

- **Severity:** Medium
- **Type:** Informational
- **File:** `src/client.rs:228-234`
- **Also affects:** `crates/core/mqtt/src/main.rs:396-402`

**Description:** When `ServiceSettings.protocol_version` does not match `PROTOCOL_VERSION`, both the agent
and MQTT service log a warning but continue operating normally. This is a deliberate trade-off to allow
rolling upgrades where controller and services may temporarily run different versions.

**Code evidence:**

```rust
// crates/core/agent/src/client.rs:228-234
if settings.protocol_version != uptrakit_internal_wire::PROTOCOL_VERSION {
    tracing::warn!(
        reported = settings.protocol_version,
        expected = uptrakit_internal_wire::PROTOCOL_VERSION,
        "controller protocol version mismatch"
    );
}
```

**Recommendation:** When protocol version 2 is introduced, revisit this to decide whether a hard disconnect
is appropriate for incompatible versions. The current warn-and-continue approach is correct for v1 where all
deployed builds share the same protocol.

### AGENT-01

**`ioreg` UUID parsing is brittle**

- **Severity:** Low
- **Type:** Informational
- **File:** `src/host_info.rs:37-44`

**Description:** The macOS machine ID is extracted by finding the line containing `"IOPlatformUUID"` and
then splitting on `"` to take the 4th segment (`nth(3)`). This works for the expected `ioreg` output format
but would silently fall through to `"unknown"` if Apple changes the output format.

**Code evidence:**

```rust
// crates/core/agent/src/host_info.rs:37-44
for line in stdout.lines() {
    if line.contains("IOPlatformUUID") {
        // Line format: "IOPlatformUUID" = "XXXXXXXX-XXXX-..."
        if let Some(uuid) = line.split('"').nth(3) {
            return uuid.to_string();
        }
    }
}
```

**Recommendation:** The current approach is pragmatic and the comment documents the expected format. The
fallback to `"unknown"` is safe. If more robustness is desired, a regex or the
`system_profiler SPHardwareDataType -json` approach could be used, but the added complexity is unlikely to be
worth it given that `ioreg` output has been stable for decades.
