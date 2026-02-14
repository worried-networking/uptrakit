# Code Review: `uptrakit-mqtt`

**Rating:** GOOD — Two unimplemented message handlers; shared magic strings.

The MQTT service crate handles tenant assignment, MQTT client lifecycle, and controller communication well.
Credentials are properly redacted in debug output. The shutdown/reconnect logic is solid with cancellation
tokens and timeouts. Two controller messages (`CaBundleUpdated`, `RequestCertRenewal`) are acknowledged but
not implemented.

## Findings

| ID | Title | Severity | Type | File |
|---|---|---|---|---|
| [CROSS-01](#cross-01) | Magic string WebSocket close reasons | High | Actionable | `src/main.rs` |
| [MQTT-01](#mqtt-01) | `CaBundleUpdated` not implemented | Medium | Actionable | `src/main.rs` |
| [MQTT-02](#mqtt-02) | `RequestCertRenewal` not implemented | Medium | Actionable | `src/main.rs` |
| [MQTT-03](#mqtt-03) | MQTT broker TLS uses system trust only | Low | Informational | `src/mqtt_client.rs` |
| [MQTT-04](#mqtt-04) | No SIGHUP handler (unlike agent) | Low | Informational | `src/main.rs` |
| [CROSS-02](#cross-02) | Protocol version mismatch only warns | Medium | Informational | `src/main.rs` |

## Details

### CROSS-01

**Magic string WebSocket close reasons**

- **Severity:** High
- **Type:** Actionable
- **File:** `src/main.rs:420-437`
- **Also affects:** `crates/core/agent/src/client.rs:421-438`, `crates/ui/web-api/src/routes/mqtt_ws.rs`
- **Related finding:** [WIRE-01](../../shared/wire/CODEREVIEW.md#wire-01),
  [CROSS-01 in agent](../agent/CODEREVIEW.md#cross-01)

**Description:** The close reason pattern match in the MQTT service is identical to the agent's. The same
bare string literals (`"certificate rotated"`, `"certificate revoked"`) are matched against controller-sent
values with no compile-time safety.

**Code evidence:**

```rust
// crates/core/mqtt/src/main.rs:420-437
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

**Recommendation:** See [WIRE-01](../../shared/wire/CODEREVIEW.md#wire-01) for the proposed constants module.
Both service crates should import and match on these constants.

### MQTT-01

**`CaBundleUpdated` not implemented**

- **Severity:** Medium
- **Type:** Actionable
- **File:** `src/main.rs:404-407`

**Description:** The `CaBundleUpdated` message is received and logged but the payload is discarded. The
agent handles this message by fetching the updated CA bundle and persisting it. The MQTT service should do
the same to stay in sync when the controller rotates its CA.

**Code evidence:**

```rust
// crates/core/mqtt/src/main.rs:404-407
Some(ControllerMessage::CaBundleUpdated(payload)) => {
    tracing::info!("received CA bundle update from controller");
    let _ = payload;
}
```

**Recommendation:** Implement CA bundle persistence using the same pattern as the agent: compute the local
CA hash, compare with the received hash, fetch the new bundle if stale, and persist via
`identity.save_ca_cert()`. The `ServiceIdentityState` from service-sdk already provides `save_ca_cert()`.

### MQTT-02

**`RequestCertRenewal` not implemented**

- **Severity:** Medium
- **Type:** Actionable
- **File:** `src/main.rs:408-410`

**Description:** The controller can push a `RequestCertRenewal` message to trigger proactive certificate
renewal before expiry. The MQTT service logs this but takes no action. Without this, the MQTT service relies
solely on its own expiry-based detection to renew certificates.

**Code evidence:**

```rust
// crates/core/mqtt/src/main.rs:408-410
Some(ControllerMessage::RequestCertRenewal(_)) => {
    tracing::info!("certificate renewal requested (not yet implemented for MQTT)");
}
```

**Recommendation:** Implement certificate renewal using the service-sdk's `generate_keypair_and_csr()`
followed by sending `ServiceMessage::RenewCertificate` and handling the `Certificate` response. The agent's
renewal flow in `client.rs` can serve as a reference. This should result in a `LoopOutcome::Reconnect` after
the new certificate is saved.

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

### MQTT-04

**No SIGHUP handler (unlike agent)**

- **Severity:** Low
- **Type:** Informational
- **File:** `src/main.rs:362-364`

**Description:** The MQTT service handles SIGINT and SIGTERM but not SIGHUP. The agent treats SIGHUP as a
graceful restart signal (disconnects with `DisconnectReason::Restart` and exits for systemd to restart it).
The MQTT service lacks this capability.

**Code evidence:**

```rust
// crates/core/mqtt/src/main.rs:362-364
#[cfg(unix)]
let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    .context_to::<AppError>()?;
```

Compare with the agent:

```rust
// crates/core/agent/src/client.rs:100-101
let mut sighup = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
    .context_to::<Error>()?;

// crates/core/agent/src/client.rs:462-470
_ = sighup.recv() => {
    tracing::info!("received SIGHUP, initiating graceful restart");
    break handle_graceful_shutdown(
        &mut conn,
        in_flight_update.take(),
        shutdown_timeout_seconds,
        DisconnectReason::Restart,
        LoopOutcome::Restart,
    ).await;
}
```

**Recommendation:** Consider adding SIGHUP handling for operational parity. SIGHUP is useful for triggering
a clean reconnect cycle (e.g., after a configuration change) without a full process restart via SIGTERM.

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

### MQTT-06

**`MqttTransport` type mapping is fragile**

- **Severity:** Low
- **Type:** Informational
- **File:** `src/tenant_manager.rs:150-161`
- **Related finding:** [WIRE-04](../../shared/wire/CODEREVIEW.md#wire-04)

**Description:** `local_mqtt_transport()` manually maps `uptrakit_internal_wire::MqttTransport` to
`uptrakit_web_api_types::mqtt_transport::MqttTransport` variant by variant. If a new transport type is
added (e.g., `WebSocket`), this function will fail to compile only if the match is exhaustive. The two
`MqttTransport` enums should be unified into a single type in `shared-types`, or a `From` impl should
be provided.
