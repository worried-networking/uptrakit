# Code Review: `uptrakit-mqtt`

Reviewed: `src/main.rs` (343 lines), `src/mqtt_client.rs` (439 lines),
`src/tenant_manager.rs` (311 lines), `src/cli.rs` (157 lines), `Cargo.toml`.

## Summary

The MQTT crate is clean and well-structured with proper credential redaction,
hash-based config change detection, and graceful shutdown. It serves as a
good example of a service built with the SDK. Key issues are limited TLS
configuration for MQTT broker connections, a missing handler for
`MqttClientCreated`, and no proactive certificate renewal timer.

## Role in the Architecture

The MQTT service is a standalone binary that bridges Uptrakit with MQTT
brokers and Home Assistant. It uses the service SDK for lifecycle management
and the wire protocol for controller communication.

**Clean dependency chain** -- depends on `uptrakit-service-sdk`,
`uptrakit-internal-wire`, `uptrakit-build-info`, and
`uptrakit-shared-macros`. No provider, database, or web-api dependencies.
Validates the service SDK's extensibility: if a new service type follows the
same pattern, it would have a similarly clean dependency chain.

## Findings

### Medium

#### M1: TLS configuration uses empty CA — no pinned CA support

**File:** `src/mqtt_client.rs:196-201`

When TLS transport is configured, the MQTT client uses
`TlsConfiguration::Simple { ca: Vec::new() }`, which relies on the system
trust store. This does not support pinned CA certificates for private MQTT
brokers using internal CAs.

```rust
MqttTransport::Tls => {
    let tls_config = rumqttc::TlsConfiguration::Simple {
        ca: Vec::new(),
        alpn: None,
        client_auth: None,
    };
    opts.set_transport(Transport::Tls(tls_config));
}
```

**Recommendation:** Consider extending `MqttTenantConfig` (in wire) and
`MqttConfig` with an optional CA PEM field for brokers using private CAs.
Alternatively, use `rumqttc::TlsConfiguration::Rustls` with a custom
`ClientConfig` built via the service-sdk's TLS utilities.

#### M2: `MqttClientCreated` message not handled

**File:** `src/main.rs:241-243`

The `ControllerMessage::MqttClientCreated` variant exists in the wire
protocol (wire `src/lib.rs:139`) but is not explicitly handled in the
MQTT authenticated loop. It falls through to the wildcard `Some(_)` arm.

```rust
Some(_) => {
    tracing::debug!("ignoring unrecognized message in authenticated loop");
}
```

If the controller sends `MqttClientCreated` to notify the MQTT service of
a new client to lease, this message is silently ignored.

**Recommendation:** Either handle `MqttClientCreated` by fetching the
client configuration (e.g., via a follow-up request or by expecting a
subsequent `TenantAssignments`), or document that this message is handled
server-side only and the MQTT service receives assignments via
`TenantAssignments`.

#### M3: No proactive certificate renewal timer

**File:** `src/main.rs:142-320`

Unlike the agent (`crates/core/agent/src/client.rs:94-95, 161-178`), the
MQTT service does not set up a renewal timer based on
`ServiceSettings.renewal_window_hours`. It only responds to
`RequestCertRenewal` from the controller.

This means certificate renewal depends entirely on the controller sending
a `RequestCertRenewal` message. If the controller fails to send this
message (e.g., controller restart during the renewal window), the MQTT
service's certificate may expire.

**Recommendation:** Add a renewal timer matching the agent's implementation
to ensure the MQTT service can proactively renew certificates.

### Low

#### L1: Credentials stored as plain `String` in `MqttConfig`

**File:** `src/mqtt_client.rs:24-26`, `src/tenant_manager.rs:164-171`

`MqttConfig.username` and `MqttConfig.password` are `Option<String>`, while
the wire protocol uses `SecretString`. The `build_config_from_wire` function
(tenant_manager.rs:164-171) calls `.expose_secret().to_string()` to extract
the plaintext.

While the custom `Debug` impl (mqtt_client.rs:31-43) properly redacts these
fields in log output, holding credentials as plain `String` means they may
appear in heap dumps or core dumps.

**Recommendation:** Consider keeping `SecretString` (or `Zeroizing<String>`)
in `MqttConfig` and only exposing the secret when passing to
`MqttOptions::set_credentials()`.

#### L2: `compute_config_hash` uses `DefaultHasher` (non-deterministic)

**File:** `src/tenant_manager.rs:177-188`

`DefaultHasher` uses `SipHash` which is randomized per process (different
seed each run). This is fine for within-process change detection (the
intended use), but worth noting that hashes are not comparable across
process restarts.

**Recommendation:** No change needed — the hash is only used for
same-process config dedup. Add a brief comment noting this.

### Info

#### I1: Credential redaction in `Debug` is thorough

**File:** `src/mqtt_client.rs:31-43`

The custom `Debug` impl always shows `[REDACTED]` for both `username` and
`password`, regardless of whether they are `Some` or `None`. Tests
(lines 339-395) verify this behavior comprehensively, including the case
where fields are `None`.

#### I2: Platform-portable signal handling

**File:** `src/main.rs:180-312`

SIGTERM and SIGHUP use `#[cfg(unix)]` / `#[cfg(not(unix))]` guards with
`futures_util::future::pending()` fallbacks. This is the correct pattern
and should be replicated in the agent crate.

#### I3: Hash-based config change detection

**File:** `src/tenant_manager.rs:91-101`

`start_or_update_client` computes a hash of relevant config fields and
skips restart if unchanged. The hash excludes `enabled`, `mqtt_client_id`,
`tenant_id`, and `updated_at` (line 305 test confirms `updated_at` is
excluded). This avoids unnecessary MQTT reconnections.

#### I4: `FuturesUnordered` for parallel shutdown

**File:** `src/tenant_manager.rs:75-88`

`shutdown_all` uses `FuturesUnordered` to shut down all MQTT clients
concurrently rather than sequentially. Good practice for minimizing
shutdown time.

#### I5: Lease-based multi-instance tenant distribution

Multiple MQTT service instances can run concurrently with coordinated
tenant assignment. This validates the SDK's extensibility for multi-instance
service types.

#### I6: Test coverage is solid

Tests cover MQTT options configuration, credential handling, TLS transport,
shutdown timeout behavior, status topic formatting, wire config conversion,
default port fallback, and hash stability.
