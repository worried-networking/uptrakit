# Code Review: `uptrakit-internal-wire`

**Rating:** EXCELLENT — Minor spec imprecision and code duplication.

The wire crate is the single source of truth for the service-controller protocol. All message types are
well-documented with serde attributes for correct serialization. The `PROTOCOL_VERSION` constant, timestamp
helpers, and `UtcDateTime` serde module are clean and correct. The AsyncAPI spec is thorough and
well-maintained.

## Extensibility Assessment

The wire crate has several extensibility-related concerns:

1. **Monolithic single-file design**: The entire wire protocol (~800+ lines of production code) lives in a
   single `lib.rs`. Splitting into logical modules (`envelopes.rs`, `payloads/`, `enums.rs`) would improve
   navigability for external developers building new services.

2. **Type duplication across crates**: `ServiceType` is defined here, in `web-api-types`, and in `shared-db`
   (three copies). `MqttTransport` exists here and in `web-api-types` (two copies). These should be
   consolidated into `shared-types` with feature-gated derives.

3. **Closed message enums**: `ServiceMessage` and `ControllerMessage` are closed enums without
   `#[non_exhaustive]`. Adding a new message type is a breaking change. External developers building new
   service types cannot extend the protocol.

4. **Private utility modules**: The `utc_datetime_millis` serde module is private but useful for external
   consumers building API clients.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| ~~[WIRE-01](#wire-01)~~ | ~~No close reason constants defined~~ **FIXED** | ~~High~~ | ~~Actionable~~ | `src/lib.rs` |
| [WIRE-02](#wire-02) | Duplicate string parsing path for `HookShell` | Low | Actionable | `src/lib.rs` |
| ~~[WIRE-03](#wire-03)~~ | ~~AsyncAPI `active_mqtt_clients` items typed `string` not `uuid`~~ **FIXED** | ~~Low~~ | ~~Actionable~~ | `asyncapi.yaml` |

## Details

### ~~WIRE-01~~ **FIXED**

**~~No close reason constants defined~~**

**Status:** Resolved. A `close_reason` module with 12 named constants added to the wire crate. All
sender sites (web-api route handlers) and receiver sites (agent, MQTT) updated to use these constants.

### WIRE-02

**Duplicate `HookShell` string parsing path** -- **RESOLVED**

- **Severity:** Low
- **Type:** Actionable
- **File:** `src/lib.rs`
- **Status:** Resolved -- `parse()` removed, `FromStr` is now self-contained with typed `ParseHookShellError`.

### ~~WIRE-03~~ **FIXED**

**~~AsyncAPI `active_mqtt_clients` items typed `string` not `uuid`~~**

**Status:** Resolved. `format: uuid` added to array items in `disconnectingPayload.active_mqtt_clients`,
`mqttRegisterPayload.active_mqtt_clients`, and `mqttReleaseTenantsPayload.mqtt_client_ids`.

**Original description:** The `active_mqtt_clients` array in both `disconnecting` and `mqttRegister` message schemas
declared items as `type: string`. In the Rust code, these are `Vec<uuid::Uuid>`, and the values are always
UUIDs. The AsyncAPI spec now reflects this with `format: uuid`.

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

### ~~WIRE-04~~ **FIXED**

**~~`ServiceType` duplicated across three crates~~**

**Status:** Resolved. `ServiceType`, `ServiceStatus`, `MqttTransport`, and `HookShell` moved to
`shared-types` with feature-gated derives (`sea-orm`, `openapi`). Wire crate re-exports from `shared-types`.
All duplicates eliminated.

### ~~WIRE-05~~ **FIXED**

**~~`ServiceMessage` and `ControllerMessage` lack `#[non_exhaustive]`~~**

**Status:** Resolved. `#[non_exhaustive]` added to both `ServiceMessage` and `ControllerMessage` enums.
All exhaustive match expressions in external crates updated with wildcard `_ =>` arms.
