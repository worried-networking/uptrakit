# Code Review: `uptrakit-agent`

**Rating:** EXCELLENT — Production-ready; minor improvements possible.

The agent crate is well-structured with clean separation between connection management, host info collection,
and update execution. Error handling follows project conventions (rootcause + thiserror). Signal handling
covers SIGINT, SIGTERM, and SIGHUP. The reconnection logic with backoff is robust.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| ~~[CROSS-01](#cross-01)~~ | ~~Magic string WebSocket close reasons~~ **FIXED** | ~~High~~ | ~~Actionable~~ | `src/client.rs` |
| [CROSS-02](#cross-02) | Protocol version mismatch only warns | Medium | Informational | `src/client.rs` |
| [AGENT-01](#agent-01) | `ioreg` UUID parsing is brittle | Low | Informational | `src/host_info.rs` |

## Details

### ~~CROSS-01~~ **FIXED**

**~~Magic string WebSocket close reasons~~**

**Status:** Resolved. A `close_reason` module with 12 named constants was added to `uptrakit-internal-wire`.
All sender sites (web-api route handlers) and receiver sites (agent `client.rs`, MQTT `main.rs`) now use
these constants instead of bare string literals.

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

## Extensibility Assessment

The agent serves as a **good reference implementation** for external developers building new services. Its
provider-registry consumption pattern (never bypassing the registry to access provider-core directly) is
exemplary. However, it is not ideal as a template due to enrollment boilerplate duplication.

### AGENT-02

**Enrollment boilerplate duplicated with MQTT service**

- **Severity:** Medium
- **Type:** Informational
- **File:** `src/main.rs`, `src/client.rs`
- **Also affects:** `crates/core/mqtt/src/main.rs`
- **Related finding:** [SDK-02](../../shared/service-sdk/CODEREVIEW.md#sdk-02)

**Description:** `run()`, `do_enrollment()`, and `run_authenticated_with_reconnect()` are structurally
~80% identical to the MQTT service's corresponding functions. This includes URL parsing, directory
resolution, identity loading, force-enroll check, CA bootstrap, certificate expiry checking, enrollment
with backoff, and reconnection with backoff. An external developer building a new service would copy this
boilerplate, creating a third copy.

**Recommendation:** Extract the enrollment-reconnect lifecycle into the service-sdk (see
[SDK-02](../../shared/service-sdk/CODEREVIEW.md#sdk-02)). Both the agent and MQTT service would then only
implement the authenticated message-handling loop.

### AGENT-03

**Hardcoded log directive default**

- **Severity:** Low
- **Type:** Informational
- **File:** `src/main.rs:28`

**Description:** The default log directive `"uptrakit_agent=info"` is hardcoded as a string. An external
developer copying this pattern for their own service must remember to change the string. The service-sdk
should provide a helper that derives the module directive from the crate name.
