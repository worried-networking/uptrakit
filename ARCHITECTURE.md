# Architecture Overview

Uptrakit is an agent-based toolkit: the **controller** orchestrates scheduling, hosts a Web UI/API, and checks upstream versions; **services** are
outbound-only, unprivileged daemons whose behavior is defined by a set of **capabilities** (`BTreeSet<Capability>`) rather than a fixed type enum.
The three current service profiles -- local agent, MQTT bridge, and SSH agent -- are all instances of the same unified service model, differentiated
only by the capabilities they advertise during enrollment (see [Capability-Based Service Identity](#capability-based-service-identity) below, and
[SSH Agent Architecture](docs/architecture/ssh-agent.md)).

A centralised [DB-backed scheduler](docs/architecture/scheduler.md) coordinates periodic tasks (version checks, cleanup, CA rotation checks,
certificate renewal) using optimistic locking for HA-safe exactly-once execution. The scheduler can run as an **embedded feature** inside the
controller (`--features embedded-scheduler`) or as a standalone **external binary** (`uptrakit-scheduler`) that enrolls as a service and receives
infrastructure credentials (database, NATS, master key) via the wire protocol. See [External Scheduler Deployment](docs/end-user/deployment/external-scheduler.md).

## Key references

- Detailed entity documentation: [docs/architecture/](docs/architecture/) (multi-tenancy, hosts, software items, update history, scheduler, autodiscovery)

- System architecture and operations: [docs/end-user/system-overview.md](docs/end-user/system-overview.md)

- API and wire protocol: [docs/api/wire-protocol.md](docs/api/wire-protocol.md) and [docs/api/http-web-api.md](docs/api/http-web-api.md)

- Autodiscovery: [docs/api/autodiscovery.md](docs/api/autodiscovery.md) and [docs/end-user/autodiscovery.md](docs/end-user/autodiscovery.md)

- Settings, authentication, and service flows: [docs/api/settings-runtime.md](docs/api/settings-runtime.md),
  [docs/api/auth-flows.md](docs/api/auth-flows.md), [docs/api/services-operations.md](docs/api/services-operations.md)

- Reverse proxy deployment: [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)

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

Each assignment carries an `execution_site` column (`auto` | `agent` | `controller`) that determines where the operation runs. Plugins declaring the
`ControllerSideFetchReleases` capability (GitHub, Docker) have their `fetch_releases` executed on the controller by default when `execution_site` is
`auto`. Local package-index plugins (Homebrew, APT) always run agent-side. Per-host version tracking (`installed_version`, `latest_version`) lives on
`host_software_items`; there is no separate `available_versions` table.

### Plugin catalogue

| Plugin type | Crate | fetch_releases site | Autodiscovery | Notes |
| --- | --- | --- | --- | --- |
| `releases_github` | `uptrakit-plugin-releases-github` | Controller (GitHub API) | No | Tracks GitHub release tags; `ControllerSideFetchReleases` capability |
| `releases_docker` | `uptrakit-plugin-releases-docker` | Controller (Registry API) | Yes | Tracks OCI image tags; discovers containers; `ControllerSideFetchReleases` capability; `daemon` feature (default) gates bollard for local Docker ops; Docker-over-SSH via StdioTunnel proxy (no second SSH connection); controller builds exclude it |
| `package_manager_homebrew` | `uptrakit-plugin-package-manager-homebrew` | Agent (`brew info`) | Yes | macOS/Linux formulae and casks; detects host compatibility |
| `discovery_proxmox_helper_scripts` | `uptrakit-plugin-discovery-proxmox-helper-scripts` | Agent (local scripts) | Yes | PVE helper-script containers (discovery-only; emits `DiscoveryTarget` for downstream plugins; classifies GitHub, npm, and APT-managed containers) |
| `package_manager_apt` | `uptrakit-plugin-package-manager-apt` | Agent (`apt-cache madison`) | Yes | Debian/Ubuntu packages via APT; detects host compatibility; post-update reboot check |
| `package_manager_npm` | `uptrakit-plugin-package-manager-npm` | Controller (npm registry) | Yes | Globally installed npm packages; upstream versions fetched from `registry.npmjs.org`; `ControllerSideFetchReleases` capability; detects host compatibility; requires `sudo` for updates |

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
| `UpdateHooks` | `update_hooks` | Pre/post-update hook execution |
| `GracefulShutdown` | `graceful_shutdown` | Supports coordinated shutdown |
| `MqttBridge` | `mqtt_bridge` | MQTT bridge: handles `register`, `tenant_assignments`, `release_tenants`, etc. |
| `SshRemote` | `ssh_remote` | Manages remote hosts over SSH |
| `Scheduler` | `scheduler` | Marker: service is an external task scheduler |
| `DatabaseAccess` | `database_access` | Service requires direct database access credentials |
| `NatsAccess` | `nats_access` | Service requires NATS connection details |
| `MasterKeyAccess` | `master_key_access` | Service requires the master encryption key |
| `CaManagement` | `ca_management` | Service can request CA certificate rotation |

### ServiceProfile (derived, never stored)

`ServiceProfile` is a runtime-only enum derived from a service's capability set via `ServiceProfile::from_capabilities()` (defined in
`uptrakit-internal-wire`). It drives controller-side behavioral defaults (ping interval, shutdown timeout, human-readable label)
but is never persisted to the database.

| Profile | Key capability | Typical services | Default ping | Shutdown timeout |
| --- | --- | --- | --- | --- |
| `MqttBridge` | `MqttBridge` | MQTT service | 15 s | None |
| `Scheduler` | `Scheduler` | External scheduler | 60 s | 30 s |
| `Agent` | `SoftwareDiscovery` | Local agent, SSH agent | 300 s | 120 s |
| `Unknown` | (none of the above) | Future services | 300 s | 120 s |

`MqttBridge` takes precedence over `Scheduler`, which takes precedence over `Agent`. For `Agent` profiles, the `SshRemote` capability
distinguishes SSH-backed agents from local agents in UI labels (`service_label`).

### Enrollment tokens

Multiple named enrollment tokens are stored in a dedicated `enrollment_tokens` table. Each token supports:

- **Capability scoping**: restrict which service types the token can approve (intersection matching).
- **Usage limits**: optional `max_uses` with atomic `current_uses` increment.
- **TTL**: optional `expires_at` for automatic expiration.
- **Audit trail**: `created_by_user_id` tracks who created the token; `enrollment_token_id` on the `services` table tracks which token enrolled each service.

Tokens are managed via the `/api/v1/enrollment-tokens` REST API (create, list, get, revoke). A wildcard token (no capability restriction)
matches any service. During enrollment, the controller iterates active tokens and verifies the provided secret against each Argon2id hash.
See [Enrollment Tokens API](docs/api/enrollment-tokens.md) for full details.

### Service connection registry

`service_connections.rs` provides a `ServiceConnectionRegistry` with a unified `register()` method that accepts any capability set, and a
`broadcast_by_capability()` method that sends a `ControllerMessage` to all connected services holding a given capability. This replaces the
former per-type registration and broadcast paths.

### Database schema

- `services.capabilities` -- JSON text column holding a serialized `Vec<Capability>` (e.g. `["software_discovery","update_hooks","graceful_shutdown"]`).
- Cross-controller event routing uses capability strings (`"software_discovery"`, `"mqtt_bridge"`, etc.) via NATS
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

## Wire protocol

All services connect to `/api/v1/ws/service` over mTLS and exchange shared `ServiceMessage`/`ControllerMessage` enums. The
AsyncAPI definition lives at `crates/shared/wire/asyncapi.yaml` and is described in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).

## Update output streaming (SSE)

The controller provides a browser-facing SSE endpoint (`GET /api/v1/update-history/{id}/output/stream`) for real-time
update log tailing. This is a standard HTTP streaming endpoint — separate from the WebSocket wire protocol between
services and the controller.

An in-process `UpdateOutputBroadcaster` (`crates/ui/web-api/src/update_output_broadcaster.rs`) maintains per-update
`tokio::sync::broadcast` channels. When an agent sends `UpdateOutput` messages over WebSocket, the handler fans lines
out to both the database and any SSE subscribers. The SSE handler replays stored lines on connect and then streams new
lines in real time. The frontend renders output in an xterm.js terminal with full ANSI color support.

## OpenAPI Client

The `uptrakit-openapi-client` crate (`crates/shared/openapi-client/`) provides a typed HTTP
client for the Uptrakit web API. It wraps `reqwest` with compile-time type-safe endpoint
methods using the shared request/response types from `uptrakit-web-api-types`. The CLI crate
uses this client exclusively for all API communication.
See [OpenAPI Client](docs/development/openapi-client.md) for details.

## Optional OIDC support

OpenID Connect (OIDC) authentication is gated behind the `oidc` Cargo feature on `uptrakit-web-api`
(enabled by default, propagated via the controller's `oidc` feature). When disabled, the
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
- **Main Helpers**: `init_crypto()`, `print_build_info()`, `init_tracing()`, `default_resolve_shutdown()`, and
  `run_lifecycle_and_handle_errors()` reduce `main()` boilerplate. `init_tracing()` configures the tracing subscriber
  (each binary calls it explicitly — the SDK does not configure the global dispatcher autonomously).
  `default_resolve_shutdown()` maps `ShutdownCause` to `(DisconnectReason, LoopOutcome)` for standard binaries.
- **Signal Handling**: Cross-platform `SignalWatcher` for `SIGINT`, `SIGTERM`, and `SIGHUP`.
- **Enrollment**: WebSocket-based enrollment with certificate issuance.
- **Identity**: Service identity state management (service ID, enrollment secret, certificate, private key).
- **TLS/CA**: TLS connector builders (server-only and mTLS), CA bootstrap (cached, file, PKI endpoint, TOFU, system trust), CA staleness checks.
- **ControllerConnection**: Shared authenticated WebSocket connection with envelope serialization, sequence validation, Ping/Pong handling, and
  close-frame reason tracking. Used by all services regardless of capability set. Sequence validation is performed before
  full message deserialization to ensure unrecognized messages do not cause sequence mismatches.
- **CertificateRenewalHandler**: Handles certificate lifecycle messages (`CaBundleUpdated`, `RequestCertRenewal`, `Certificate`) automatically in the
  event loop. Also provides shared renewal timer helpers (`create_renewal_sleep`, `update_renewal_schedule`, `compute_renewal_delay`).
- **Backoff**: Exponential backoff with jitter for reconnection delays.
- **CLI**: Common CLI arguments (`--url`, `--config-dir`, `--state-dir`, `--force-enroll`, etc.).
