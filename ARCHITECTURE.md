# Architecture Overview

Uptrakit is an agent-based toolkit: the **controller** orchestrates scheduling, hosts a Web UI/API, and checks upstream versions; **agents** run
outbound-only, unprivileged daemons that report installed versions and execute user-approved updates; the **MQTT service** integrates with Home
Assistant via MQTT; the **SSH agent** manages remote hosts over SSH (see [SSH Agent Architecture](docs/architecture/ssh-agent.md)).

A centralised [DB-backed scheduler](docs/architecture/scheduler.md) coordinates periodic tasks (version checks, cleanup, CA rotation checks,
certificate renewal) across controller instances using optimistic locking for HA-safe exactly-once execution.

## Key references

- Detailed entity documentation: [docs/architecture/](docs/architecture/) (multi-tenancy, hosts, software items, update history, scheduler)

- System architecture and operations: [docs/end-user/system-overview.md](docs/end-user/system-overview.md)

- API and wire protocol: [docs/api/wire-protocol.md](docs/api/wire-protocol.md) and [docs/api/http-web-api.md](docs/api/http-web-api.md)

- Settings, authentication, and service flows: [docs/api/settings-runtime.md](docs/api/settings-runtime.md),
  [docs/api/auth-flows.md](docs/api/auth-flows.md), [docs/api/services-operations.md](docs/api/services-operations.md)

- Reverse proxy deployment: [docs/end-user/deployment/reverse-proxy.md](docs/end-user/deployment/reverse-proxy.md)

- Security architecture: [docs/security/security-architecture.md](docs/security/security-architecture.md)

- Provider development expectations: [docs/development/provider-guidelines.md](docs/development/provider-guidelines.md)

- Command executor abstraction: [docs/development/command-executor.md](docs/development/command-executor.md)

## Project layout

- Rust workspace (`resolver = "3"`) under `crates/*/*` for controller, agent, SSH agent, MQTT service, providers, shared libraries, and CLI/web API.
- Frontend is a SvelteKit SPA in `frontend/` built with Tailwind CSS and Skeleton UI.

## Wire protocol

Agents, SSH agents, and MQTT services connect to `/api/v1/ws/service` over mTLS and exchange shared `ServiceMessage`/`ControllerMessage` enums. The
AsyncAPI definition lives at `crates/shared/wire/asyncapi.yaml` and is described in [docs/api/wire-protocol.md](docs/api/wire-protocol.md).

## OpenAPI Client

The `uptrakit-openapi-client` crate (`crates/shared/openapi-client/`) provides a typed HTTP
client for the Uptrakit web API. It wraps `reqwest` with compile-time type-safe endpoint
methods using the shared request/response types from `uptrakit-web-api-types`. The CLI crate
uses this client exclusively for all API communication.
See [OpenAPI Client](docs/development/openapi-client.md) for details.

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
  services implement three methods (`config()`, `enrollment_info()`, `run_authenticated_loop()`) and get directory setup, identity management, CA
  bootstrap, enrollment with backoff, and reconnection with backoff for free. See [Service Lifecycle](docs/development/service-lifecycle.md).
- **Enrollment**: WebSocket-based enrollment with certificate issuance.
- **Identity**: Service identity state management (service ID, enrollment secret, certificate, private key).
- **TLS/CA**: TLS connector builders (server-only and mTLS), CA bootstrap (cached, file, PKI endpoint, TOFU, system trust).
- **ControllerConnection**: Shared authenticated WebSocket connection with envelope serialization, sequence validation, Ping/Pong handling, and
  close-frame reason tracking. Used by both agent and MQTT service.
- **Backoff**: Exponential backoff with jitter for reconnection delays.
- **CLI**: Common CLI arguments (`--url`, `--config-dir`, `--state-dir`, `--force-enroll`, etc.).
