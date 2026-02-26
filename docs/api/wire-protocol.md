# Wire Protocol

Agent, SSH agent, and MQTT service communication with the controller happens over a secure WebSocket (`/api/v1/ws/service`). All
services share the same `ServiceMessage`/`ControllerMessage` enums, with variants enabled per capability set.

## Connection Types

| Type | Authentication | Purpose |
| --- | --- | --- |
| Anonymous | None | Enrollment (initial handshake). |
| Enrolled | Bearer token | Certificate request (`request_certificate`). |
| Authenticated | mTLS client cert | Normal operation (heartbeats, commands, data). |

Agents and SSH agents initiate outbound-only connections and never accept inbound traffic. MQTT services and SSH agents
use the same enrollment model (shared service abstraction).

## Service Activity Tracking

For Agent, SSH agent, and MQTT services, the controller updates `services.last_seen_at` on every successful WebSocket connect
and on heartbeat ping/pong exchanges. The controller updates `services.ip_address` on each connect when a client IP is
resolved by the network middleware.

Accurate client IP tracking depends on trusted-proxy configuration; see
[docs/security/reverse-proxy-security.md](../security/reverse-proxy-security.md).

## Agent Lifecycle

1. Connect anonymously and send `enroll` with hostname, capabilities, and optional enrollment token.
1. Controller assigns UUIDv7 `service_id` and responds with `enrolled` (includes enrollment secret).
1. Agent generates an ECDSA P-256 keypair locally and submits a CSR (`request_certificate`).
1. Controller validates the CSR, signs it, and returns `certificate`.
1. Agent reconnects with mTLS and enters authenticated state.
1. Normal operation: `ping`/`pong`, `version_check_results`, `update_output`, etc.
1. On shutdown, agent sends `disconnecting` and waits for in-flight updates to finish.

## Message Types

### Shared (service -> controller)

`ping`, `enroll`, `request_certificate`, `renew_certificate`, `disconnecting`, `error`

### Agent-specific (service -> controller)

`report_hosts`, `version_check_results`, `update_started`, `update_output`, `update_result`, `discovery_results`

### SSH agent-specific (service -> controller)

`report_hosts`, `version_check_results`, `update_started`, `update_output`, `update_result`, `discovery_results`

### MQTT-specific (service -> controller)

`register`, `release_tenants`, `mqtt_client_status`, `mqtt_trigger_update`

### Shared (controller -> service)

`pong`, `enrolled`, `approved`, `rejected`, `certificate`, `error`, `service_settings`, `ca_bundle_updated`,
`request_cert_renewal`, `server_restarting`

#### `server_restarting` payload

Sent by the controller to all connected services before it shuts down (SIGTERM / SIGINT / SIGUSR1 takeover).

```json
{
  "seq": 1,
  "type": "server_restarting",
  "reason": "graceful restart"
}
```

On receipt, services initiate their own graceful shutdown:

1. Drain any in-flight work (agents wait for a running update to complete within the shutdown timeout).
2. Send `disconnecting` with `reason: restart`.
3. Exit the event loop with `LoopOutcome::Disconnected`.
4. Reconnect with backoff once the controller is available again.

This differs from an OS-signal shutdown (`SIGTERM`/`SIGINT`): a signal causes the service to exit its lifecycle
entirely (`LoopOutcome::Shutdown`), while `server_restarting` causes it to reconnect automatically
(`LoopOutcome::Disconnected`). See [Graceful Restart](../development/graceful-restart.md) for the full sequence.

### Agent-specific (controller -> service)

`check_versions`, `execute_update`, `discover_software`

Both the regular agent and the SSH agent receive `check_versions`, `execute_update`, and `discover_software` messages.
The `host_machine_id` field in each payload determines which host the operation targets
(see [`host_machine_id` Field](#host_machine_id-field)).

#### `check_versions` payload

Each `VersionCheckAssignment` carries role-based `PluginAssignment` entries instead of flat plugin fields.
The `detect_version` and `fetch_releases` fields are optional -- absent when no plugin is configured for
that role on this host-software pair.

```json
{
  "seq": 1,
  "type": "check_versions",
  "host_machine_id": "abc-123",
  "assignments": [
    {
      "software_item_id": "550e8400-...",
      "name": "Nginx",
      "detect_version": {
        "plugin_type": "package_manager_apt",
        "package_identifier": "nginx",
        "config": {}
      },
      "fetch_releases": {
        "plugin_type": "package_manager_apt",
        "package_identifier": "nginx",
        "config": {}
      }
    },
    {
      "software_item_id": "660e8400-...",
      "name": "1Password",
      "detect_version": {
        "plugin_type": "package_manager_homebrew",
        "package_identifier": "1password-cli",
        "config": { "package_type": "cask" }
      }
    }
  ]
}
```

**`PluginAssignment` fields:**

| Field | Type | Description |
| --- | --- | --- |
| `plugin_type` | string | Plugin discriminator (e.g. `"apt"`, `"homebrew"`, `"releases_github"`) |
| `package_identifier` | string | Plugin-specific package identifier |
| `config` | object | Merged plugin configuration (base config + override) |

The `fetch_releases` field is only included for agent-side plugins (those without the
`ControllerSideFetchReleases` capability, or with `execution_site = "agent"`). Controller-side
`fetch_releases` (e.g. GitHub Releases, Docker Registry) is handled by the controller scheduler
and is not sent to the agent.

#### `execute_update` payload

```json
{
  "seq": 1,
  "type": "execute_update",
  "host_machine_id": "abc-123",
  "update_history_id": "770e8400-...",
  "software_item_id": "550e8400-...",
  "software_item_name": "Nginx",
  "to_version": "1.24.0",
  "detect_version_plugin": {
    "plugin_type": "package_manager_apt",
    "package_identifier": "nginx",
    "config": {}
  },
  "execute_update_plugin": {
    "plugin_type": "package_manager_apt",
    "package_identifier": "nginx",
    "config": {}
  },
  "pre_update_hooks": [],
  "post_update_hooks": []
}
```

| Field | Type | Description |
| --- | --- | --- |
| `host_machine_id` | string | Target host's machine ID |
| `update_history_id` | UUID | Update history record ID |
| `software_item_id` | UUID | Software item being updated |
| `software_item_name` | string | Display name for logging |
| `to_version` | string | Target version |
| `detect_version_plugin` | `PluginAssignment?` | Plugin for before/after installed-version detection. Absent when no `detect_version` plugin is configured. |
| `execute_update_plugin` | `PluginAssignment` | Plugin for the `execute_update` role (required) |
| `pre_update_hooks` | `Vec<HookCommand>` | Pre-update hook commands |
| `post_update_hooks` | `Vec<HookCommand>` | Post-update hook commands |

#### `discover_software` payload

```json
{
  "seq": 1,
  "type": "discover_software",
  "host_machine_id": "abc-123",
  "plugins": [
    {
      "plugin_config_id": "550e8400-...",
      "plugin_type": "package_manager_homebrew",
      "config": { "package_type": "formula" }
    }
  ]
}
```

`plugin_config_id` is `null` for auto-discovery runs where no pre-existing `PluginConfig` exists for the plugin
type. In that case the agent uses the default/empty config and plugins emit `DiscoveryTarget` values inside
each `DiscoveredSoftware` item's `targets` array. The controller creates the appropriate `PluginConfig` records
from these structured targets.

Known `plugin_type` values for discovery: `package_manager_homebrew`, `discovery_proxmox_helper_scripts`, `package_manager_apt`.

#### `discovery_results` payload

```json
{
  "seq": 1,
  "type": "discovery_results",
  "host_machine_id": "abc-123",
  "results": [
    {
      "plugin_config_id": null,
      "plugin_type": "discovery_proxmox_helper_scripts",
      "discoveries": [
        {
          "package_identifier": "booklore",
          "name": "BookLore",
          "installed_version": "1.18.5",
          "targets": [
            {
              "plugin_type": "releases_github",
              "plugin_config": { "owner": "BookLore", "repo": "BookLore" },
              "plugin_config_name": "BookLore/BookLore",
              "roles": ["detect_version", "fetch_releases", "execute_update"]
            }
          ]
        }
      ],
      "error": null
    },
    {
      "plugin_config_id": "550e8400-...",
      "plugin_type": "package_manager_homebrew",
      "discoveries": [
        {
          "package_identifier": "wget",
          "name": "Wget",
          "installed_version": "1.21.4"
        }
      ],
      "error": null
    }
  ]
}
```

#### `DiscoveredSoftware` fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `package_identifier` | string | Yes | Plugin-specific package identifier |
| `name` | string | Yes | Human-readable display name |
| `installed_version` | string | Yes | Currently installed version |
| `targets` | `DiscoveryTarget[]` | No | Structured targets for plugin config creation (empty = use discovering plugin's config) |
| `extra` | object | No | Informational metadata only (e.g. Docker's container names). Not used for config synthesis. |

#### `DiscoveryTarget` fields

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `plugin_type` | string | Yes | Target plugin type (may differ from discovering plugin) |
| `plugin_config` | object | Yes | Config JSON for find-or-create of the target plugin config |
| `plugin_config_name` | string | Yes | Display name for auto-created plugin config |
| `roles` | `string[]` | Yes | Which roles this target covers (e.g. `["detect_version", "fetch_releases", "execute_update"]`) |
| `package_identifier` | string | No | Package identifier override (default: same as parent item) |
| `config_override` | object | No | Per-assignment config override |
| `execution_site` | string | No | Execution site hint (`"auto"`, `"agent"`, `"controller"`; default: `"auto"`) |

See [docs/api/autodiscovery.md](autodiscovery.md#plugin-driven-discovery-targets) for the full
processing rules and plugin-specific target patterns.

See [docs/api/autodiscovery.md](autodiscovery.md) for the full autodiscovery workflow.

### MQTT-specific (controller -> service)

`registered`, `tenant_assignments`, `tenant_config_updated`, `tenant_revoked`, `software_states`

## `host_machine_id` Field

`CheckVersionsPayload` and `ExecuteUpdatePayload` both carry a required `host_machine_id: String` field as their first
field. The controller groups role-based plugin assignments by `(service_id, host_machine_id)` and sends one message per
host so that each agent instance receives only the checks and updates relevant to its managed hosts.

### Regular agent behavior

The regular agent validates that the received `host_machine_id` matches the local machine ID collected at startup. A
mismatch is logged as a warning and the message is silently ignored (returns `Ok(None)`). This is a defensive sanity
check; under normal operation the controller only routes messages to the correct agent.

### SSH agent behavior

The SSH agent uses `host_machine_id` to look up which SSH host to connect to by calling `find_host_by_machine_id()`
against its local `ssh_hosts` database. If no matching host is found the message is rejected with an error. When
`ReportHosts` completes, `update_host_machine_id()` persists each remote host's `machine_id` so that subsequent
routing lookups succeed.

See [SSH Agent Architecture — Version Check and Update Execution](../architecture/ssh-agent.md#version-check-and-update-execution)
for the full dispatch flow.

## Replay Protection

Every envelope (`ServiceEnvelope` / `ControllerEnvelope`) carries a monotonically increasing `seq` starting at `1`.
Each connection tracks per-direction counters; mismatched sequences cause the connection to close with
`ErrorCode::SequenceError`.

Sequence validation is performed before full message deserialization. When a message has a valid sequence number but
an unrecognized type (e.g., from a newer protocol version), the sequence counter is correctly advanced and the message
is silently skipped. This ensures that unknown message types do not cause sequence mismatches on subsequent messages.

## Connection Limits

| Limit | Value | Description |
| --- | --- | --- |
| Maximum incoming message size | 1 MB (1,048,576 bytes) | Messages exceeding this limit are rejected and the connection is closed. |
| Anonymous connection timeout | 30 seconds | An anonymous connection that does not send `Enroll` within 30 seconds is closed. |
| Update output cap | 1 MB | The controller caps accumulated `update_history.output`. Further `UpdateOutput` messages are silently dropped. |
| Approval polling interval | 5 seconds | The controller polls the database for approval status changes at a fixed 5-second interval. |
| TCP connect timeout (client) | 30 seconds | The enrollment client aborts the TCP connection if it cannot be established within 30 seconds. |
| Response timeout (client) | 60 seconds | The `Enroll` and `RequestCertificate` request-response exchanges time out after 60 seconds. |
| Approval timeout (client) | 30 minutes | The `wait_for_approval` loop times out after 30 minutes. The caller retries the enrollment flow on timeout. |

## Error Codes

The `ErrorCode` enum is marked `#[non_exhaustive]` — new codes may be added in future protocol versions. Consumers should include a wildcard match arm.

| Code | Description |
| --- | --- |
| `bad_request` | Malformed or invalid message. |
| `enrollment_failed` | Enrollment could not be completed. |
| `not_approved` | Service has not been approved yet. |
| `forbidden` | Service is not authorized for this action. |
| `certificate_error` | CSR validation or certificate issuance failed. |
| `internal_error` | Unexpected server-side error. |
| `sequence_error` | Incoming sequence number does not match the expected value. Connection is closed. |

## WebSocket Close Reasons

When the controller closes a WebSocket connection, it includes a reason string in the close frame. These
reasons are defined as variants of the `CloseReason` enum in the `uptrakit_internal_wire::close_reason`
module. The enum provides `Display` (for sending) and `FromStr` (for receiving) implementations that
produce and parse the same wire strings. An `Unknown(String)` variant provides forward compatibility
for strings not yet recognized by the receiver.

| Variant | Wire String | Description |
| --- | --- | --- |
| `CloseReason::CertificateRotated` | `"certificate rotated"` | Service certificate was rotated; reconnect with new cert. |
| `CloseReason::CertificateRevoked` | `"certificate revoked"` | Service certificate was revoked; re-enrollment required. |
| `CloseReason::NoValidCertificate` | `"no valid certificate"` | No valid client certificate presented. |
| `CloseReason::InternalError` | `"internal error"` | Unexpected server-side error. |
| `CloseReason::CertificateNotRecognized` | `"certificate not recognized"` | Client certificate not recognized by the controller. |
| `CloseReason::ServiceDeactivated` | `"service deactivated"` | Service has been deactivated by an administrator. |
| `CloseReason::ServiceNotApproved` | `"service not approved"` | Service has not been approved for connection. |
| `CloseReason::ServiceNotFound` | `"service not found"` | Service ID not found in the database. |
| `CloseReason::EnrollmentTimeout` | `"enrollment timeout"` | Enrollment did not complete within the allowed time. |
| `CloseReason::Superseded` | `"superseded by new connection"` | Another instance connected with the same service ID. |
| `CloseReason::RateLimitExceeded` | `"rate limit exceeded"` | Connection rate limit exceeded. |
| `CloseReason::Unknown(String)` | *(any other string)* | Forward-compatible catch-all for unrecognized reasons. |

Services should match on enum variants (not raw strings) to determine reconnection behavior:

- `CloseReason::CertificateRotated` → reconnect immediately with new certificate.
- `CloseReason::CertificateRevoked` → stop; re-enrollment needed.
- Other variants → reconnect with backoff or terminate depending on severity.

## `ServiceSettingsPayload` Fields

The `ServiceSettingsPayload` struct is sent by the controller as a `service_settings` message after an
authenticated connection is established. It carries runtime configuration for the connected service.

| Field | Type | Serde | Description |
| --- | --- | --- | --- |
| `renewal_window_hours` | `u16` | required | Hours before certificate expiry to initiate renewal |
| `ca_bundle_hash` | `String` | `#[serde(default)]` | Hash of the current CA bundle for staleness detection |
| `capabilities` | `BTreeSet<Capability>` | `#[serde(default, skip_serializing_if = "BTreeSet::is_empty")]` | Set of capabilities advertised by the controller; used for capability negotiation |
| `shutdown_timeout_seconds` | `Option<u32>` | `#[serde(default, skip_serializing_if)]` | Max seconds to wait during shutdown; present for agents, absent for MQTT |
| `ping_interval` | `Duration` | `#[serde(with = "duration_seconds")]` | Controller-managed ping interval; derived from per-service DB override or service-profile default (300s agent/SSH agent, 15s MQTT) |

The `ping_interval` field is serialized as a `u32` number of seconds on the wire (e.g. `"ping_interval": 300`)
using the `duration_seconds` serde module. The controller reads `ping_interval_seconds` from the `services` table
for each service and falls back to `ServiceProfile` defaults when no override is set.

### `duration_seconds` serde module

The `duration_seconds` module (`uptrakit_internal_wire::duration_seconds`) provides `serialize` and `deserialize`
functions for converting `std::time::Duration` to/from a `u32` number of seconds in JSON. Use it with
`#[serde(with = "duration_seconds")]` on `Duration` fields that should appear as plain integer seconds on the wire.

## `HostInfo` Fields

The `HostInfo` struct (used inside `ReportHostsPayload.hosts`) contains:

| Field | Type | Description |
| --- | --- | --- |
| `machine_id` | `String` | Persistent system identifier (required) |
| `os_type` | `Option<String>` | Operating system type (e.g. `linux`, `macos`) |
| `os_version` | `Option<String>` | OS version or pretty name |
| `architecture` | `Option<String>` | CPU architecture (e.g. `x86_64`, `aarch64`) |
| `hostname` | `Option<String>` | Machine hostname |
| `ip_address` | `Option<String>` | IP address or hostname used to reach the host |

The `hostname` and `ip_address` fields were added as part of the SSH agent host reporting feature.
They use `#[serde(default)]` for backward compatibility — agents that do not send these fields
will have them default to `None` on the controller side.

Both the regular agent and the SSH agent send `report_hosts`. The regular agent sets `hostname`
from local system calls. The SSH agent sets `hostname` from the remote host's `hostname` command
and `ip_address` from the SSH target address.

## Capability Negotiation

Protocol feature negotiation is capability-based rather than version-based. Both sides advertise the features they
support at the start of each authenticated connection; neither requires a hard cutover when features are added.

### How It Works

1. After mTLS authentication succeeds, the controller sends `service_settings` containing `capabilities: [...]`.
2. The service sends `report_hosts` (agent/SSH agent) or `register` (MQTT) containing its own `capabilities: [...]`.
3. Each side independently computes the **agreed set**: the intersection of the two capability sets, excluding `Other`
   values (unrecognized capabilities from a newer peer).
4. The agreed set is stored on the connection and can be used to gate feature-specific flows.

The HTTP path `/api/v1/ws/service` provides the hard-break slot for truly incompatible format changes.

### Defined Capabilities

| Capability | Wire String | Description |
| --- | --- | --- |
| `SoftwareDiscovery` | `software_discovery` | Service supports `discover_software` → `discovery_results` flow. Controller gates autodiscovery requests on this capability. |
| `UpdateHooks` | `update_hooks` | Service supports pre-/post-update hook commands in `execute_update`. Controller omits hooks when absent. |
| `GracefulShutdown` | `graceful_shutdown` | Service sends `disconnecting` before clean exit and honours `shutdown_timeout_seconds`. |
| `MqttBridge` | `mqtt_bridge` | Service is an MQTT bridge: handles `register`, `tenant_assignments`, `release_tenants`, `mqtt_client_status`, etc. Maps to `ServiceProfile::MqttBridge`. |
| `SshRemote` | `ssh_remote` | Service manages remote hosts over SSH. Combined with `SoftwareDiscovery`, maps to `ServiceProfile::Agent` with SSH label. |
| `Other(String)` | *(any unknown string)* | Forward-compatible catch-all. Never participates in intersection. |

### Advertised Sets per Component

| Component | `software_discovery` | `update_hooks` | `graceful_shutdown` | `mqtt_bridge` | `ssh_remote` |
| --- | :---: | :---: | :---: | :---: | :---: |
| Controller | ✓ | ✓ | ✓ | ✓ | ✓ |
| Agent | ✓ | ✓ | ✓ | — | — |
| SSH Agent | ✓ | ✓ | ✓ | — | ✓ |
| MQTT Service | — | — | ✓ | ✓ | — |

The controller advertises all known capabilities so every service can compute its agreed set regardless of its type.

### Service-Profile Derivation from Capabilities

The controller derives a `ServiceProfile` from each service's persisted capability set. The profile drives
behavioral defaults (ping interval, shutdown timeout, human-readable label). `ServiceProfile` is never
stored -- it is always computed from capabilities.

| Capability set | `ServiceProfile` | Label |
| --- | --- | --- |
| Has `mqtt_bridge` | `MqttBridge` | "MQTT Bridge" |
| Has `ssh_remote` + `software_discovery` | `Agent` | "SSH Agent" |
| Has `software_discovery`, no `mqtt_bridge`, no `ssh_remote` | `Agent` | "Agent" |
| Unrecognized combination | `Unknown` | "Unknown" |

`EnrollPayload.capabilities` is a required `BTreeSet<Capability>` field. The controller persists the
capabilities in the `services.capabilities` column (JSON array of snake_case strings) and derives
the `ServiceProfile` at read time.

## Forward Compatibility

Several wire protocol enums are marked `#[non_exhaustive]` to allow adding new variants without breaking downstream
consumers:

- `ErrorCode` — new error codes may be added.
- `EnrollmentStatus` — new enrollment statuses may be added.
- `UpdateFinalStatus` — new update result statuses may be added.
- `DisconnectReason` — new disconnect reasons may be added.

Consumers matching on these enums must include a wildcard (`_`) arm to handle unknown variants gracefully.

## `MqttTenantConfig` Fields

The `MqttTenantConfig` struct is used in `tenant_assignments` and `tenant_config_updated` messages. Key fields:

| Field | Type | Description |
| --- | --- | --- |
| `mqtt_client_id` | UUID | Primary identifier from the `mqtt_clients` table |
| `tenant_id` | UUID | Tenant UUID |
| `enabled` | bool | Whether this MQTT client is enabled |
| `transport` | `tcp` / `tls` | Transport protocol |
| `host` | String | MQTT broker hostname |
| `port` | u16 | MQTT broker port |
| `client_id` | String | MQTT client ID for broker connection |
| `username` | `Option<SecretString>` | Broker authentication username |
| `password` | `Option<SecretString>` | Broker authentication password |
| `ca_pem` | `Option<SecretString>` | Custom CA certificate in PEM format for private brokers |
| `topic_prefix` | String | MQTT topic prefix |
| `ha_discovery` | bool | Whether to publish Home Assistant MQTT discovery topics (`#[serde(default)]`) |
| `ha_discovery_prefix` | String | HA MQTT discovery topic prefix (default `"homeassistant"`) |
| `updated_at` | i64 | Last update timestamp in milliseconds |

The `ca_pem` field is optional and uses `#[serde(default, skip_serializing_if = "Option::is_none")]` for
backward compatibility. When present, the MQTT service uses the PEM bytes as the trusted CA for TLS
connections instead of the system trust store. Credentials (`username`, `password`, `ca_pem`) use
`SecretString` for zeroize-on-drop and redacted debug output. The `ca_pem` field and both `ha_discovery*`
fields are included in the config hash computation for change detection.

## `software_states` Payload

The controller pushes this message to all locally connected MQTT services for a tenant whenever version
data changes (e.g. after a version check or an update completes). It is also written to the outbox for
cross-controller delivery (contains no credentials). MQTT services filter by `tenant_id`.

```json
{
  "seq": 1,
  "type": "software_states",
  "tenant_id": "550e8400-e29b-41d4-a716-446655440001",
  "items": [
    {
      "software_item_id": "660e8400-e29b-41d4-a716-446655440002",
      "name": "My App",
      "hosts": [
        {
          "host_id": "770e8400-e29b-41d4-a716-446655440003",
          "hostname": "my-host",
          "installed_version": "1.2.3",
          "latest_version": "1.3.0",
          "update_available": true
        }
      ]
    }
  ]
}
```

Each item in `items` corresponds to one enabled, non-deactivated software item whose `discovery_state` is
`null` or `"approved"`. Each `hosts` entry contains the version data for one host that tracks the software
item. An empty string in `installed_version` or `latest_version` means the version is not yet known.

When `ha_discovery = true` for an MQTT client, the MQTT service publishes HA discovery configs and retained
state topics from this payload. See [Home Assistant Integration](../end-user/home-assistant-mqtt.md).

## `mqtt_trigger_update` Payload

Sent by the MQTT service to the controller when a Home Assistant user presses **Install** on a tracked
software item. The controller validates the request and dispatches `execute_update` to the appropriate
agent. On validation failure the controller sends `error` back to the MQTT service (soft error — the
WebSocket connection is not closed).

```json
{
  "seq": 2,
  "type": "mqtt_trigger_update",
  "tenant_id": "550e8400-e29b-41d4-a716-446655440001",
  "software_item_id": "660e8400-e29b-41d4-a716-446655440002",
  "host_id": "770e8400-e29b-41d4-a716-446655440003",
  "to_version": "1.3.0",
  "mqtt_client_id": "880e8400-e29b-41d4-a716-446655440004"
}
```

| Field | Type | Description |
| --- | --- | --- |
| `tenant_id` | UUID | Tenant the update targets — must match an assigned tenant for this MQTT service |
| `software_item_id` | UUID | The software item to update |
| `host_id` | UUID | The host on which to apply the update |
| `to_version` | String | Target version string resolved from the last `software_states` push |
| `mqtt_client_id` | UUID | The MQTT client that received the Install command; stored as `actor_id` in `update_history` |

The resulting `update_history` record has `actor_type = "mqtt"` and `actor_id = <mqtt_client_id>`.

The controller rejects the request with an `error` message (no WS close) in the following cases:

- `tenant_id` does not match an assigned tenant for this MQTT service instance.
- The software item, host, or host assignment does not exist or is deactivated.
- The host has no approved agent linked.
- An update with status `pending` or `in_progress` already exists for the same `(host_id, software_item_id)` pair.

## AsyncAPI Specification

The full message schema and payload definitions are published in `crates/shared/wire/asyncapi.yaml`. Use this document
to generate clients or validate payload structures. Ensure protobuf/JSON serializers conform to the spec before
releasing.
