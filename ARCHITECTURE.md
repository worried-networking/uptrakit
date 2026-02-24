# Architecture Overview

Uptrakit is an agent-based toolkit: the **controller** orchestrates scheduling, hosts a Web UI/API, and checks upstream versions; **agents** run
outbound-only, unprivileged daemons that report installed versions and execute user-approved updates; the **MQTT service** integrates with Home
Assistant via MQTT; the **SSH agent** manages remote hosts over SSH (see [SSH Agent Architecture](docs/architecture/ssh-agent.md)).

A centralised [DB-backed scheduler](docs/architecture/scheduler.md) coordinates periodic tasks (version checks, cleanup, CA rotation checks,
certificate renewal) across controller instances using optimistic locking for HA-safe exactly-once execution.

## Key references

- Detailed entity documentation: [docs/architecture/](docs/architecture/) (multi-tenancy, hosts, software items, update history, scheduler, autodiscovery)

- System architecture and operations: [docs/end-user/system-overview.md](docs/end-user/system-overview.md)

- API and wire protocol: [docs/api/wire-protocol.md](docs/api/wire-protocol.md) and [docs/api/http-web-api.md](docs/api/http-web-api.md)

- Autodiscovery: [docs/api/autodiscovery.md](docs/api/autodiscovery.md) and [docs/end-user/autodiscovery.md](docs/end-user/autodiscovery.md)

- Settings, authentication, and service flows: [docs/api/settings-runtime.md](docs/api/settings-runtime.md),
  [docs/api/auth-flows.md](docs/api/auth-flows.md), [docs/api/services-operations.md](docs/api/services-operations.md)

- Reverse proxy deployment: [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)

- Security architecture: [docs/security/security-architecture.md](docs/security/security-architecture.md)

- Provider development expectations: [docs/development/provider-guidelines.md](docs/development/provider-guidelines.md)

- Command executor abstraction: [docs/development/command-executor.md](docs/development/command-executor.md)

## Project layout

- Rust workspace (`resolver = "3"`) under `crates/*/*` for controller, agent, SSH agent, MQTT service, providers, shared libraries, and CLI/web API.
- Frontend is a SvelteKit SPA in `frontend/` built with Tailwind CSS and Skeleton UI.
  It can be served from the filesystem (`--static-dir` / auto-detection) or embedded into
  the controller binary at compile time via the `embed-frontend` Cargo feature for
  single-binary deployment.

## Providers

Providers are pluggable modules that define how to detect installed versions, resolve latest upstream versions, and execute updates. Each provider
crate implements the `Provider` trait and is registered in `uptrakit-provider-registry`. See
[Provider Development Guidelines](docs/development/provider-guidelines.md) for the extension pattern.

| Provider type | Crate | Version resolution | Autodiscovery | Notes |
| --- | --- | --- | --- | --- |
| `github_releases` | `uptrakit-provider-github` | Controller (GitHub API) | No | Tracks GitHub release tags |
| `docker_registry` | `uptrakit-provider-docker-registry` | Controller (Registry API) | No | Tracks OCI image tags |
| `homebrew` | `uptrakit-provider-homebrew` | Agent (`brew info`) | Yes | macOS/Linux formulae and casks |
| `proxmox_helper_scripts` | `uptrakit-provider-proxmox-helper-scripts` | Agent (local scripts) | Yes | PVE helper-script containers |
| `apt` | `uptrakit-provider-apt` | Agent (`apt-cache madison`) | Yes | Debian/Ubuntu packages via APT |

Providers with a local package index (`homebrew`, `proxmox_helper_scripts`, `apt`) resolve both installed and latest versions on the agent.
All other providers resolve upstream versions on the controller.

## Wire protocol

Agents, SSH agents, and MQTT services connect to `/api/v1/ws/service` over mTLS and exchange shared `ServiceMessage`/`ControllerMessage` enums. The
AsyncAPI definition lives at `crates/shared/wire/asyncapi.yaml` and is described in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).

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
  services declare three associated constants (`DIR_NAME`, `SERVICE_LABEL`, `SERVICE_TYPE`) and implement callbacks (`on_connected`, `on_message`,
  `on_shutdown`, etc.) to get directory setup, identity management, CA bootstrap, enrollment with backoff, and reconnection with backoff for free.
  See [Service Lifecycle](docs/development/service-lifecycle.md).
- **Event Loop**: The SDK owns the unified `tokio::select!` loop that handles ping/pong, certificate renewal, CA staleness checks, signal handling,
  and close-reason dispatch. Services inject custom behaviour through `ServiceHandler` callbacks (`poll_service_event`, `on_service_event`).
- **Main Helpers**: `init_crypto()`, `print_build_info()`, and `run_lifecycle_and_handle_errors()` reduce `main()`
  boilerplate. Each binary initializes its own tracing subscriber — the SDK does not configure the global dispatcher.
- **Signal Handling**: Cross-platform `SignalWatcher` for `SIGINT`, `SIGTERM`, and `SIGHUP`.
- **Enrollment**: WebSocket-based enrollment with certificate issuance.
- **Identity**: Service identity state management (service ID, enrollment secret, certificate, private key).
- **TLS/CA**: TLS connector builders (server-only and mTLS), CA bootstrap (cached, file, PKI endpoint, TOFU, system trust), CA staleness checks.
- **ControllerConnection**: Shared authenticated WebSocket connection with envelope serialization, sequence validation, Ping/Pong handling, and
  close-frame reason tracking. Used by all service types. Sequence validation is performed before full message deserialization to ensure
  unrecognized messages do not cause sequence mismatches.
- **CertificateRenewalHandler**: Handles certificate lifecycle messages (`CaBundleUpdated`, `RequestCertRenewal`, `Certificate`) automatically in the
  event loop. Also provides shared renewal timer helpers (`create_renewal_sleep`, `update_renewal_schedule`, `compute_renewal_delay`).
- **Backoff**: Exponential backoff with jitter for reconnection delays.
- **CLI**: Common CLI arguments (`--url`, `--config-dir`, `--state-dir`, `--force-enroll`, etc.).
