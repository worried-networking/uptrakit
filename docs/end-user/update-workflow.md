# Update Workflow

- Scheduler runs periodic version checks but does not install updates.
- Version checks can also be manually triggered per software item from the Web UI (via the context menu on the
  Software page) or via the CLI (`uptrakit check item <id>`).
- Users trigger updates manually from the Web UI, CLI, or MQTT/Home Assistant integration.
- Each update record stores the initiating user (`user UUID`, `scheduler`, or `mqtt`) and command output.
- Update history is available via `/api/v1/update-history`.
- Hooks (systemd, Docker Compose, custom) log phase markers in their stdout for easier debugging.
