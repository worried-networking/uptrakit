# Architecture Overview

Uptrakit is an agent-based toolkit: the **controller** orchestrates scheduling, hosts a Web UI/API, and checks upstream versions; **services** are
outbound-only, unprivileged daemons whose behavior is defined by a set of **capabilities** (`BTreeSet<Capability>`) rather than a fixed type enum.
The three current service profiles -- local agent, MQTT bridge, and SSH agent -- are all instances of the same unified service model, differentiated
only by the capabilities they advertise during enrollment (see [Capability-Based Service Identity](#capability-based-service-identity) below, and
[SSH Agent Architecture](docs/architecture/ssh-agent.md)).

A centralised [DB-backed scheduler](docs/architecture/scheduler.md) coordinates periodic tasks (version checks, cleanup, CA rotation checks,
certificate renewal) using optimistic locking for HA-safe exactly-once execution. The scheduler can run as an **embedded feature** inside the
controller (`--features embedded-scheduler`) or as a standalone **external binary** (`uptrakit-scheduler`) that enrolls as a service and receives
infrastructure credentials (database, NATS, master key) via the wire protocol.
See [External Scheduler Deployment](docs/end-user/deployment/external-scheduler.md).

The controller supports a unified [embedded service infrastructure](docs/architecture/embedded-services.md) that allows any service
(scheduler, agent, MQTT, agent-ssh) to run inside the controller process via in-process mpsc channels. Each embedded service is
auto-provisioned in the database and registered in the `ServiceConnectionRegistry`. A capability-based coexistence policy controls
whether an embedded service yields to an external counterpart when one connects.

Similarly, the controller can embed a local agent (`--features embedded-agent`) that manages the host the controller runs on,
providing software discovery, version checks, and updates without a separate `uptrakit-agent` binary. The embedded agent is
provisioned as a tenant service under the default tenant and yields to an external agent with the same `machine_id`.
See [Embedded Services](docs/architecture/embedded-services.md#embedded-agent).

## Key references

- Detailed entity documentation: [docs/architecture/](docs/architecture/) (multi-tenancy, hosts, software items,
  update history, scheduler, autodiscovery)

- System architecture and operations: [docs/end-user/system-overview.md](docs/end-user/system-overview.md)

- API and wire protocol: [docs/api/wire-protocol.md](docs/api/wire-protocol.md) and [docs/api/http-web-api.md](docs/api/http-web-api.md)

- Autodiscovery: [docs/api/autodiscovery.md](docs/api/autodiscovery.md) and [docs/end-user/autodiscovery.md](docs/end-user/autodiscovery.md)

- Settings, authentication, and service flows: [docs/api/settings-runtime.md](docs/api/settings-runtime.md),
  [docs/api/auth-flows.md](docs/api/auth-flows.md), [docs/api/services-operations.md](docs/api/services-operations.md)

- Reverse proxy deployment: [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)

- Zero-configuration discovery (mDNS/DNS-SD): [docs/development/zeroconf-discovery.md](docs/development/zeroconf-discovery.md)

- Security architecture: [docs/security/security-architecture.md](docs/security/security-architecture.md)

- Plugin development expectations: [docs/development/plugin-guidelines.md](docs/development/plugin-guidelines.md)
- Plugin system architecture: [docs/development/plugin-system.md](docs/development/plugin-system.md)

- Command executor abstraction: [docs/development/command-executor.md](docs/development/command-executor.md)
  (includes StdioTunnel for Docker-over-SSH proxy)

## Project layout

- Rust workspace (`resolver = "3"`) under `crates/*/*` for controller, agent, SSH agent, MQTT service, plugins, shared libraries, and CLI/web API.
- Frontend is a SvelteKit SPA in `frontend/` built with Tailwind CSS and Skeleton UI.
  It can be served from the filesystem (`--static-dir` / auto-detection) or embedded into
  the controller binary at compile time via the `embed-frontend` Cargo feature for
  single-binary deployment.

## Plugins

Plugins are first-party extension modules that define how to detect installed versions, resolve latest upstream versions, and execute updates. Each
plugin crate implements the `Plugin` trait and is registered in `uptrakit-plugin-infrastructure-registry`. See
[Plugin Development Guidelines](docs/development/plugin-guidelines.md) for the extension pattern,
and [Plugin System Architecture](docs/development/plugin-system.md) for the broader design.

### Role-based plugin assignment

Each `(host, software_item)` pair can have up to one plugin per **role**, stored in the `host_software_item_plugins` table. Three roles are defined by
the `PluginRole` enum (`crates/shared/types/src/plugin_role.rs`):

| Role | Purpose |
| --- | --- |
| `detect_version` | Detects the installed version on the agent host |
| `fetch_releases` | Fetches the latest available version (controller-side or agent-side) |
| `execute_update` | Executes the actual software update on the agent host |

Each assignment carries an `execution_site` column (`auto` | `agent` | `controller`) that determines where the operation runs.
Each assignment also has a `plugin_type` column and an optional `plugin_config_id` (nullable -- assignments without a
named profile use only type settings and per-assignment config). Plugins declaring the
`ControllerSideFetchReleases` capability (GitHub, Docker) have their `fetch_releases` executed on the controller by default when `execution_site` is
`auto`. Local package-index plugins (Homebrew, APT) always run agent-side. Per-host version tracking (`installed_version`, `latest_version`) lives on
`host_software_items`; there is no separate `available_versions` table.

### Plugin catalogue

| Plugin type | Crate | fetch_releases site | Autodiscovery | Notes |
| --- | --- | --- | --- | --- |
| `releases_github` | `uptrakit-plugin-releases-github` | Controller (GitHub API) | No | Tracks GitHub release tags; `ControllerSideFetchReleases` capability; agent-side `execute_update` downloads release assets to a configured `install_path` with SHA-256 verification and optional pre/post install commands |
| `releases_docker` | `uptrakit-plugin-releases-docker` | Controller (Registry API) | Yes | Tracks OCI image tags; discovers containers; `ControllerSideFetchReleases` capability; `daemon` feature (default) gates bollard for local Docker ops; Docker-over-SSH via StdioTunnel proxy (no second SSH connection); controller builds exclude it |
| `package_manager_homebrew` | `uptrakit-plugin-package-manager-homebrew` | Agent (`brew info`) | Yes | macOS/Linux formulae and casks; detects host compatibility |
| `discovery_proxmox_helper_scripts` | `uptrakit-plugin-discovery-proxmox-helper-scripts` | Agent (local scripts) | Yes | PVE helper-script containers (discovery-only; emits `DiscoveryTarget` for downstream plugins; classifies GitHub, npm, and APT-managed containers) |
| `package_manager_apt` | `uptrakit-plugin-package-manager-apt` | Agent (`apt-cache madison`) | Yes | Debian/Ubuntu packages via APT; detects host compatibility; post-update reboot check |
| `package_manager_npm` | `uptrakit-plugin-package-manager-npm` | Controller (npm registry) | Yes | Globally installed npm packages; upstream versions fetched from `registry.npmjs.org`; `ControllerSideFetchReleases` capability; detects host compatibility; requires `sudo` for updates |
| `infrastructure_proxmox` | `uptrakit-plugin-infrastructure-proxmox` | Controller (PVE REST API) | No | Proxmox VE infrastructure plugin; discovers QEMU VMs and LXC containers; manual host matching; uses Extensions framework for all UI/CLI interaction; also provides agent-side modules (`pve_setup`, `guest_exec`) for PVE node detection and guest command execution during SSH agent bootstrap |

### Two-tier config model

Plugin configuration uses a two-tier model: **type settings** store tenant-level defaults per plugin type
(e.g. APT `discovery_filter`, Homebrew `package_type`) in the `plugin_type_settings` table, while **plugin
configs** store named profiles with credentials and per-profile settings. At runtime, the effective config
is resolved by merging three layers: type settings, profile config, and per-assignment config (see
[Plugin Guidelines](docs/development/plugin-guidelines.md#type-settings-vs-plugin-configs)).

Plugins with a local package index (`package_manager_homebrew`,
`discovery_proxmox_helper_scripts`, `package_manager_apt`) resolve both
installed and latest versions on the agent.
Plugins with `ControllerSideFetchReleases` (`releases_github`, `releases_docker`,
`package_manager_npm`) resolve upstream versions on the controller via public HTTP APIs,
requiring no local package index on the agent.

## Capability-based service identity

Services are identified by the capabilities they advertise, not by a fixed type enum. The former `ServiceType` enum has been removed entirely.

### Capability set replaces ServiceType

Each service sends a `BTreeSet<Capability>` in its `EnrollPayload` during enrollment. The controller persists this set as a JSON text column
(`services.capabilities`) and uses it for all routing and behavioral decisions. There is no stored "service type" anywhere in the system.

| Capability | Wire string | Description |
| --- | --- | --- |
| `SoftwareDiscovery` | `software_discovery` | Supports `discover_software` / `discovery_results` flow |
| `UpdateHooks` | `update_hooks` | Pre/post-update lifecycle hook plugin execution |
| `GracefulShutdown` | `graceful_shutdown` | Supports coordinated shutdown |
| `UpdateTracking` | `update_tracking` | Update tracking service: handles `register`, `tenant_assignments`, `release_tenants`, etc. |
| `SshRemote` | `ssh_remote` | Manages remote hosts over SSH |
| `Scheduler` | `scheduler` | Marker: service is an external task scheduler |
| `DatabaseAccess` | `database_access` | Service requires direct database access credentials |
| `NatsAccess` | `nats_access` | Service requires NATS connection details |
| `MasterKeyAccess` | `master_key_access` | Service requires the master encryption key |
| `CaManagement` | `ca_management` | Service can request CA certificate rotation |
| `ResetData` | `reset_data` | Supports reset-data protocol: truncates local data on controller broadcast |
| `WorkloadClaims` | `workload_claims` | Enables the exclusive workload claim protocol for config-key-level tenant assignment |

### ServiceProfile (derived, never stored)

`ServiceProfile` is a runtime-only enum derived from a service's capability set via `ServiceProfile::from_capabilities()` (defined in
`uptrakit-internal-wire`). It drives controller-side behavioral defaults (ping interval, shutdown timeout, human-readable label)
but is never persisted to the database.

| Profile | Key capability | Typical services | Default ping | Shutdown timeout |
| --- | --- | --- | --- | --- |
| `UpdateTracker` | `UpdateTracking` | MQTT service | 15 s | None |
| `Scheduler` | `Scheduler` | External scheduler | 60 s | 30 s |
| `Agent` | `SoftwareDiscovery` | Local agent, SSH agent | 300 s | 120 s |
| `Unknown` | (none of the above) | Future services | 300 s | 120 s |

`UpdateTracker` takes precedence over `Scheduler`, which takes precedence over `Agent`. For `Agent` profiles, the `SshRemote` capability
distinguishes SSH-backed agents from local agents in UI labels (`service_label`).

### Enrollment tokens

Multiple named enrollment tokens are stored in a dedicated `enrollment_tokens` table. Each token supports:

- **Capability scoping**: restrict which service types the token can approve (intersection matching).
- **Usage limits**: optional `max_uses` with atomic `current_uses` increment.
- **TTL**: optional `expires_at` for automatic expiration.
- **Audit trail**: `created_by_user_id` tracks who created the token; `enrollment_token_id` on the `services` table
  tracks which token enrolled each service.

Tokens are managed via the `/api/v1/enrollment-tokens` REST API (create, list, get, revoke). A wildcard token (no capability restriction)
matches any service. During enrollment, the controller iterates active tokens and verifies the provided secret against each Argon2id hash.
See [Enrollment Tokens API](docs/api/enrollment-tokens.md) for full details.

### Service connection registry

`service_connections.rs` provides a `ServiceConnectionRegistry` with a unified `register()` method that accepts any capability set, and a
`broadcast_by_capability()` method that sends a `ControllerMessage` to all connected services holding a given capability. This replaces the
former per-type registration and broadcast paths.

### Workload Claims

The `WorkloadClaimRegistry` (in `crates/ui/web-api/src/workload_claims.rs`) provides exclusive
config-key-level ownership across service instances. Each config key (e.g. `clients.{uuid}`) is
owned by exactly one service at a time, preventing duplicate work (such as duplicate MQTT publishes).

The registry maintains:

- `claims: HashMap<String, ClaimOwner>` -- config key to owner mapping
- `by_service: HashMap<Uuid, BTreeSet<String>>` -- reverse index for fast release
- `tenant_services: HashMap<Uuid, BTreeSet<Uuid>>` -- tenant routing index
- `pending_desires: HashMap<Uuid, BTreeMap<String, Uuid>>` -- rejected claims for proactive re-grant

Cross-controller synchronization uses NATS `WorkloadClaimAnnouncement` messages. Conflict resolution
is timestamp-based: earlier `claimed_at` wins, with `service_id` as tiebreaker.

See [Workload Claim Protocol](docs/api/wire-protocol.md#workload-claim-protocol) for wire-level details.

### Database schema

- `services.capabilities` -- JSON text column holding a serialized `Vec<Capability>`
  (e.g. `["software_discovery","update_hooks","graceful_shutdown"]`).
- Cross-controller event routing uses capability strings (`"software_discovery"`, `"update_tracking"`, etc.) via NATS
  JetStream subjects when NATS is configured. See [Cross-Controller Communication](docs/development/cross-controller-comm.md).

### Credential delivery

Services advertising credential capabilities (`DatabaseAccess`, `NatsAccess`, `MasterKeyAccess`) automatically
receive a `ServiceCredentials` message after mTLS authentication. The controller populates the payload based on
the service's capability set:

| Capability | Field populated | Condition |
| --- | --- | --- |
| `DatabaseAccess` | `db_url` | Always (controller has a database) |
| `NatsAccess` | `nats_url` | Only if NATS is configured on the controller |
| `MasterKeyAccess` | `master_key_hex` | Only if encryption is enabled on the controller |

`ServiceCredentials` is **never** published to NATS — it is delivered exclusively via the authenticated
WebSocket connection. See [Secrets and Encryption](docs/security/secrets-and-encryption.md).

### REST API

`ServiceResponse` returns `capabilities` (list of snake_case strings) and `service_label` (human-readable, derived from `ServiceProfile`).
Filtering uses `?capability=software_discovery` on the list endpoint. There is no `service_type` field in any request or response.

## Two-tier service model

The controller manages two independent service tiers. Enrollment routing happens at the WebSocket
level based on the presence of `Capability::SystemService` in `EnrollPayload.capabilities`.

| Tier | Table | Scoped to | REST path | Examples |
| --- | --- | --- | --- | --- |
| Tenant services | `services` | `tenant_id` | `/api/v1/services` | Agents, SSH agents |
| System services | `system_services` | Global | `/api/v1/system-services` | MQTT bridge, external scheduler |

System services have no `tenant_id` and no `enrollment_token_id`. They authenticate using a single
global token stored AES-256-GCM encrypted in the `settings` table. Certificates are stored in a
separate `system_service_certificates` table (FK to `system_services`, not `services`).

A credential guard in the tenant enrollment path rejects any service that requests
`database_access`, `nats_access`, `master_key_access`, or `ca_management` without the
`system_service` capability, preventing tenant services from receiving infrastructure secrets.

See [System Services Architecture](docs/architecture/system-services.md) for the full design.

## Wire protocol

All services connect to `/api/v1/ws/service` over mTLS and exchange shared `ServiceMessage`/`ControllerMessage` enums. The
AsyncAPI definition lives at `crates/shared/wire/asyncapi.yaml` and is described in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).

## Database backend support

Uptrakit supports three database backends via SeaORM:

| Backend | Feature flag | Notes |
| --- | --- | --- |
| SQLite | `db-sqlite` (default) | In-memory or file-based; single-server deployments |
| PostgreSQL | `db-postgres` | Multi-server deployments; HA with external scheduler |
| MariaDB/MySQL | `db-mysql` | MariaDB 11+; index key limits and partial index workarounds apply |

All migrations use the same schema definitions across backends with backend-specific code paths
where needed (e.g., table recreation vs. ALTER TABLE, partial indexes vs. composite indexes).
See [Database Migrations](docs/development/database-migrations.md#mysqlmariadb-compatibility-workarounds)
for MariaDB-specific migration patterns.

## Notification subsystem

The controller includes a channel-agnostic notification subsystem that delivers event-driven alerts through
pluggable notification plugins. Event producers emit `NotificationEvent` values (internal, channel-agnostic); a
fire-and-forget `NotificationDispatcher` matches them against tenant-scoped rules, builds a
`DeliveryMessage`, and hands it to the appropriate plugin implementation for delivery.

Notification plugins live under `crates/plugins/notifications/` with one crate per concern:

| Crate | Package | Purpose |
| --- | --- | --- |
| `core/` | `uptrakit-notification-plugin-core` | `DeliveryMessage`, `MessageAction`, `NotificationPluginError`, `escape_html()` |
| `webhook/` | `uptrakit-notification-plugin-webhook` | Webhook plugin (SSRF-safe DNS, HMAC-SHA256 signatures); implements `PluginBase` + `NotificationTransportPlugin` |
| `telegram/` | `uptrakit-notification-plugin-telegram` | Telegram plugin (inline keyboard for actionable notifications); implements `PluginBase` + `NotificationTransportPlugin` |
| `email/` | `uptrakit-notification-plugin-email` | Email plugin (SMTP via mail-send); implements `PluginBase` + `NotificationTransportPlugin` |

Each notification plugin is behind its own Cargo feature flag in the unified `uptrakit-plugin-infrastructure-registry`
crate (`webhook` -- default on, `telegram`, `email`). Notification plugins are registered in the `PluginRegistry`
via `with_notifications(config)` and accessed through `PluginOps::notification_transport(channel_type)`.

### Adding a new notification plugin

1. Create a new crate under `crates/plugins/notifications/<name>/`.
2. Implement `PluginBase` + `NotificationTransportPlugin` traits from `uptrakit-plugin-infrastructure-core`.
3. Add an `extensions.rs` module with `handle_action()` for settings CRUD, channel listing,
   and callback handling. Use the shared `list_channels` helper from `notification-plugin-core`.
4. Register the plugin in `PluginRegistry::with_notifications()` behind `#[cfg(feature = "...")]`.
5. Add the feature in `crates/plugins/infrastructure/registry/Cargo.toml`.
6. Propagate the feature: `web-api/Cargo.toml` -> `controller/Cargo.toml`.

Supported event types: `update_available`, `update_completed`, `update_failed`, `new_software_discovered`,
`new_service_enrolled`, `ca_rotated`, `batch_update_completed`, `batch_update_partially_completed`.
Rules can be scoped to a specific host, software item, and/or plugin type.
`UpdateAvailable` events produce actionable notifications — on Telegram, an inline keyboard button triggers
the update via a callback endpoint.

Channel configuration is stored encrypted (`EncryptedString`) in the `notification_channels` table. Secrets
are masked in API responses. Delivery history is recorded in the `notification_log` table with status tracking
(`pending` → `delivered` | `failed`).

### UI extension integration

The unified plugin registry also provides `ExtensionManifest` and `ActionDef` entries for each enabled
notification plugin. These are registered as `ExtensionOwner::Notification`
in the `ExtensionRegistry` at startup, producing per-transport channel management tabs in the
Settings page. Each notification plugin owns its own `extensions.rs` module with a
`handle_action()` function that handles settings CRUD, channel listing, and callback handling.
A shared `list_channels` helper in `notification-plugin-core` provides pagination and config
flattening that all plugins share. SMTP settings are managed through extension actions
(`get_smtp`, `save_smtp`) within the email plugin using raw-key settings store functions.

Notification rules and delivery log are built-in Svelte components (not extensions) with direct
REST API calls.

See [Notifications Development](docs/development/notifications.md), [Notifications API](docs/api/notifications.md),
and [Notifications Security](docs/security/notifications-security.md).

## Audit log subsystem

The controller records all authenticated HTTP requests through a pluggable audit log subsystem. A fire-and-forget
`AuditLogDispatcher` (same `mpsc::UnboundedSender` pattern as `NotificationDispatcher`) enqueues `AuditEntry` values
from the `audit_log` middleware (runs inside `require_auth`) and persists them through one or more backends.

Two tables store entries: `audit_logs` (tenant-scoped, no FK on `tenant_id` for compliance) and `system_audit_logs`
(global, no tenant column). The `DatabaseBackend` routes entries based on `AuditEntry.tenant_id`. An optional
separate database (`--audit-log-db-url`) provides physical isolation of audit data.

Backends are selected via the repeatable `--audit-log-backend` CLI flag (`db`, `journald`, `none`). Multiple
backends fan out concurrently via `MultiplexBackend`. The `journald` backend is feature-gated and emits structured
tracing events with target `uptrakit_audit`.

A global filter (`--audit-log-filter`: `all` | `mutations` | `none`) controls which requests are logged, with
per-tenant overrides via the `audit_log.filter` setting key. Retention cleanup runs as a scheduled task
(`AuditLogCleanup`, default 90 days).

The `uptrakit-audit-log` crate (`crates/shared/audit-log/`) houses core types, backends, and the dispatcher.
See [Audit Logs Development](docs/development/audit-logs.md) and [Audit Logs Security](docs/security/audit-logs.md).

## Batch updates

The controller supports batch updates — triggering multiple updates in a single request with sequential per-host
dispatch. Two modes: host-wide (update all outdated items on a host, optionally filtered by update category) and
item-wide (roll out a software item version to multiple hosts). The `update_batches` table tracks batch metadata;
individual updates link back via `update_history.batch_id`. After each update completes on a host, the controller
dispatches the next pending update for that host within the same batch.

An `UpdateCategory` enum (`security`, `bugfix`, `feature`, `unknown`) classifies available updates based on upstream
source metadata. The APT plugin detects security updates from `*-security` repositories. Categories are stored on
`host_software_items` and `update_history` and exposed in API responses.

A `BatchProgressBroadcaster` (`crates/ui/web-api/src/batch_progress_broadcaster.rs`) provides per-batch
`tokio::sync::broadcast` channels for real-time SSE streaming of batch progress events.

See [Update History Entity](docs/architecture/update-history-entity.md) and
[Batch Update Endpoints](docs/api/http-web-api.md#batch-update-endpoints).

## Batch actions (group operations)

All management resources support batch actions via `POST /api/v1/{resource}/batch` endpoints. A
single request carries an action name and up to 100 UUIDs. The server processes each item
independently and returns partial-success results — items that fail (wrong state, not found) do
not block successful items. No new database tables are required; batch operations use existing
columns (status updates, soft-deletes). Side effects (WebSocket notifications, admin events, CRL
renewal) fire per succeeded item, same as the single-item endpoints.

Covered resources: services, system services, software items, hosts, host tags,
software ignore rules, and plugin configs. Extensions can mark `ActionDef` as batch-capable
via `batch_action: true`. See [docs/api/batch-actions.md](docs/api/batch-actions.md).

The frontend provides multi-select checkboxes on all list pages with a shared `BatchActionBar`
toolbar and `BatchResultDialog` for partial-success feedback. See
[docs/development/frontend-components.md](docs/development/frontend-components.md#batch-action-components).

## Host tags

Hosts support user-defined tags for organization and classification. Tags are tenant-scoped metadata
with a name, hex color, and optional description. A many-to-many relationship via the
`host_tag_assignments` join table links tags to hosts. Tags use soft-delete (`deactivated_at`) with a
partial unique index ensuring unique active names per tenant. Auto-generated colors are picked from a
12-color palette when no color is specified. The `HostResponse` includes a `tags` field with slim tag
summaries, batch-loaded to avoid N+1 queries.

See [Host Tags Architecture](docs/architecture/host-tags.md) for the full entity design.

## Unified software tracking

All software -- from individually tracked applications to system-level packages -- is stored in the
unified `software_items` table. A `featured` boolean flag controls visibility: featured items (Docker,
PHS-discovered apps) appear individually in the main Software list, while non-featured items (APT,
Homebrew, npm) appear as aggregated per-host package summaries. All discovered items are created
immediately with `enabled: true` -- there is no pending state or approval workflow.

For featured items, role-based plugin assignments live in `host_software_item_plugins`. For non-featured
items, `plugin_config_id` and `package_identifier` are stored directly on the `host_software_items`
junction row. Batch updates are supported via `Plugin::execute_batch_update()`. The `HostResponse`
includes a `HostUpdateSummary` with aggregate update counts.

See [Unified Software Tracking](docs/architecture/unified-software-tracking.md) for the full design.

## Update output streaming (SSE)

The controller provides a browser-facing SSE endpoint (`GET /api/v1/update-history/{id}/output/stream`) for real-time
update log tailing. This is a standard HTTP streaming endpoint — separate from the WebSocket wire protocol between
services and the controller.

An in-process `UpdateOutputBroadcaster` (`crates/ui/web-api/src/update_output_broadcaster.rs`) maintains per-update
`tokio::sync::broadcast` channels. When an agent sends `UpdateOutput` messages over WebSocket, the handler fans lines
out to both the database and any SSE subscribers. The SSE handler replays stored lines on connect and then streams new
lines in real time. The frontend renders output in an xterm.js terminal with full ANSI color support.

## Interactive updates (feature-gated)

When compiled with the `interactive` Cargo feature, agents can allocate a PTY for update processes
and keep stdin open for bidirectional terminal I/O. The controller exposes a WebSocket endpoint at
`GET /api/v1/update-history/{id}/interactive` that relays stdin data from admin clients to the
agent's PTY and streams output back. This enables admins to respond to package manager prompts
(e.g., APT config file conflicts) and send signals (Ctrl+C) to running updates.

The feature is gated behind an `interactive` Cargo feature that propagates through the crate chain:
`uptrakit-command` → `uptrakit-agent-core` → `uptrakit-agent`/`uptrakit-agent-ssh` → `uptrakit-web-api`
→ `uptrakit-controller`. Wire protocol types are unconditional for forward compatibility. Agents
advertise the `InteractiveUpdates` capability when compiled with this feature.

See [Interactive Updates API](docs/api/interactive-updates.md) and
[Interactive Updates Development](docs/development/interactive-updates.md) for details.

## Admin events SSE

The controller pushes lightweight admin events to the frontend via `GET /api/v1/events/stream`.
An `EventBroadcaster` (`crates/ui/web-api/src/event_broadcaster.rs`) maintains per-tenant
`tokio::sync::broadcast` channels (capacity 512). Route handlers and WebSocket message processors
fire events after state mutations; the broadcaster is fire-and-forget (no-op when no subscribers).
Frontend pages subscribe to relevant event types and refetch data on demand, replacing the
previous 30–60s polling intervals with a 5-minute safety-net fallback.

A similar `DeviceFlowBroadcaster` provides SSE delivery for device authorization
(`GET /api/v1/auth/device/stream`), enabling the CLI to receive tokens instantly on approval
instead of polling every 5 seconds.

See [SSE Events — Developer Guide](docs/development/sse-events.md) for implementation details.

## Web-API crate split

The HTTP API is composed of three independent crates under `crates/ui/`:

- **`uptrakit-web-api-auth`** (`crates/ui/web-api-auth/`): authentication subsystem (~5k lines).
  Contains the `auth` module (JWT, sessions, OIDC, tokens, permissions, registration), `SettingKey`,
  and `settings_store`. Raw-key settings functions (`upsert_setting_raw`, `load_settings_by_prefix`,
  etc.) live in `uptrakit-shared-db::raw_settings` so that notification plugins can access settings
  without depending on `web-api-auth`. The `oidc` feature gates `openidconnect` dependency. Authorization uses
  32 granular permissions grouped into 8 built-in roles with 5 access presets. See
  [Authentication and Authorization](docs/security/auth-and-authorization.md).

- **`uptrakit-web-api-queries`** (`crates/ui/web-api-queries/`): database query logic (~10.5k lines).
  Contains all query modules, `TenantDb` (tenant-scoped DB wrapper), and the `ServiceNotifier` trait
  (decouples queries from the concrete `NotificationService`). DB feature flags (`db-sqlite`,
  `db-postgres`, `db-mysql`) are forwarded from the parent crate.

- **`uptrakit-web-api`** (`crates/ui/web-api/`): HTTP API layer (~27k lines). Contains routes,
  middleware, `AppState`, router, `NotificationService`, PKI utilities, and SSE broadcasters.
  Depends on both sibling crates and re-exports their public APIs at the original module paths
  (`crate::auth::*`, `crate::queries::*`, `crate::SettingKey`, `crate::settings_store::*`)
  for backward compatibility.

The auth and queries crates have no dependency on each other, enabling parallel compilation.
Only `uptrakit-controller` depends on `uptrakit-web-api`.

## OpenAPI Client

The `uptrakit-openapi-client` crate (`crates/shared/openapi-client/`) provides a typed HTTP
client for the Uptrakit web API. It wraps `reqwest` with compile-time type-safe endpoint
methods using the shared request/response types from `uptrakit-web-api-types`. The CLI crate
uses this client exclusively for all API communication.
See [OpenAPI Client](docs/development/openapi-client.md) for details.

## Optional OIDC support

OpenID Connect (OIDC) authentication is gated behind the `oidc` Cargo feature on `uptrakit-web-api`,
which propagates to `uptrakit-web-api-auth/oidc` (enabled by default, propagated via the controller's
`oidc` feature). When disabled, the
`openidconnect` crate and its heavy transitive tree (`oauth2`, `reqwest` 0.12, RSA/EC crypto,
`rand` 0.8, `thiserror` v1, `base64` 0.21) are excluded from the build. OIDC DB entities
(`oidc_provider`, `user_oidc_link`), `AuthMethod::Oidc`, and the `require_token_for_oidc`
setting field remain unconditional — they carry no `openidconnect` types. Without the feature,
OIDC API routes, OpenAPI schemas, and rate-limit entries are omitted, and disabling password
authentication returns an error stating OIDC support is not available.

Key design decisions:

- **Hand-written, not code-generated** -- the shared types already exist in `uptrakit-web-api-types`; a code generator would duplicate them.
- **Re-exports all types** -- downstream crates import types via
  `uptrakit_openapi_client::types::*` and `uptrakit_openapi_client::Uuid`
  rather than depending on `uptrakit-web-api-types` or `uuid` directly.
- **UUID type safety** -- all entity ID parameters use `&Uuid` (not `&str`)
  and all response ID fields are `Uuid` (not `String`), providing compile-time
  validation of identifiers across the API boundary.
- **Typed error handling** -- `ClientError` enum with variants for HTTP, JSON,
  API, rate-limiting, not-found, and authentication errors, using
  `rootcause::Report` for context propagation.

## Service SDK

The `uptrakit-service-sdk` crate (`crates/shared/service-sdk/`) provides shared infrastructure for building Uptrakit services:

- **Lifecycle**: The `ServiceHandler` trait and `run_service_lifecycle()` function encapsulate the full bootstrap-enrollment-reconnect flow. New
  services declare two associated constants (`DIR_NAME`, `SERVICE_LABEL`) and override the `capabilities()` method to return their
  `BTreeSet<Capability>`. The SDK sends this capability set in `EnrollPayload` during enrollment. Callbacks (`on_connected`, `on_message`,
  `on_shutdown`, etc.) provide directory setup, identity management, CA bootstrap, enrollment with backoff, and reconnection with backoff for free.
  See [Service Lifecycle](docs/development/service-lifecycle.md).
- **Event Loop**: The SDK owns the unified `tokio::select!` loop that handles ping/pong, certificate renewal, CA staleness checks, signal handling,
  and close-reason dispatch. Services inject custom behaviour through `ServiceHandler` callbacks (`poll_service_event`, `on_service_event`).
- **Main Helpers**: `init_crypto()`, `print_build_info()`, `default_resolve_shutdown()`, and
  `run_lifecycle_and_handle_errors()` reduce `main()` boilerplate. Each binary owns its own `init_tracing()` function
  using `tracing-subscriber` directly — the SDK does not configure the global dispatcher.
  `default_resolve_shutdown()` maps `ShutdownCause` to `(DisconnectReason, LoopOutcome)` for standard binaries.
- **Signal Handling**: Cross-platform `SignalWatcher` for `SIGINT`, `SIGTERM`, and `SIGHUP`.
- **Enrollment**: WebSocket-based enrollment with certificate issuance.
- **Identity**: Service identity state management (service ID, enrollment secret, certificate, private key).
- **TLS/CA**: TLS connector builders (server-only and mTLS), CA bootstrap (cached, file, PKI endpoint, TOFU, system trust), CA staleness checks.
- **ControllerConnection**: Shared authenticated WebSocket connection with split-stream architecture: the WebSocket is
  split into read/write halves, with a background writer task draining a bounded channel (`WRITE_CHANNEL_CAPACITY = 128`).
  `send()` serializes and pushes to the channel (non-blocking), `send_best_effort()` uses `try_send()` (drop on full),
  and error signaling uses `Arc<AtomicBool>`. Envelope serialization, sequence validation, Ping/Pong handling, and
  close-frame reason tracking are built in. Used by all services regardless of capability set. Sequence validation is
  performed before full message deserialization to ensure unrecognized messages do not cause sequence mismatches. The
  `send_paginated` method automatically splits large report payloads (discovery results, version checks, host reports,
  batch updates) into pages when they exceed the controller-provided per-page limits.
- **CertificateRenewalHandler**: Handles certificate lifecycle messages (`CaBundleUpdated`, `RequestCertRenewal`, `Certificate`) automatically in the
  event loop. Also provides shared renewal timer helpers (`create_renewal_sleep`, `update_renewal_schedule`, `compute_renewal_delay`).
- **Backoff**: Exponential backoff with jitter for reconnection delays.
- **CLI**: Common CLI arguments (`--url`, `--config-dir`, `--state-dir`, `--force-enroll`, etc.).
- **Extension Handling**: The `ServiceHandler` trait includes an `on_extension_request` default method.
  The event loop dispatches `ControllerMessage::ExtensionRequest` to this callback automatically.

## UI Extensions

The extensions framework enables connected services and plugins to dynamically extend the
UI with custom pages, panels, context menu actions, and table columns. Each extension is
described by an `ExtensionManifest` (defined in `crates/shared/extension-framework/src/lib.rs`).

Two registration models coexist:

- **Compile-time (plugins)**: `PluginOps::extension_manifests()` returns manifests at
  controller startup. Actions are dispatched in-process via
  `PluginOps::handle_extension_action()`. The `PluginOps` trait is defined in
  `infrastructure-core` (feature `plugin-ops`) and implemented by `PluginRegistry` in
  `infrastructure-registry`. The Proxmox VE plugin uses this model.
- **Runtime (services)**: Services register via `ServiceMessage::ExtensionRegister` over
  WebSocket. Actions are proxied to the service via request/response correlation.

- **Extension Registry** (`crates/ui/web-api/src/extension_registry.rs`): tracks active
  manifests and their provider sets. Services register extensions via
  `ServiceMessage::ExtensionRegister` after connecting. Same extension ID from the same
  `service_app_name` is deduplicated; from different app names is rejected.
- **Extension Proxy** (`crates/ui/web-api/src/extension_proxy.rs`): proxies action
  invocations to connected services using request/response correlation via oneshot channels.
  Supports configurable timeouts per action.
- **REST API**: Three endpoints under `/api/v1/extensions` — list extensions, list providers,
  and invoke actions. The frontend and CLI both use these endpoints.
- **Frontend**: Schema-driven Svelte components render `DataTable`, `Form`, `KeyValue`, and
  `Actions` UI variants based on the manifest definition. Extension pages appear dynamically
  in the sidebar navigation.
- **CLI**: `uptrakit extensions list|providers|invoke` subcommands.

See [Extensions Architecture](docs/architecture/extensions.md) for the detailed design.

## Observability

All binaries use the `tracing` crate with a registry-based subscriber. Critical code paths are instrumented with
`#[tracing::instrument(skip_all)]` spans. HTTP requests carry a unique `x-request-id` header (UUID v7 generated
or preserved from the client). WebSocket and NATS envelopes propagate a `TraceContext` (W3C-compatible trace/span
IDs) for distributed tracing correlation across service boundaries. When OpenTelemetry is enabled, the existing
span tree and trace context propagation will light up automatically.
See [Tracing Conventions](docs/development/tracing.md) and [Logging](docs/development/logging.md).
