# System Overview

Uptrakit tracks installed software across hosts without performing automatic updates.

- **Controller:** central API, scheduler (checks only), Web UI (SvelteKit), and provider orchestrator.
- **Agents:** outbound-only daemons that detect installed versions and run user-approved updates using sudo allowlists.
- **MQTT Service:** standalone binary for Home Assistant integration via MQTT auto-discovery.
- **Providers:** modular components responsible for version detection, upstream resolution, and update execution.

Communication opens secure WebSocket connections to `/api/v1/ws/service` with mutual TLS. Agents and MQTT services enroll via tokens, CSRs, and a
managed CA.

Operational note: all Uptrakit binaries (`uptrakit-controller`, `uptrakit-agent`, `uptrakit-mqtt`, `uptrakit-cli`)
support `--version` for deterministic build capability reporting (crate version, enabled features, target/cfg/profile).
