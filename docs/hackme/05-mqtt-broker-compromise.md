# ATK-05: MQTT Broker Compromise

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | MQTT integration |
| Prerequisites | Control of the MQTT broker or ability to publish to broker topics |
| STRIDE | Tampering, Denial of Service |

## Attack description

1. The attacker compromises the external MQTT broker configured for Home Assistant
   integration, or gains the ability to publish messages to the broker's topic
   namespace.
2. The attacker publishes to the MQTT command topic
   `{prefix}/update/{software_item_uuid}/{host_uuid}/set` with an "install" payload.
3. The MQTT service's `resolve_update_trigger()` parses the topic, extracts the
   software item ID and host ID, and checks them against its in-memory
   `software_states` cache.
4. If the IDs match known entries, the MQTT service sends a `mqtt_trigger_update`
   message to the controller over the mTLS WebSocket.
5. The controller validates the request and dispatches `execute_update` to the
   appropriate agent, installing the latest known version.

Alternatively, the attacker can:

- **Suppress state updates** by intercepting and dropping retained messages on state
  topics, causing Home Assistant to display stale version information.
- **Inject false state data** by publishing to
  `{prefix}/update/{item_uuid}/{host_uuid}/state` topics with fabricated version
  strings.
- **Capture credentials** if the MQTT connection uses `MqttTransport::Tcp` (plaintext)
  or if the broker's TLS is compromised. MQTT username and password are sent during
  the CONNECT phase.

## Worst-case impact

- **Unauthorized update execution.** The attacker can trigger software updates on any
  managed host for which the MQTT service has cached state. Updates install the
  `latest_version` from the controller's cache (not an attacker-controlled version),
  but unplanned updates can still cause service disruption.
- **Denial of service.** Rapid update triggers can exhaust agent resources, cause
  repeated service restarts via post-update hooks, or saturate the controller's
  update pipeline.
- **Credential exposure.** Plaintext MQTT transport (`MqttTransport::Tcp`) exposes
  broker credentials to network observers. The `username` and `password` fields use
  `SecretString` in memory but are sent as cleartext in the MQTT CONNECT packet
  when TLS is not configured.
- **Home Assistant dashboard manipulation.** Injected state messages cause HA to
  display incorrect version information, potentially hiding available updates or
  showing phantom updates.

## Current mitigations

- **Update targets are validated server-side.** The controller validates the
  `mqtt_trigger_update` request against the database: the software item, host, and
  host assignment must exist and be active, and the tenant ID must match.
- **Version is controller-controlled.** The `to_version` in a triggered update comes
  from the controller's cached `latest_version`, not from the MQTT message payload.
  The attacker cannot specify an arbitrary version to install.
- **Duplicate update prevention.** The controller rejects update triggers if an update
  with status `pending` or `in_progress` already exists for the same
  `(host_id, software_item_id)` pair.
- **TLS support.** `MqttTransport::Tls` is supported with optional custom CA
  certificates for private brokers. Credentials are protected in transit when TLS is
  enabled.
- **Credential delivery is WebSocket-only.** MQTT broker credentials
  (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are never published
  to NATS. They are delivered exclusively over the authenticated mTLS WebSocket
  connection.
- **Topic structure uses UUIDs.** Command topics require two valid UUIDs in the path,
  making random topic guessing impractical without knowledge of the software item
  and host inventory.

## Residual risk

- **No message authenticity on inbound MQTT.** The MQTT service does not verify the
  identity of the publisher. Any entity that can publish to the command topic can
  trigger updates. There is no HMAC signing, payload authentication, or ACL
  enforcement at the application level.
- **Plaintext transport is a valid configuration.** `MqttTransport::Tcp` (no TLS) is
  accepted at runtime without error. The MQTT service will connect and send
  credentials in plaintext if configured to do so.
- **Anonymous broker access.** When `username` is `None`, no credentials are sent —
  anonymous broker access is permitted by configuration.
- **Broker-side ACL is external.** Uptrakit delegates all topic-level access control
  to the MQTT broker (e.g., Mosquitto ACL). A misconfigured broker with permissive
  ACLs exposes the entire command surface.
- **UUID enumeration.** Software item and host UUIDs are UUIDv7 (time-ordered). An
  attacker who can observe any state topic can extract valid UUIDs and use them for
  command topic targeting.

## Recommended improvements

- Implement payload-level authentication (e.g., HMAC signature) on inbound MQTT
  command messages to verify they originate from a legitimate Home Assistant instance.
- Add a configuration option to require TLS for MQTT connections and emit a startup
  error (not just a warning) when plaintext transport is used in production.
- Implement application-level rate limiting on `mqtt_trigger_update` messages per
  tenant to prevent update flood attacks.
- Document recommended MQTT broker ACL configuration that restricts publish access
  on command topics to only the Home Assistant instance.
- Consider adding a confirmation step for MQTT-triggered updates (e.g., requiring
  the controller admin to pre-approve MQTT-triggered updates per software item).

## References

- [Wire Protocol — MQTT messages](../api/wire-protocol.md#mqtt-specific-controller---service)
- [Home Assistant Integration](../end-user/home-assistant-mqtt.md)
- [Notification Subsystem Security](../security/notifications-security.md)
- `crates/core/mqtt/src/mqtt_client.rs` — `MqttConfig`, `build_mqtt_options()`
- `crates/core/mqtt/src/tenant_manager.rs` — `resolve_update_trigger()`
- `crates/core/mqtt/src/ha_discovery.rs` — topic structure and state publishing
