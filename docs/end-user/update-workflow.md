# Update Workflow

- Scheduler runs periodic version checks but does not install updates.
- Version checks can also be manually triggered per software item from the Web UI (via the context menu on the
  Software page) or via the CLI (`uptrakit check item <id>`).
- Users trigger updates manually from the Web UI, CLI, or MQTT/Home Assistant integration.
- Each update record stores the initiating user (`user UUID`, `scheduler`, or `mqtt`) and command output.
- Update history is available via `/api/v1/update-history`.
- Hooks (systemd, Docker Compose, custom) log phase markers in their stdout for easier debugging.
- **Real-time log tailing**: during an active update, output can be streamed live via an SSE endpoint
  (`GET /api/v1/update-history/{id}/output/stream`). The Web UI renders output in an xterm.js terminal
  with full ANSI color support. The CLI supports `--follow` on `update trigger` and `history tail` for
  terminal-native streaming. See [Update History](update-history.md) for details.
