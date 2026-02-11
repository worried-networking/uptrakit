# Architecture Overview

Uptrakit is an agent-based toolkit: the **controller** orchestrates scheduling, hosts a Web UI/API, and checks upstream versions; **agents** run outbound-only, unprivileged daemons that report installed versions and execute user-approved updates; the **MQTT service** integrates with Home Assistant via MQTT.

## Key references
- System architecture and operations: `docs/end-user/system-overview.md`
- API and wire protocol: `docs/api/wire-protocol.md` and `docs/api/http-web-api.md`
- Settings, authentication, and service flows: `docs/api/settings-runtime.md`, `docs/api/auth-flows.md`, `docs/api/services-operations.md`
- Reverse proxy deployment: `docs/end-user/deployment/reverse-proxy.md`
- Security architecture: `docs/security/security-architecture.md`
- Provider development expectations: `docs/development/provider-guidelines.md`

## Project layout
- Rust workspace (`resolver = "3"`) under `crates/*/*` for controller, agent, MQTT service, providers, shared libraries, and CLI/web API.
- Frontend is a SvelteKit SPA in `frontend/` built with Tailwind CSS and Skeleton UI.

## Wire protocol
Agents and MQTT services connect to `/api/v1/ws/service` over mTLS and exchange shared `ServiceMessage`/`ControllerMessage` enums. The AsyncAPI definition lives at `crates/shared/wire/asyncapi.yaml` and is described in `docs/api/wire-protocol.md`.
