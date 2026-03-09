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
   The `enrollment_token` field carries a single plaintext string; the controller resolves it
   against multiple stored tokens server-side (see [Enrollment Tokens API](enrollment-tokens.md)).
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

`report_hosts`, `version_check_results`, `update_started`, `update_output`, `update_result`, `discovery_results`,
`batch_host_package_update_result`

### SSH agent-specific (service -> controller)

`report_hosts`, `version_check_results`, `update_started`, `update_output`, `update_result`, `discovery_results`

> **Note:** For the SSH agent, `report_hosts` is sent both at connect time and dynamically during a
> session whenever the local `ssh_hosts` database changes (host added, removed, or updated). The
> controller's `handle_report_hosts` is idempotent — repeated calls upsert hosts by `machine_id`
> and are safe to process at any point during an authenticated session. See
> [Dynamic Host Reload](../architecture/ssh-agent.md#dynamic-host-reload) for the full mechanism.

### MQTT-specific (service -> controller)

`register`, `release_tenants`, `mqtt_client_status`, `mqtt_trigger_update`,
`mqtt_trigger_host_package_update`

### Shared (controller -> service)

`pong`, `enrolled`, `approved`, `rejected`, `certificate`, `error`, `service_settings`, `ca_bundle_updated`,
`request_cert_renewal`, `server_restarting`

#### `server_restarting` payload

Sent by the controller to all connected services before it shuts down (SIGTERM / SIGINT / SIGUSR1 takeover).

```json
{
  "protocol_version": 1,
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

`check_versions`, `execute_update`, `discover_software`, `execute_batch_host_package_update`,
`set_update_freeze`

Both the regular agent and the SSH agent receive `check_versions`, `execute_update`, `discover_software`, and
`execute_batch_host_package_update` messages.
The `host_machine_id` field in each payload determines which host the operation targets
(see [`host_machine_id` Field](#host_machine_id-field)).

#### `check_versions` payload

> **Security note:** When published to NATS JetStream, plugin config fields in
> this message are AES-256-GCM encrypted using the shared master key. Receiving
> controllers decrypt the configs before delivering to agents. See
> [NATS Integration — Plugin Config Protection](../development/nats-integration.md#plugin-config-protection).

Each `VersionCheckAssignment` carries role-based `PluginAssignment` entries instead of flat plugin fields.
The `detect_version` and `fetch_releases` fields are optional -- absent when no plugin is configured for
that role on this host-software pair.

```json
{
  "protocol_version": 1,
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

> **Security note:** When published to NATS JetStream, plugin config fields in
> this message are AES-256-GCM encrypted using the shared master key. Receiving
> controllers decrypt the configs before delivering to agents. See
> [NATS Integration — Plugin Config Protection](../development/nats-integration.md#plugin-config-protection).

```json
{
  "protocol_version": 1,
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
  "post_update_hooks": [],
  "release_info": {
    "tag": "v1.24.0",
    "release_url": "https://github.com/owner/repo/releases/tag/v1.24.0",
    "assets": [
      {
        "name": "nginx_1.24.0_linux_amd64.tar.gz",
        "download_url": "https://github.com/owner/repo/releases/download/v1.24.0/nginx_1.24.0_linux_amd64.tar.gz",
        "sha256_digest": "a1b2c3d4..."
      }
    ],
    "attestation_status": "Verified",
    "require_attestation": false
  }
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
| `release_info` | `ReleaseInfo?` | Release metadata from the upstream source. Only present for GitHub Releases items. See [`ReleaseInfo` fields](#releaseinfo-fields). |
| `timeout_seconds` | `Duration` | Maximum execution time for the entire update (including hooks). Rust field is `timeout`, serialized as seconds on the wire (`timeout_seconds`). Defaults to 7200 (2 hours) when omitted. |

#### `ReleaseInfo` fields

`ReleaseInfo` carries metadata about the upstream release being installed. It is populated by the
controller from `latest_release_metadata` at update-trigger time and is used by the agent for
attestation verification and by update plugins for download URL resolution.

| Field | Type | Serde | Description |
| --- | --- | --- | --- |
| `tag` | `string` | required | Release tag name (e.g. `"v1.24.0"`) |
| `release_url` | `string` | required | URL of the release page (e.g. `"https://github.com/owner/repo/releases/tag/v1.24.0"`) |
| `assets` | `ReleaseAsset[]` | `#[serde(default, skip_serializing_if = "Vec::is_empty")]` | Release assets available for download |
| `attestation_status` | `AttestationStatus?` | `#[serde(default, skip_serializing_if = "Option::is_none")]` | GitHub Actions attestation status determined at fetch time. Only set for GitHub Releases items when `verify_attestation = true`. See [`AttestationStatus`](#attestationstatus). |
| `require_attestation` | `bool` | `#[serde(default)]` | When `true`, the agent aborts the update if `attestation_status` is `NotFound` after independent re-verification. Set by the controller from `GitHubConfig.require_attestation`. |

#### `ReleaseAsset` fields

| Field | Type | Serde | Description |
| --- | --- | --- | --- |
| `name` | `string` | required | Asset filename (e.g. `"nginx_1.24.0_linux_amd64.tar.gz"`) |
| `download_url` | `string` | required | Direct download URL for the asset |
| `sha256_digest` | `string?` | `#[serde(default, skip_serializing_if = "Option::is_none")]` | SHA-256 hex digest parsed from the release checksums file. Exactly 64 lowercase hex characters. Used by the agent for attestation re-verification. |

#### `AttestationStatus`

Describes the result of querying the GitHub Attestations API for the release. Only present in
`release_info` for GitHub Releases items when `verify_attestation = true`.

The enum is `#[non_exhaustive]` — consumers must include a wildcard match arm.

| Value | Description |
| --- | --- |
| `Verified` | At least one attestation was found for the release asset digest via the GitHub Attestations API. |
| `NotFound` | The GitHub Attestations API returned no attestations (404 or empty array) for the digest. |
| `Unverified` | The check was skipped or inconclusive (no checksums file found, network error, or `verify_attestation = false`). |

See [GitHub Actions Attestation Verification](../security/github-attestation.md) for the full
two-stage verification flow.

#### `discover_software` payload

> **Security note:** When published to NATS JetStream, plugin config fields in
> this message are AES-256-GCM encrypted using the shared master key. Receiving
> controllers decrypt the configs before delivering to agents. See
> [NATS Integration — Plugin Config Protection](../development/nats-integration.md#plugin-config-protection).

```json
{
  "protocol_version": 1,
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
  "protocol_version": 1,
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

#### `set_update_freeze` payload

Remotely enables or disables the agent-side execution freeze file
(`<state-dir>/update-freeze`). When `enabled` is `true`, the agent creates the
freeze file and stops processing `ExecuteUpdate`/`ExecuteBatchHostPackageUpdate`
messages. When `false`, the agent removes the file and resumes normal operation.

```json
{
  "protocol_version": 1,
  "seq": 5,
  "type": "set_update_freeze",
  "enabled": true,
  "reason": "Emergency freeze: investigating suspicious update activity"
}
```

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `enabled` | `bool` | Yes | `true` to create the freeze file, `false` to remove it. |
| `reason` | `string` | No | Optional human-readable reason (logged by the agent). |

This message is safe for NATS publication (no credentials in the payload).

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

## Protocol Version

Every envelope (`ServiceEnvelope` / `ControllerEnvelope`) carries a required `protocol_version: u32` field
(currently `1`). Both the `protocol_version` and `seq` fields are always the first fields serialized so that
receivers can extract them before attempting a full parse.

When a breaking change is introduced to the wire format (a new required field, a variant renamed or removed,
or a change to capability-negotiation semantics), `CURRENT_PROTOCOL_VERSION` is incremented. Receivers that
encounter an unknown `protocol_version` must close the connection immediately with a protocol error and log
the mismatch. There is **no backwards-compatibility**: both peers must speak the same protocol version.

To upgrade the protocol version in a deployment, update all controllers and services simultaneously.

## Replay Protection

Every envelope (`ServiceEnvelope` / `ControllerEnvelope`) carries a monotonically increasing `seq` starting at `1`.
Each connection tracks per-direction counters; mismatched sequences cause the connection to close with
`ErrorCode::SequenceError`.

Sequence and protocol version validation are performed before full message deserialization. When a message has valid
header fields but an unrecognized `type` (e.g., a new variant from a future service build running the same protocol
version), the sequence counter is correctly advanced and the message is silently skipped. This ensures that unknown
message types do not cause sequence mismatches on subsequent messages.

## Distributed Tracing (TraceContext)

Every envelope may carry an optional `trace_context` object for distributed tracing correlation:

```json
{
  "protocol_version": 1,
  "seq": 1,
  "trace_context": {
    "trace_id": "0123456789abcdef0123456789abcdef",
    "span_id": "fedcba9876543210"
  },
  "type": "ping",
  "service_ts": 1706400000000
}
```

- `trace_id` (string, required within the object): 32 lowercase hex characters (128-bit W3C trace ID).
- `span_id` (string, optional): 16 lowercase hex characters (64-bit W3C span ID).

Senders always populate `trace_context`. Receivers tolerate its absence for compatibility
with older peers (`#[serde(default)]`). NATS event envelopes (`NatsEventEnvelope`) also
carry `trace_context`.

See [Tracing Conventions](../development/tracing.md) for the full tracing guide.

## Connection Limits

| Limit | Value | Description |
| --- | --- | --- |
| Maximum incoming message size | 1 MB (1,048,576 bytes) | Messages exceeding this limit are rejected and the connection is closed. |
| Message rate limit | 50 messages/second | Sliding-window-counter algorithm prevents boundary burst attacks. |
| Consecutive unknown messages | 10 | Connection is closed after 10 consecutive `Unknown` messages. |
| Anonymous connection timeout | 30 seconds | An anonymous connection that does not send `Enroll` within 30 seconds is closed. |
| Update output cap | 1 MB | The controller caps accumulated `update_history.output`. Further `UpdateOutput` messages are silently dropped. |
| Approval polling interval | 5 seconds | The controller polls the database for approval status changes at a fixed 5-second interval. |
| TCP connect timeout (client) | 30 seconds | The enrollment client aborts the TCP connection if it cannot be established within 30 seconds. |
| Response timeout (client) | 60 seconds | The `Enroll` and `RequestCertificate` request-response exchanges time out after 60 seconds. |
| Approval timeout (client) | 30 minutes | The `wait_for_approval` loop times out after 30 minutes. The caller retries the enrollment flow on timeout. |
| Per-hook timeout (agent) | 5 minutes | Individual pre/post-update hooks are killed after 300 seconds. |
| Update cooldown (agent) | 5 seconds | Agents reject consecutive updates within the cooldown period. |
| Report pagination total timeout | 5 minutes | All pages of a paginated report must arrive within 5 minutes of the first page. |
| Report pagination idle timeout | 15 seconds | Consecutive pages must arrive within 15 seconds of each other. |
| Maximum report pages | 50 | A single paginated report can have at most 50 pages. |
| Maximum pending reports | 10 | At most 10 paginated reports can be in-flight per connection. |

### Payload Size Limits

After deserialization, all wire protocol payloads are validated via the
`WireValidate` trait. Payloads exceeding any limit are rejected as
deserialization failures (hard fail, connection close).

#### Collection limits

| Constant | Value | Applies to |
| --- | --- | --- |
| `MAX_REPORT_HOSTS` | 500 | `ReportHostsPayload.hosts` |
| `MAX_VERSION_CHECK_ASSIGNMENTS` | 2,000 | `CheckVersionsPayload.assignments` |
| `MAX_VERSION_CHECK_RESULTS` | 2,000 | `VersionCheckResultsPayload.results` |
| `MAX_UPDATE_HOOKS` | 50 | `pre_update_hooks`, `post_update_hooks` |
| `MAX_BATCH_UPDATES` | 500 | `ExecuteBatchHostPackageUpdatePayload.updates` |
| `MAX_BATCH_UPDATE_RESULTS` | 500 | `BatchHostPackageUpdateResultPayload.results` |
| `MAX_DISCOVERY_PLUGINS` | 50 | `DiscoverSoftwarePayload.plugins` |
| `MAX_DISCOVERY_PLUGIN_RESULTS` | 50 | `DiscoveryResultsPayload.results` |
| `MAX_DISCOVERIES_PER_PLUGIN` | 1,000 | `DiscoveryPluginResult.discoveries` |
| `MAX_HOOK_ARGS` | 100 | `HookCommand::Exec.args` |
| `MAX_RELEASE_ASSETS` | 500 | `ReleaseInfo.assets` |

#### String length limits

| Constant | Value | Applies to |
| --- | --- | --- |
| `MAX_SHORT_STRING_LEN` | 1,024 | Identifiers, names, versions, hostnames |
| `MAX_MEDIUM_STRING_LEN` | 4,096 | Error messages, URLs |
| `MAX_LONG_STRING_LEN` | 65,536 | PEM certificates, CSRs, release notes |
| `MAX_OUTPUT_STRING_LEN` | 1,048,576 | Command output (matches 1 MB frame limit) |
| `SHA256_DIGEST_LEN` | 64 | `ReleaseAsset.sha256_digest` (must be exactly 64 lowercase hex characters) |

All limits are defined in `crates/shared/wire/src/limits.rs`. Implementations
are in `crates/shared/wire/src/wire_validate_impls.rs`.

### Sliding-Window Rate Limiter

The WebSocket message rate limiter uses a sliding-window-counter algorithm.
Two half-windows track message counts. The effective rate estimate is:

```text
estimate = prev_count * (1 - elapsed_fraction) + curr_count
```

This prevents boundary burst attacks where a fixed-window limiter would allow
2× the configured limit (N at the end of one window + N at the start of the
next). When the estimate exceeds `WS_MESSAGE_RATE_LIMIT` (50), the connection
is closed with `CloseReason::RateLimitExceeded`.

## Report Pagination

When a service-to-controller report payload (`discovery_results`, `version_check_results`,
`report_hosts`, `batch_host_package_update_result`) exceeds 768 KB
(`PAGINATION_SIZE_THRESHOLD`), the sender automatically splits it across multiple WebSocket
frames. Each frame carries pagination metadata in the envelope.

### Envelope Fields

The `ServiceEnvelope` gains an optional `pagination` object (omitted for single-message
reports):

```json
{
  "protocol_version": 1,
  "seq": 42,
  "pagination": {
    "report_id": "550e8400-e29b-41d4-a716-446655440000",
    "page": 1,
    "total_pages": 3
  },
  "type": "discovery_results",
  "data": { "..." }
}
```

| Field | Type | Description |
| --- | --- | --- |
| `report_id` | UUID | Groups all pages of the same logical report. |
| `page` | u32 | 1-based page number. |
| `total_pages` | u32 | Total number of pages (known upfront by the sender). |

### Pagination Limits

| Constant | Value | Description |
| --- | --- | --- |
| `PAGINATION_SIZE_THRESHOLD` | 786,432 (768 KB) | Payloads above this size are split into pages. |
| `MAX_REPORT_PAGES` | 50 | Maximum pages per report. |
| `MAX_PENDING_REPORTS_PER_CONNECTION` | 10 | Maximum concurrent in-flight paginated reports per connection. |
| `REPORT_TOTAL_TIMEOUT` | 300 seconds | Maximum wall-clock time for all pages of a report. |
| `REPORT_IDLE_TIMEOUT` | 15 seconds | Maximum time between consecutive pages. |

All limits are defined in `crates/shared/wire/src/limits.rs`.

### How It Works

**Sender side (`ControllerConnection::send_auto_paginate`):**

1. Serialize the full payload and check its size against `PAGINATION_SIZE_THRESHOLD`.
2. If under the threshold, send as a single message with no `pagination` field (zero overhead).
3. If over the threshold, split the payload's primary `Vec` field across pages. Each item
   (e.g. each `DiscoveryPluginResult`) stays whole -- never split across pages.
4. Assign a random `report_id` (UUID v4) and stamp each page with `page` / `total_pages`.
5. Send each page as a separate WebSocket text frame with its own sequence number.

**Controller side (`ReportTracker`):**

1. Each page is processed immediately and dropped (no payload buffering).
2. A lightweight `ReportTracker` (per-connection, ~200 bytes per pending report) records
   which pages have arrived.
3. For `discovery_results`, the accumulated discovered-software count is tracked across
   pages; the `NewSoftwareDiscovered` notification is emitted only on the final page.
4. For other report types, each page's finalization (e.g. `push_software_states_for_tenant`)
   runs independently -- the controller does not defer processing.
5. Expired reports (idle timeout or total timeout exceeded) are evicted automatically.

### Design Constraints

- **No payload buffering**: The controller stores only page-arrival metadata (~200 bytes per
  report), not the payloads themselves. This prevents memory-based DDoS.
- **Per-connection state**: Pagination tracking lives in a local variable with the same
  lifetime as the WebSocket session. There is no cross-controller coordination (HA safe).
- **Snapshot semantics preserved**: Each `DiscoveryPluginResult` stays whole on a single
  page. Since deactivation of missing packages is per-plugin-config, it runs correctly on
  each page independently.

### Paginatable Payload Types

| Payload type | Splittable field | Into `ServiceMessage` variant |
| --- | --- | --- |
| `DiscoveryResultsPayload` | `results: Vec<DiscoveryPluginResult>` | `discovery_results` |
| `VersionCheckResultsPayload` | `results: Vec<VersionCheckResult>` | `version_check_results` |
| `ReportHostsPayload` | `hosts: Vec<HostInfo>` | `report_hosts` |
| `BatchHostPackageUpdateResultPayload` | `results: Vec<BatchHostPackageUpdateResult>` | `batch_host_package_update_result` |

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
| `shutdown_timeout_seconds` | `Option<Duration>` | `#[serde(default, skip_serializing_if, with = "option_duration_seconds", rename = "shutdown_timeout_seconds")]` | Max time to wait during shutdown; Rust field is `shutdown_timeout`, serialized as seconds on the wire. Present for agents, absent for MQTT |
| `tenant_id` | `Option<Uuid>` | `#[serde(default, skip_serializing_if = "Option::is_none")]` | Tenant UUID that this service belongs to; present for tenant-scoped services (agents, SSH agents), absent for system services (MQTT, scheduler) |
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
| `SystemService` | `system_service` | Routes enrollment to the `system_services` table instead of `services`. Required for any service that requests system credentials. The MQTT bridge declares this alongside `mqtt_bridge`. See [System Services Architecture](../architecture/system-services.md). |
| `Scheduler` | `scheduler` | Marker: service is an external task scheduler. Maps to `ServiceProfile::Scheduler`. |
| `DatabaseAccess` | `database_access` | Service requires direct database access credentials. Requires `system_service`. |
| `NatsAccess` | `nats_access` | Service requires NATS connection details. Requires `system_service`. |
| `MasterKeyAccess` | `master_key_access` | Service requires the master encryption key. Requires `system_service`. |
| `CaManagement` | `ca_management` | Service can request CA certificate rotation. Requires `system_service`. |
| `UiExtensions` | `ui_extensions` | Service has UI extensions to register via `extension_register`. The controller gates `extension_register` processing on this capability. See [UI Extension Architecture](../architecture/ui-extensions.md). |
| `Other(String)` | *(any unknown string)* | Forward-compatible catch-all. Never participates in intersection. |

### Advertised Sets per Component

| Component | `software_discovery` | `update_hooks` | `graceful_shutdown` | `mqtt_bridge` | `ssh_remote` | `system_service` | `scheduler` | `database_access` | `nats_access` | `master_key_access` | `ca_management` | `ui_extensions` |
| --- | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| Controller | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ |
| Agent | ✓ | ✓ | ✓ | — | — | — | — | — | — | — | — | — |
| SSH Agent | ✓ | ✓ | ✓ | — | ✓ | — | — | — | — | — | — | ✓ |
| MQTT Bridge | — | — | ✓ | ✓ | — | ✓ | — | — | — | — | — | — |
| External Scheduler | — | — | ✓ | — | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | — |

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
the `ServiceProfile` at read time. When `system_service` is present in the capability set, the
enrollment is routed to `system_services` instead (see [System Services Architecture](../architecture/system-services.md)).

### Capability Evolution Across Versions

Services can add or drop capabilities when their binary is upgraded without re-enrollment. To support
this, the SDK sends an `update_capabilities` message automatically on every authenticated reconnect,
immediately after processing `service_settings`. The controller replaces the stored capability set
with the freshly-reported one and refreshes in-session gating flags.

**`update_capabilities` (service → controller)**

| Field | Type | Description |
| --- | --- | --- |
| `capabilities` | `BTreeSet<Capability>` | The full capability set declared by the current service binary. |

On receipt the controller:

1. Overwrites `services.capabilities` (or `system_services.capabilities`) with the new set.
2. Re-derives in-session flags such as `has_ui_extensions` without requiring reconnection.

This replaces enrollment-time persistence as the authoritative source of a service's live capability
set. The enrolled set is only used as a bootstrap value until the first `update_capabilities` message
arrives in that session.

## Forward Compatibility

The wire protocol is designed for safe rolling upgrades where controllers and services may temporarily run
different software versions.

### Compile-time forward compatibility (`#[non_exhaustive]`)

Several wire protocol enums are marked `#[non_exhaustive]` to allow adding new variants without breaking downstream
consumers:

- `ErrorCode` — new error codes may be added.
- `EnrollmentStatus` — new enrollment statuses may be added.
- `UpdateFinalStatus` — new update result statuses may be added.
- `DisconnectReason` — new disconnect reasons may be added.

Consumers matching on these enums must include a wildcard (`_`) arm to handle unknown variants gracefully.

### Runtime forward compatibility (`#[serde(other)]` on message enums)

Both `ServiceMessage` and `ControllerMessage` carry a terminal `Unknown` unit variant tagged with
`#[serde(other)]`. When serde encounters an unrecognised `"type"` field value, it deserialises the
message to `Unknown` instead of returning a hard error. This keeps the WebSocket connection alive
across rolling upgrades where one peer is newer than the other.

```json
{ "protocol_version": 1, "seq": 5, "type": "future_message_type", "data": {} }
```

The above JSON payload from a newer peer is silently decoded as `ServiceMessage::Unknown` (or
`ControllerMessage::Unknown`). Neither the controller nor the service closes the connection on receipt.

**Behaviour when an `Unknown` message is received:**

| Recipient | Action |
| --- | --- |
| Controller (from service) | Emits `tracing::warn!` and continues the event loop. |
| Service / SDK (from controller) | Emits `tracing::warn!` and continues the event loop. |

**NATS publication:** `ControllerMessage::Unknown` is excluded from NATS publication
(`is_nats_publishable()` returns `false`). Its payload cannot be forwarded because the data has
already been discarded by serde.

**Sequence counter:** The sequence number on an `Unknown` message is still validated and consumed.
Both sides keep their counters in sync even when individual messages cannot be interpreted.

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
data changes. It is also published to NATS (when configured) for cross-controller delivery (contains no
credentials). MQTT services filter by `tenant_id`.

**Push triggers:**

- Version check results received from an agent
- An update is triggered (REST, MQTT command, or scheduler) — sets `update_in_progress: true`
- An agent sends `update_started` (status transitions to `in_progress`) — `update_in_progress` stays `true`
- An update result (completed or failed) is received — clears `update_in_progress: false`
- An MQTT service first connects and receives its tenant assignments
- A host package batch update is triggered or completed

```json
{
  "protocol_version": 1,
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
          "update_available": true,
          "update_in_progress": false,
          "release_url": "https://github.com/owner/repo/releases/tag/v1.3.0",
          "release_notes": "## What's New\n- Feature A\n- Bug fix B"
        }
      ]
    }
  ],
  "host_package_hosts": [
    {
      "host_id": "770e8400-e29b-41d4-a716-446655440003",
      "hostname": "my-host",
      "pending_count": 3,
      "total_count": 42,
      "update_in_progress": false
    }
  ]
}
```

Each item in `items` corresponds to one enabled, non-deactivated software item whose `discovery_state` is
`null` or `"approved"`. Each `hosts` entry contains the version data for one host that tracks the software
item. An empty string in `installed_version` or `latest_version` means the version is not yet known.

The `hosts` entries use the following fields:

| Field | Type | Description |
| --- | --- | --- |
| `host_id` | UUID | Host UUID |
| `hostname` | String | Machine hostname |
| `friendly_name` | String | User-defined display name for the host |
| `installed_version` | `Option<String>` | Currently installed version (omitted if unknown) |
| `latest_version` | `Option<String>` | Latest available version (omitted if unknown) |
| `update_available` | bool | `true` when `latest_version` differs from `installed_version` |
| `update_in_progress` | bool | `true` while an `update_history` record with status `pending` or `in_progress` exists for this `(host_id, software_item_id)` pair; defaults to `false` when absent (older controller) |
| `release_url` | `Option<String>` | URL to the upstream release page (omitted when unavailable) |
| `release_notes` | `Option<String>` | Full release notes or changelog text (omitted when unavailable) |

`release_url` and `release_notes` are populated only when the plugin fetches release metadata (e.g. GitHub
Releases). They are absent (`null` / omitted) for plugins that track only version numbers.

### `host_package_hosts` field

The `host_package_hosts` field (defaults to `[]` when absent — older controllers omit it) carries one
`MqttHostPackageHostState` entry per host that has at least one tracked host package for the tenant.

| Field | Type | Description |
| --- | --- | --- |
| `host_id` | UUID | Host UUID |
| `hostname` | String | Machine hostname |
| `friendly_name` | String | User-defined display name for the host |
| `pending_count` | u32 | Number of packages where both versions are known and differ |
| `security_pending_count` | u32 | Number of packages with `update_category = "security"` where both versions are known and differ |
| `total_count` | u32 | Total number of enabled, non-deactivated packages tracked for this host |
| `update_in_progress` | bool | `true` while a `host_package_update_history` record is `pending` or `in_progress` for this host |

The MQTT service publishes these to retained topics per host. For the **all-packages** entity:

- `{prefix}/hosts/{host_id}/state` — `"N updates pending"` (when `pending_count > 0`) or `"up-to-date"`
- `{prefix}/hosts/{host_id}/latest_version` — always `"up-to-date"`
- `{prefix}/hosts/{host_id}/attributes` — `{"in_progress": bool, "pending_count": N}`

And for the **security updates** entity:

- `{prefix}/hosts/{host_id}/security/state` — `"N security updates pending"` or `"up-to-date"`
- `{prefix}/hosts/{host_id}/security/latest_version` — always `"up-to-date"`
- `{prefix}/hosts/{host_id}/security/attributes` — `{"in_progress": bool, "pending_count": N}`

When `ha_discovery = true`, the MQTT service also publishes HA discovery configs for both per-host
`update` entities (both disabled by default). See [Home Assistant Integration](../end-user/home-assistant-mqtt.md).

When `ha_discovery = true` for an MQTT client, the MQTT service publishes HA discovery configs and retained
state topics from this payload. The `update_in_progress` field is published to the
`{prefix}/update/{item_id}/{host_id}/attributes` MQTT topic as `{"in_progress": true/false}`, which
Home Assistant uses to display a live installing indicator on the `update` entity. See
[Home Assistant Integration](../end-user/home-assistant-mqtt.md).

## `mqtt_trigger_update` Payload

Sent by the MQTT service to the controller when a Home Assistant user presses **Install** on a tracked
software item. The controller validates the request and dispatches `execute_update` to the appropriate
agent. On validation failure the controller sends `error` back to the MQTT service (soft error — the
WebSocket connection is not closed).

```json
{
  "protocol_version": 1,
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

## `mqtt_trigger_host_package_update` Payload

Sent by the MQTT service to the controller when a Home Assistant user presses **Install** on a
per-host packages or security updates entity. The controller finds all qualifying outdated host
packages for the host and dispatches a single `execute_batch_host_package_update` to the agent.

```json
{
  "protocol_version": 1,
  "seq": 3,
  "type": "mqtt_trigger_host_package_update",
  "tenant_id": "550e8400-e29b-41d4-a716-446655440001",
  "host_id": "770e8400-e29b-41d4-a716-446655440003",
  "mqtt_client_id": "880e8400-e29b-41d4-a716-446655440004",
  "security_only": false
}
```

| Field | Type | Default | Description |
| --- | --- | --- | --- |
| `tenant_id` | UUID | — | Tenant scope — must match an assigned tenant for this MQTT service |
| `host_id` | UUID | — | The host whose outdated packages should be updated |
| `mqtt_client_id` | UUID | — | The MQTT client that received the Install command; stored as `actor_id` in `host_package_update_history` |
| `security_only` | bool | `false` | When `true`, only packages with `update_category = "security"` are included in the batch. Set automatically by the MQTT service when the security updates entity is triggered. |

The controller:

1. Validates that `tenant_id` is assigned to this MQTT service instance.
2. Loads all enabled, non-deactivated host packages where `installed_version != latest_version` (both
   must be known). When `security_only = true`, further filters to `update_category = "security"`.
3. Guards against a concurrent batch already `pending` or `in_progress` for this host.
4. Creates one `update_batch` and one `host_package_update_history` row per outdated package (both
   inside a transaction).
5. Groups packages by `plugin_config_id` and sends one `execute_batch_host_package_update` per group
   to the agent.
6. Immediately pushes a `software_states` message so MQTT/HA reflects `update_in_progress: true`.

The controller responds with an `error` message (no WS close) in the following cases:

- `tenant_id` is not assigned to this MQTT service instance.
- No qualifying outdated packages exist for the host (nothing to do).
- A batch is already `pending` or `in_progress` for this host.
- The host has no connected agent.

The resulting `update_batch` record has `actor_type = "mqtt"`, `actor_id = <mqtt_client_id>`, and
`batch_type = "host_package"`. Each `host_package_update_history` row gets the same actor attribution.

## Controller–Controller Messages (NATS only)

Some `ControllerMessage` variants are exchanged between controller instances via NATS JetStream rather than
directly over a WebSocket. These messages are published to the `uptrakit.events.controller` subject.

### `request_crl_renewal`

Published by:

- A controller that just revoked a certificate (immediately after `revocation_notify.notify_one()`).
- The `CrlRenewal` scheduler task via `SchedulerNotifier::signal_crl_renewal()` (embedded and external
  scheduler modes).

Consumed by:

- All controller instances that receive the NATS event; each fires `revocation_notify.notify_one()` to
  trigger a local CRL rebuild.

```json
{
  "protocol_version": 1,
  "seq": 1,
  "type": "request_crl_renewal"
}
```

The payload is empty — no additional fields are required. The consuming controller rebuilds its CRL from
the database (all revoked certificates for each active CA).

See [PKI and Certificate Lifecycle — CRLs](../security/pki-certificates.md#crls) for the full three-path
rebuild model.

### `request_ca_rotation`

Published by the external scheduler when the active CA certificate enters the 6-month rotation window.
Consumed by controllers to trigger `ca_rotation_trigger.notify_one()`.

```json
{
  "protocol_version": 1,
  "seq": 1,
  "type": "request_ca_rotation",
  "reason": "CA certificate approaching expiry (detected by external scheduler)"
}
```

## Batch host package update messages

### `execute_batch_host_package_update` (controller -> agent)

> **Security note:** When published to NATS JetStream, plugin config fields in
> this message are AES-256-GCM encrypted using the shared master key. Receiving
> controllers decrypt the configs before delivering to agents. See
> [NATS Integration — Plugin Config Protection](../development/nats-integration.md#plugin-config-protection).

Triggers a batch update for host packages grouped by plugin type. The agent calls
`plugin.execute_batch_update()` to execute a single bulk command (e.g., `apt-get upgrade`).

```json
{
  "protocol_version": 1,
  "seq": 1,
  "type": "execute_batch_host_package_update",
  "host_machine_id": "abc-123",
  "batch_id": "550e8400-...",
  "plugin_type": "package_manager_apt",
  "plugin_config": { "...merged config..." },
  "updates": [
    {
      "host_package_id": "uuid",
      "update_history_id": "uuid",
      "package_identifier": "nginx",
      "to_version": "1.24.0",
      "release_info": null
    }
  ],
  "pre_update_hooks": [],
  "post_update_hooks": [],
  "timeout_seconds": 600
}
```

### `batch_host_package_update_result` (agent -> controller)

Reports per-package outcomes after a batch update completes.

```json
{
  "protocol_version": 1,
  "seq": 1,
  "type": "batch_host_package_update_result",
  "batch_id": "550e8400-...",
  "results": [
    {
      "host_package_id": "uuid",
      "update_history_id": "uuid",
      "status": "completed",
      "output": "...command output...",
      "installed_version": "1.24.0",
      "error": null
    }
  ]
}
```

See [Host Packages Architecture](../architecture/host-packages.md) for the full entity design.

## `report_plugin_config` / `report_plugin_config_response`

Services can report plugin configurations to the controller at runtime. This is used by the SSH
agent to register a Proxmox VE plugin configuration after detecting a PVE node during bootstrap.

### `report_plugin_config` (service -> controller)

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `type` | string | Yes | `"report_plugin_config"` |
| `seq` | integer | Yes | Monotonically increasing sequence number |
| `request_id` | string | Yes | Unique request ID for correlation (max 64 chars) |
| `plugin_type` | string | Yes | Plugin type identifier, e.g. `"proxmox"` (max 64 chars) |
| `name` | string | Yes | Human-readable config name (max 128 chars) |
| `config` | object | Yes | Plugin-specific configuration (validated by the plugin) |

### `report_plugin_config_response` (controller -> service)

| Field | Type | Required | Description |
| --- | --- | --- | --- |
| `type` | string | Yes | `"report_plugin_config_response"` |
| `seq` | integer | Yes | Monotonically increasing sequence number |
| `request_id` | string | Yes | Correlates with the original request |
| `success` | boolean | Yes | Whether the config was created or found |
| `plugin_config_id` | string | No | ID of the plugin configuration (on success) |
| `error` | string | No | Error message (on failure) |

The controller validates the config via `PluginOps::validate_config()`, checks for an existing
config with the same `(tenant_id, plugin_type, name)`, and either returns the existing ID
(idempotent) or creates a new one.

## AsyncAPI Specification

The full message schema and payload definitions are published in `crates/shared/wire/asyncapi.yaml`. Use this document
to generate clients or validate payload structures. Ensure protobuf/JSON serializers conform to the spec before
releasing.
