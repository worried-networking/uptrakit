# Home Assistant and MQTT Integration

Uptrakit integrates with MQTT brokers to publish software version state. Once an MQTT client is
configured and connected, Uptrakit publishes the installed and latest versions for every tracked software
item to the broker automatically. Home Assistant Discovery is an optional layer on top that creates
`update` entities in Home Assistant — one per tracked software item per host — so you can view versions
and trigger updates from the Home Assistant UI.

## MQTT State Topics (always active)

Once an MQTT client is **enabled** and connected to the broker, Uptrakit publishes the following retained
topics for every `(software item, host)` pair regardless of whether Home Assistant Discovery is enabled:

| Topic | Purpose |
| --- | --- |
| `{prefix}/update/{item_id}/{host_id}/state` | Installed version string (empty if unknown) |
| `{prefix}/update/{item_id}/{host_id}/latest_version` | Latest available version string (empty if unknown) |
| `{prefix}/update/{item_id}/{host_id}/set` | Command topic — publish `"install"` to trigger an update |

Where `{prefix}` is the **Topic Prefix** configured on the MQTT client (default: `uptrakit`).

These topics update automatically whenever a version check completes or an update finishes. No extra
configuration is required.

## Prerequisites

Before enabling Home Assistant Discovery:

1. **An MQTT broker** is reachable by both Uptrakit and Home Assistant (e.g. Mosquitto).
2. **An MQTT client** is configured in Uptrakit under **Settings > MQTT Clients** and the MQTT service is
   running and connected to the broker.
3. **Home Assistant** has the [MQTT integration](https://www.home-assistant.io/integrations/mqtt/)
   enabled and pointed at the same broker.
4. At least one **agent** is connected and has performed a version check for the software items you want
   to track. Items whose versions are not yet known will appear but show no version data until the first
   version check completes.

## Enabling Home Assistant Discovery

Discovery is configured per MQTT client:

1. Go to **Settings > MQTT Clients**.
2. Create a new client or click **Edit** on an existing one.
3. Enable the **Home Assistant Discovery** toggle.
4. Optionally change the **Discovery Prefix** (default: `homeassistant`). This must match the MQTT
   Discovery prefix configured in Home Assistant (default is also `homeassistant`).
5. Save the configuration.

You can also enable discovery via the CLI:

```sh
# Create a new MQTT client with HA discovery enabled
uptrakit settings mqtt create \
  --host broker.local --port 1883 \
  --client-id uptrakit-ha \
  --topic-prefix uptrakit \
  --ha-discovery \
  --ha-discovery-prefix homeassistant

# Enable HA discovery on an existing client
uptrakit settings mqtt update <id> --ha-discovery
```

## What Entities Are Created

Uptrakit creates one Home Assistant `update` entity for every `(software item, host)` combination where:

- The software item is **enabled** and not deactivated.
- The software item's discovery state is `null` (manual) or `approved` (auto-discovered and reviewed).
- The host is **active** (not deactivated) and assigned to the software item.

Each entity displays:

| Attribute | Description |
| --- | --- |
| Installed version | The currently installed version on the host (blank if unknown) |
| Latest version | The newest available version (blank if unknown) |
| Update available | `true` when latest > installed |

Entity IDs follow the pattern: `update.uptrakit_<tenant_id>_<software_item_id>_<host_id>`.

All entities share a single HA device named **Uptrakit** (identified by `uptrakit_<tenant_id>`).

## Triggering Updates from Home Assistant

When an update is available, an **Install** button appears on the entity card. Pressing it sends an MQTT
command to Uptrakit via the command topic for that entity.

Uptrakit validates the request and, if accepted, creates an `update_history` record with:

- `actor_type = "mqtt"`
- `actor_id = <mqtt_client_id>`

The update is then dispatched to the appropriate agent exactly as if it had been triggered from the web UI
or CLI. Progress is not streamed back to Home Assistant — the update entity state will refresh the next
time Uptrakit pushes updated version data (after the update completes or fails).

> **Note:** Uptrakit never triggers updates automatically. Update execution always requires an explicit
> user action — from the web UI, CLI, or Home Assistant.

## Reconnect Resilience

If the MQTT broker restarts or the connection is interrupted, Uptrakit automatically republishes all
state and version topics (and Home Assistant discovery configs, if HA Discovery is enabled) when the
connection is re-established. This ensures the broker always has current retained data after network
events.

If Home Assistant restarts while the broker and Uptrakit remain connected, Uptrakit detects the HA birth
message (`online` on `{ha_discovery_prefix}/status`) and immediately republishes all HA discovery configs
so HA picks them up fresh. State and version topics remain retained on the broker and do not need
re-sending. This follows the standard [HA MQTT birth/will pattern](https://www.home-assistant.io/integrations/mqtt/#birth-and-last-will-messages).

## Why Some Software Items May Not Appear

| Symptom | Cause |
| --- | --- |
| Item not visible in HA | Software item is `pending` (auto-discovered, not yet approved) |
| Item not visible in HA | Software item or host is deactivated |
| Entity shows blank versions | Version check has not run yet since the MQTT client was configured |
| Entity shows blank versions | No agent is connected for that host |
| No entities at all | HA discovery not enabled on the MQTT client |
| No entities at all | MQTT service is not connected to the broker |
| No entities at all | HA MQTT integration not configured for the same broker |
| State topics absent on broker | MQTT client not enabled or MQTT service not running |

## Custom CA Certificates for Private Brokers

When connecting to MQTT brokers that use a private or internal certificate authority (e.g. self-signed
certificates or enterprise CAs), you can provide a custom CA certificate in PEM format.

This is configured per MQTT client in **Settings > MQTT Clients**:

1. Create or edit an MQTT client.
2. Set the transport to **TLS**.
3. Paste the CA certificate PEM into the **CA Certificate (PEM)** field.
4. Save the configuration.

The CA certificate is encrypted at rest using AES-256-GCM and transmitted to the MQTT service via the
wire protocol. The MQTT service uses the provided PEM as the trusted CA for TLS connections instead of the
system trust store. See [Secrets and Encryption](../security/secrets-and-encryption.md) for details.

If no custom CA is provided, the MQTT service falls back to the system trust store (default behavior for
public brokers).

The CLI supports this via `--ca-pem <PEM_STRING>` (inline) or `--ca-pem-file <PATH>` (from file) on
the `settings mqtt create` and `settings mqtt update` subcommands.

## Security Notes

- MQTT credentials (username, password, CA certificate) are stored encrypted at rest using AES-256-GCM.
  See [Secrets and Encryption](../security/secrets-and-encryption.md).
- The MQTT command topics that accept Install commands are scoped per `(software item, host)` and are
  accessible to anyone with publish access to the broker. Ensure your broker uses authentication and
  access control to limit who can publish to Uptrakit command topics.
- Update requests received via MQTT are validated by the controller (same checks as REST API triggers):
  - Tenant scope verification (the MQTT client must be assigned the same tenant as the software item).
  - Host assignment and agent approval checks.
  - Duplicate update prevention (no two concurrent updates for the same `(host, software item)` pair).

## Troubleshooting

**Discovery topics are published but Home Assistant doesn't see any entities:**

- Confirm HA MQTT integration is pointed at the same broker.
- Confirm the discovery prefix in Uptrakit matches HA's MQTT discovery prefix (default `homeassistant`).
- In HA, go to **Settings > Devices & Services > MQTT** and check whether discovery is enabled.

**Install button not visible:**

- The Install button only appears when `latest_version > installed_version`. Verify that the version
  check has run and the latest version is populated (visible in the Uptrakit web UI under the software
  item's version list).

**MQTT service is connected but no version data is pushed:**

- Check that at least one agent has reported version check results for the relevant software items.
- Manually trigger a version check: `uptrakit check --item <id>` or from the web UI.
- After the check completes, Uptrakit automatically pushes updated states to connected MQTT services.

**Update command received but nothing happens:**

- Check the Uptrakit controller logs for `mqtt_trigger_update` errors.
- Verify the agent for the target host is connected (`uptrakit services list` or the web UI).
- Check whether an update is already pending for that `(host, software item)` pair in the update history.
