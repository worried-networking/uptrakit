# Code Review: `uptrakit-internal-wire`

**Rating:** EXCELLENT — Minor spec imprecision and code duplication.

The wire crate is the single source of truth for the service-controller protocol. All message types are
well-documented with serde attributes for correct serialization. The `PROTOCOL_VERSION` constant, timestamp
helpers, and `UtcDateTime` serde module are clean and correct. The AsyncAPI spec is thorough and
well-maintained.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| [WIRE-01](#wire-01) | No close reason constants defined | High | Actionable | `src/lib.rs` |
| [WIRE-02](#wire-02) | `HookShell::parse()` duplicates `FromStr` | Low | Actionable | `src/lib.rs` |
| [WIRE-03](#wire-03) | AsyncAPI `active_mqtt_clients` items typed `string` not `uuid` | Low | Actionable | `asyncapi.yaml` |

## Details

### WIRE-01

**No close reason constants defined**

- **Severity:** High
- **Type:** Actionable
- **File:** `src/lib.rs`
- **Also affects:** `crates/core/agent/src/client.rs:421-438`,
  `crates/core/mqtt/src/main.rs:420-437`,
  `crates/ui/web-api/src/routes/service_ws.rs`,
  `crates/ui/web-api/src/routes/agent_ws.rs`,
  `crates/ui/web-api/src/routes/mqtt_ws.rs`
- **Related findings:** [CROSS-01 in agent](../../core/agent/CODEREVIEW.md#cross-01),
  [CROSS-01 in MQTT](../../core/mqtt/CODEREVIEW.md#cross-01)

**Description:** The wire crate defines message types and error codes as proper Rust types but does not
define the WebSocket close reason strings. These 12+ strings are scattered as magic literals across the
controller (sender) and service (receiver) crates with no shared definition.

Close reasons observed across the codebase:

| Reason string | Sender location |
|---|---|
| `"certificate rotated"` | `agent_ws.rs:281`, `mqtt_ws.rs:356` |
| `"certificate revoked"` | `service_ws.rs:484` |
| `"no valid certificate"` | `service_ws.rs:456` |
| `"internal error"` | `service_ws.rs:461`, `service_ws.rs:500`, `service_ws.rs:531` |
| `"certificate not recognized"` | `service_ws.rs:495` |
| `"service deactivated"` | `service_ws.rs:514` |
| `"service not approved"` | `service_ws.rs:519` |
| `"service not found"` | `service_ws.rs:526` |
| `"enrollment timeout"` | `service_ws.rs:732` |
| `"agent version too old"` | `agent_ws.rs:176`, `agent_ws.rs:198` |
| `"superseded by new connection"` | `agent_ws.rs:567`, `agent_ws.rs:852`, `mqtt_ws.rs:404` |
| `"rate limit exceeded"` | `agent_ws.rs:127`, `agent_ws.rs:664`, `mqtt_ws.rs:91`, `mqtt_ws.rs:241`, `mqtt_ws.rs:460` |

**Recommendation:** Add a `close_reason` module with named constants:

```rust
/// WebSocket close reason strings sent by the controller.
///
/// Used in `CloseFrame::reason` to communicate why a connection was closed.
/// Both the controller (sender) and services (receiver) must use these
/// constants to avoid silent mismatches from typos.
pub mod close_reason {
    pub const CERTIFICATE_ROTATED: &str = "certificate rotated";
    pub const CERTIFICATE_REVOKED: &str = "certificate revoked";
    pub const NO_VALID_CERTIFICATE: &str = "no valid certificate";
    pub const INTERNAL_ERROR: &str = "internal error";
    pub const CERTIFICATE_NOT_RECOGNIZED: &str = "certificate not recognized";
    pub const SERVICE_DEACTIVATED: &str = "service deactivated";
    pub const SERVICE_NOT_APPROVED: &str = "service not approved";
    pub const SERVICE_NOT_FOUND: &str = "service not found";
    pub const ENROLLMENT_TIMEOUT: &str = "enrollment timeout";
    pub const AGENT_VERSION_TOO_OLD: &str = "agent version too old";
    pub const SUPERSEDED: &str = "superseded by new connection";
    pub const RATE_LIMIT_EXCEEDED: &str = "rate limit exceeded";
}
```

Then update all sender and receiver sites to use these constants. Document them in the AsyncAPI spec under
connection lifecycle.

### WIRE-02

**`HookShell::parse()` duplicates `FromStr`**

- **Severity:** Low
- **Type:** Actionable
- **File:** `src/lib.rs:121-137`

**Description:** `HookShell` has both a `parse()` inherent method (returning `Option<Self>`) and a `FromStr`
impl (returning `Result<Self, String>`). The `FromStr` impl delegates to `parse()`, so there is no logic
duplication, but having two parsing entry points with different return types is unnecessary API surface.

**Code evidence:**

```rust
// src/lib.rs:121-128
impl HookShell {
    /// Parses a string into a `HookShell` variant.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "bash" => Some(Self::Bash),
            "sh" => Some(Self::Sh),
            "powershell" => Some(Self::PowerShell),
            _ => None,
        }
    }
}

// src/lib.rs:131-137
impl std::str::FromStr for HookShell {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| format!("unknown shell: {s}"))
    }
}
```

**Recommendation:** Remove the `parse()` inherent method and have callers use `FromStr` (via
`"bash".parse::<HookShell>()`) or add a `TryFrom<&str>` alias. If `Option`-returning parsing is needed in a
hot path, keep `parse()` but mark `FromStr` as the canonical entry point in documentation.

### WIRE-03

**AsyncAPI `active_mqtt_clients` items typed `string` not `uuid`**

- **Severity:** Low
- **Type:** Actionable
- **File:** `asyncapi.yaml:1785-1789`, `asyncapi.yaml:1821-1826`

**Description:** The `active_mqtt_clients` array in both `disconnecting` and `mqttRegister` message schemas
declares items as `type: string`. In the Rust code, these are `Vec<uuid::Uuid>`, and the values are always
UUIDs. The AsyncAPI spec should reflect this with `format: uuid`.

**Code evidence:**

```yaml
# asyncapi.yaml:1785-1789
active_mqtt_clients:
  type: array
  items:
    type: string
  description: List of active MQTT client IDs (MQTT services only, for graceful handoff)

# asyncapi.yaml:1821-1826
active_mqtt_clients:
  type: array
  items:
    type: string
  description: |
    Currently active MQTT client IDs (for reconnect reconciliation).
```

**Recommendation:** Update both occurrences to include `format: uuid`:

```yaml
active_mqtt_clients:
  type: array
  items:
    type: string
    format: uuid
  description: ...
```

This improves documentation accuracy and enables code generators to use typed UUID values.
