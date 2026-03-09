# Home Assistant and MQTT Integration

Uptrakit integrates with MQTT brokers to publish software version state. Once an MQTT client is
configured and connected, Uptrakit publishes the installed and latest versions for every tracked software
item to the broker automatically. Home Assistant Discovery is an optional layer on top that creates
`update` entities in Home Assistant — one per tracked software item per host — so you can view versions
and trigger updates from the Home Assistant UI.

Uptrakit also publishes **per-host summary entities**: a single `update` entity per host summarising
the overall state of all non-featured software items (APT packages, Homebrew formulae, npm globals,
etc.) on that host. See [Host Summary Update Entities](#host-summary-update-entities) below.

## MQTT State Topics (always active)

Once an MQTT client is **enabled** and connected to the broker, Uptrakit publishes the following retained
topics for every `(software item, host)` pair regardless of whether Home Assistant Discovery is enabled:

| Topic | Retained | Purpose |
| --- | :---: | --- |
| `{prefix}/hosts/{host_id}/items/{item_id}/state` | ✓ | Installed version string (empty if unknown) |
| `{prefix}/hosts/{host_id}/items/{item_id}/latest_version` | ✓ | Latest available version string (empty if unknown) |
| `{prefix}/hosts/{host_id}/items/{item_id}/attributes` | ✓ | JSON attributes: `{"in_progress": true/false}` |
| `{prefix}/hosts/{host_id}/items/{item_id}/set` | — | Command topic — publish `"install"` to trigger an update |

Additionally, Uptrakit publishes the following retained identity topics per host:

| Topic | Retained | Purpose |
| --- | :---: | --- |
| `{prefix}/hosts/{host_id}/hostname` | ✓ | Raw hostname string (for MQTT explorer visibility) |
| `{prefix}/hosts/{host_id}/friendly_name` | ✓ | User-defined display name (for MQTT explorer visibility) |

For every **host** that has at least one tracked non-featured software item, Uptrakit also publishes:

| Topic | Retained | Purpose |
| --- | :---: | --- |
| `{prefix}/hosts/{host_id}/state` | ✓ | `"unknown"` when updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/latest_version` | ✓ | `"N available"` when updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/attributes` | ✓ | JSON: `{"in_progress": bool, "pending_count": N}` |
| `{prefix}/hosts/{host_id}/set` | — | Command topic — publish `"install"` to update all outdated non-featured items |
| `{prefix}/hosts/{host_id}/security/state` | ✓ | `"unknown"` when security updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/security/latest_version` | ✓ | `"N available"` when security updates pending, `"up-to-date"` otherwise |
| `{prefix}/hosts/{host_id}/security/attributes` | ✓ | JSON: `{"in_progress": bool, "pending_count": N}` |
| `{prefix}/hosts/{host_id}/security/set` | — | Command topic — publish `"install"` to update security packages only |

Where `{prefix}` is the **Topic Prefix** configured on the MQTT client (default: `uptrakit`).

The `attributes` topic carries a JSON payload with an `in_progress` boolean flag. It transitions to
`true` the moment an update is queued (either from the HA Install button, the web UI, or the CLI) and
returns to `false` once the update completes or fails. This flag is used by the Home Assistant `update`
entity to display a live "Installing…" spinner.

All four topics update automatically whenever a version check completes, an update is triggered, or an
update finishes. No extra configuration is required.

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

- The software item is **enabled**, **featured**, and not deactivated.
- The host is **active** (not deactivated) and assigned to the software item.

Each entity displays:

| Attribute | Description |
| --- | --- |
| Installed version | The currently installed version on the host (blank if unknown) |
| Latest version | The newest available version (blank if unknown) |
| Update available | `true` when latest > installed |
| In progress | `true` while an update is pending or executing (displays a spinner in the HA UI) |
| Release URL | Link to the upstream release page (GitHub releases only; absent otherwise) |
| Release summary | First 500 characters of the release notes (GitHub releases only; absent otherwise) |

When the plugin for a software item fetches releases from GitHub, Uptrakit includes `release_url` and
`release_summary` in the HA MQTT discovery config for each entity. Home Assistant surfaces these as
entity attributes, giving users a direct link to the GitHub release page and a preview of the changelog
without leaving Home Assistant.

For plugins that only track version numbers (e.g. apt, Homebrew, Docker Hub) the `release_url` and
`release_summary` attributes are omitted from the discovery config.

All entities for a given host -- featured software items, host summaries, and security updates -- are grouped under
a single HA device per host. The device is identified by
`uptrakit_host_{tenant_id_hex}_{host_id_hex}` and named after the host's **friendly name** (the
user-defined display name set on each host). This means all software items, packages, and security
entities for a host appear together under one device in Home Assistant.

Each software item entity is named after the software item (e.g. "nginx"). Entity IDs are assigned on
first registration using a stable `default_entity_id` in the form
`uptrakit_{friendly_name_slug}_{item_slug}`, where slugs are lowercase with non-alphanumeric characters
replaced by underscores. For example, software item "uptrakit pangolin" on a host with friendly name
"pangolin.uk.home.yantsen.su" gets the entity ID
`update.uptrakit_pangolin_uk_home_yantsen_su_uptrakit_pangolin`.

## Triggering Updates from Home Assistant

When an update is available, an **Install** button appears on the entity card. Pressing it sends an MQTT
command to Uptrakit via the command topic for that entity.

Uptrakit validates the request and, if accepted, creates an `update_history` record with:

- `actor_type = "mqtt"`
- `actor_id = <mqtt_client_id>`

The update is then dispatched to the appropriate agent exactly as if it had been triggered from the web UI
or CLI.

**In-progress state feedback:** Uptrakit immediately publishes `{"in_progress": true}` to the
`{prefix}/hosts/{host_id}/items/{item_id}/attributes` topic after accepting the update command. Home
Assistant recognises this attribute on the `update` entity and displays a spinner. Once the agent reports
the final result (completed or failed), Uptrakit publishes updated state topics including
`{"in_progress": false}`.

> **Note:** Uptrakit never triggers updates automatically. Update execution always requires an explicit
> user action — from the web UI, CLI, or Home Assistant.

## Host Summary Update Entities

In addition to per-software-item entities, Uptrakit creates **two** Home Assistant `update` entities
**per host** for non-featured software tracking. Both represent auto-discovered packages (APT,
Homebrew, npm, etc.) grouped at the host level.

> **Note:** Both host summary entities are **disabled by default** in Home Assistant. You must
> explicitly enable them in **Settings > Devices & Services > Entities** before they appear in your
> dashboard. This prevents noise for users who do not need package-level tracking in HA.

### All-packages entity (`{friendly_name} packages`)

Tracks all outdated non-featured items on the host regardless of category.

### Security updates entity (`{friendly_name} security updates`)

Tracks only non-featured items where `update_category = "security"`. Pressing **Install** on this
entity applies only security updates, leaving other pending updates in place.

### When Are the Entities Created

Both entities are published when:

- The host has at least one **enabled**, non-deactivated, non-featured software item tracked.
- The MQTT client is connected and has the host's tenant assigned.

If a host has no tracked non-featured items, no entities are published for it.

### What the Entities Show

| Pending | `state` (installed) | `latest_version` | HA behaviour |
| --- | --- | --- | --- |
| 0 | `"up-to-date"` | `"up-to-date"` | No update badge |
| N > 0 | `"unknown"` | `"N available"` | Update badge, shows "N available" |

Home Assistant shows the **Update available** badge whenever `installed_version != latest_version`.
When updates are pending, `state` is `"unknown"` (there is no single aggregate installed version)
and `latest_version` shows the count, triggering the badge.

Each entity exposes two attributes:

| Attribute | Type | Description |
| --- | --- | --- |
| `pending_count` | integer | Number of qualifying packages with an available update |
| `in_progress` | boolean | `true` while a batch update is pending or running |

### Triggering a Host Batch Update from Home Assistant

When at least one update is available, an **Install** button appears on the entity card.

**All-packages entity:** Pressing **Install** sends an MQTT command to
`{prefix}/hosts/{host_id}/set`. The controller finds all outdated non-featured items and
dispatches a batch update.

**Security entity:** Pressing **Install** sends an MQTT command to
`{prefix}/hosts/{host_id}/security/set`. The controller filters the batch to only items with
`update_category = "security"`.

In both cases, Uptrakit validates the request and, if accepted:

1. Finds the qualifying outdated items (all, or security-only).
2. Creates an `update_history` record per item and a single `update_batch`.
3. Sends an `execute_batch_update` command to the agent for each plugin group.
4. Immediately sets `in_progress: true` in the `attributes` topic so the HA spinner appears.

Once the agent completes the update, Uptrakit updates installed versions and publishes fresh state
topics, returning `in_progress: false` and recalculating `pending_count`.

> **Note:** It is not possible to select individual non-featured items via the HA Install button.
> Individual item control is available via the Uptrakit web UI.

### Device and Entity Naming

All entities (featured software items, host summaries, security) are grouped under a single HA device per host:

- **Device**: `identifiers: ["uptrakit_host_{tenant_hex}_{host_hex}"]`, `name: {friendly_name}`

| Entity | Unique ID | Name | Default entity ID |
| --- | --- | --- | --- |
| Software item | `uptrakit_{tenant_hex}_{host_hex}_{item_hex}` | `{item_name}` | `update.uptrakit_{fn_slug}_{item_slug}` |
| All-packages | `uptrakit_{tenant_hex}_{host_hex}_pkgs` | `{friendly_name} packages` | `update.uptrakit_{fn_slug}_packages` |
| Security | `uptrakit_{tenant_hex}_{host_hex}_sec` | `{friendly_name} security updates` | `update.uptrakit_{fn_slug}_security_updates` |

Where `{tenant_hex}` and `{host_hex}` are UUID strings with dashes removed, and `{fn_slug}` is the
friendly name slugified (lowercased, non-alphanumeric characters replaced by underscores).

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
| Item not visible in HA | Software item is not featured (non-featured items appear in host summary entities) |
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
- The MQTT command topics that accept Install commands are scoped per entity and are accessible to anyone
  with publish access to the broker. Ensure your broker uses authentication and access control to limit
  who can publish to Uptrakit command topics.
  - Software item command topics: `{prefix}/hosts/{host_id}/items/{item_id}/set`
  - Host summary command topics: `{prefix}/hosts/{host_id}/set`
  - Host security command topics: `{prefix}/hosts/{host_id}/security/set`
- Update requests received via MQTT are validated by the controller (same checks as REST API triggers):
  - Tenant scope verification (the MQTT client must be assigned the same tenant as the target resource).
  - Host assignment and agent approval checks.
  - Duplicate update prevention (no two concurrent updates for the same host).

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
