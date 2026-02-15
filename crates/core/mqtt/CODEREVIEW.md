# Code Review: `uptrakit-mqtt`

**Rating:** GOOD — Two unimplemented message handlers; shared magic strings.

The MQTT service crate handles tenant assignment, MQTT client lifecycle, and controller communication well.
Credentials are properly redacted in debug output. The shutdown/reconnect logic is solid with cancellation
tokens and timeouts. Two controller messages (`CaBundleUpdated`, `RequestCertRenewal`) are acknowledged but
not implemented.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| ~~[CROSS-01](#cross-01)~~ | ~~Magic string WebSocket close reasons~~ **FIXED** | ~~High~~ | ~~Actionable~~ | `src/main.rs` |
| ~~[MQTT-01](#mqtt-01)~~ | ~~`CaBundleUpdated` not implemented~~ **FIXED** | ~~Medium~~ | ~~Actionable~~ | `src/main.rs` |
| ~~[MQTT-02](#mqtt-02)~~ | ~~`RequestCertRenewal` not implemented~~ **FIXED** | ~~Medium~~ | ~~Actionable~~ | `src/main.rs` |
| [MQTT-03](#mqtt-03) | MQTT broker TLS uses system trust only | Low | Informational | `src/mqtt_client.rs` |
| ~~[MQTT-04](#mqtt-04)~~ | ~~No SIGHUP handler (unlike agent)~~ **FIXED** | ~~Low~~ | ~~Informational~~ | `src/main.rs` |
| [CROSS-02](#cross-02) | Protocol version mismatch only warns | Medium | Informational | `src/main.rs` |

## Details

### ~~CROSS-01~~ **FIXED**

**~~Magic string WebSocket close reasons~~**

**Status:** Resolved. Close reason constants from `uptrakit_internal_wire::close_reason` are now used
in both pattern matches and sender sites. See [WIRE-01](../../shared/wire/CODEREVIEW.md#wire-01).

### ~~MQTT-01~~ **FIXED**

**~~`CaBundleUpdated` not implemented~~**

**Status:** Resolved. The MQTT service now persists the updated CA bundle via
`identity.save_ca_cert(&payload.ca_bundle_pem)`, matching the agent's implementation.
`AuthenticatedContext.identity` changed from `&` to `&mut` to support this.

### ~~MQTT-02~~ **FIXED**

**~~`RequestCertRenewal` not implemented~~**

**Status:** Resolved. The MQTT service now implements the full certificate renewal flow: generates
keypair+CSR via `generate_keypair_and_csr()`, sends `RenewCertificate`, handles the `Certificate`
response by saving cert+key and reconnecting with `LoopOutcome::Reconnect`. Matches the agent's
implementation.

### MQTT-03

**MQTT broker TLS uses system trust only**

- **Severity:** Low
- **Type:** Informational
- **File:** `src/mqtt_client.rs:196-201`

**Description:** When the MQTT transport is TLS, the `TlsConfiguration::Simple` is constructed with an empty
`ca` vec. This means the MQTT broker's certificate is validated against the system trust store only. There is
no mechanism to provide a custom CA for the broker connection (separate from the controller CA).

**Code evidence:**

```rust
// crates/core/mqtt/src/mqtt_client.rs:196-201
MqttTransport::Tls => {
    let tls_config = rumqttc::TlsConfiguration::Simple {
        ca: Vec::new(),
        alpn: None,
        client_auth: None,
    };
    opts.set_transport(Transport::Tls(tls_config));
}
```

**Recommendation:** For environments with internal/private MQTT brokers using custom CAs, a future
enhancement could allow the tenant configuration to include a CA bundle for broker TLS validation. The
current system-trust-only approach is acceptable for public brokers and standard enterprise setups.

### ~~MQTT-04~~ **FIXED**

**~~No SIGHUP handler (unlike agent)~~**

**Status:** Resolved. The MQTT service now handles SIGHUP alongside SIGINT/SIGTERM. SIGHUP triggers
a graceful restart (`DisconnectReason::Restart`, `LoopOutcome::Restart`), matching the agent's
behavior. The `handle_graceful_shutdown()` function was refactored to accept `DisconnectReason` and
`LoopOutcome` parameters instead of hardcoding `Shutdown`.

### CROSS-02

**Protocol version mismatch only warns**

- **Severity:** Medium
- **Type:** Informational
- **File:** `src/main.rs:396-402`
- **Also affects:** `crates/core/agent/src/client.rs:228-234`
- **Related finding:** [CROSS-02 in agent](../agent/CODEREVIEW.md#cross-02)

**Description:** When `ServiceSettings.protocol_version` does not match `PROTOCOL_VERSION`, the MQTT service
logs a warning but continues operating. This is the same deliberate trade-off as the agent: rolling upgrades
are supported without hard disconnects.

**Code evidence:**

```rust
// crates/core/mqtt/src/main.rs:396-402
if settings.protocol_version != uptrakit_internal_wire::PROTOCOL_VERSION {
    tracing::warn!(
        reported = settings.protocol_version,
        expected = uptrakit_internal_wire::PROTOCOL_VERSION,
        "controller protocol version mismatch"
    );
}
```

**Recommendation:** Same as for the agent. Revisit when protocol version 2 introduces breaking changes.

## Extensibility Assessment

The MQTT crate is the **best reference implementation** for external service developers. It demonstrates
the complete service lifecycle (enroll, authenticate, handle messages, graceful shutdown) with a minimal
dependency footprint. Its only weakness as a template is the duplicated enrollment boilerplate.

### MQTT-05

**Enrollment boilerplate duplicated with agent**

- **Severity:** Medium
- **Type:** Informational
- **File:** `src/main.rs`
- **Also affects:** `crates/core/agent/src/main.rs`
- **Related finding:** [SDK-02](../../shared/service-sdk/CODEREVIEW.md#sdk-02),
  [AGENT-02](../agent/CODEREVIEW.md#agent-02)

**Description:** ~200 lines of enrollment/reconnection boilerplate are duplicated between the MQTT
service and the agent. This is the strongest signal that the service-sdk should provide a higher-level
lifecycle abstraction. An external developer building a new service would create a third copy.

### ~~MQTT-06~~ **FIXED**

**~~`MqttTransport` type mapping is fragile~~**

**Status:** Resolved. `MqttTransport` consolidated into `shared-types` with feature-gated derives.
The manual `local_mqtt_transport()` mapping function removed from `tenant_manager.rs`.
