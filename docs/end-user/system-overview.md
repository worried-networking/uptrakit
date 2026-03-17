# System Overview

Uptrakit tracks installed software across hosts without performing automatic updates.

- **Controller:** central API, scheduler (checks only), Web UI (SvelteKit), and plugin orchestrator.
- **Agents:** outbound-only daemons that detect installed versions and run user-approved updates using sudo allowlists.
- **MQTT Service:** standalone binary for Home Assistant integration via MQTT auto-discovery.
- **Plugins:** modular components responsible for version detection, upstream resolution, and update execution.

Communication opens secure WebSocket connections to `/api/v1/ws/service` with mutual TLS. Agents and MQTT services enroll via tokens, CSRs, and a
managed CA.

Operational note: all Uptrakit binaries (`uptrakit-controller`, `uptrakit-agent`, `uptrakit-mqtt`, `uptrakit`)
support `--version` for deterministic build capability reporting (crate version, enabled features, target/cfg/profile).

## Embedded Services

In single-binary deployments (e.g. homelab), the controller can run the scheduler and an agent
inside its own process. These **embedded services** appear in the services list alongside
externally connected services.

Key points:

- Embedded services are marked with `is_embedded: true` and show an "(embedded)" indicator
  in the CLI and Web UI.
- They are auto-provisioned and auto-approved — no manual enrollment is needed.
- They **cannot be removed or deactivated**. The controller manages their lifecycle.
- When an external service with the same role connects, the embedded service **yields** and
  defers its responsibilities to the external instance. The "(yielded)" indicator replaces
  "(embedded)" in the listing.
- Yielding is automatic and reversible — when the external service disconnects, the embedded
  service resumes.
