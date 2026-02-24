# Zero-downtime graceful restart

The controller supports HAProxy-style zero-downtime restarts using `SO_REUSEPORT`. This allows a new controller process
to start accepting connections while the old process drains existing ones.

**CLI flags:**

| Flag | Default | Description |
| --- | --- | --- |
| `--reuseport` | `false` | Enable `SO_REUSEPORT` socket option (required on both processes) |
| `--takeover-from <PID>` | — | PID of old process to take over from; sends SIGUSR1 to initiate graceful shutdown |
| `--shutdown-timeout-secs` | `30` | Graceful shutdown timeout (how long to drain connections) |

**Restart sequence:**

1. Old process is running with `--reuseport`
1. New process starts with `--reuseport --takeover-from <OLD_PID>`
1. New process binds to the same port (SO_REUSEPORT allows this)
1. New process starts accepting connections immediately
1. New process sends SIGUSR1 to old process
1. Old process stops accepting new connections
1. Old process scatters `ServerRestarting` notifications to agents over 5 seconds (avoids thundering herd)
1. Old process cancels background tasks and waits for drain timeout
1. Old process exits cleanly
1. New process serves all traffic

**Signal handling:**

| Signal | Action |
| --- | --- |
| SIGTERM | Initiate graceful shutdown |
| SIGINT | Initiate graceful shutdown |
| SIGUSR1 | Initiate graceful shutdown (used for takeover) |

**Wire protocol:** The `ServerRestarting` message (`ControllerMessage::ServerRestarting(ServerRestartingPayload)`)
notifies services that the controller is restarting. On receipt, services initiate their own graceful shutdown: they
drain any in-flight work (agents wait for a running update to complete), send a `Disconnecting` message with
`reason: restart`, and exit the event loop with `LoopOutcome::Disconnected`. The service lifecycle then reconnects
with backoff once the controller is available again.

**Service shutdown cause mapping:**

| Trigger | `DisconnectReason` sent | `LoopOutcome` returned |
| --- | --- | --- |
| `SIGHUP` | `restart` | `Restart` (exits lifecycle for external restart) |
| `SIGINT` / `SIGTERM` | `shutdown` | `Shutdown` (exits lifecycle cleanly) |
| `ServerRestarting` | `restart` | `Disconnected` (reconnects with backoff) |

The mapping is implemented via the `ShutdownCause` enum in `uptrakit-service-sdk` and the `resolve_shutdown` helper
in each service. See [Service Lifecycle](service-lifecycle.md) for the full reconnect flow.

**Platform support:** `SO_REUSEPORT` is available on Linux, macOS, FreeBSD, and OpenBSD. Not available on Windows.

**Key files:**

| File | Purpose |
| --- | --- |
| `crates/core/controller/src/tasks.rs` | `BackgroundTasks` struct with coordinated shutdown sequence |
| `crates/core/controller/src/durations.rs` | `BACKGROUND_TASK_SHUTDOWN_TIMEOUT` (5s), `RESTART_NOTIFICATION_SCATTER` (5s) |
| `crates/core/controller/src/main.rs` | Signal handler setup (SIGTERM, SIGINT, SIGUSR1) and server event loop |
| `crates/shared/service-sdk/src/lifecycle.rs` | `ShutdownCause` enum and `ServiceHandler::on_shutdown` trait method |
| `crates/shared/service-sdk/src/event_loop.rs` | `ServerRestarting` handler — calls `on_shutdown` with `ShutdownCause::ServerRestarting` |
| `crates/core/agent/src/main.rs` | `resolve_shutdown` helper and `AgentHandler::on_shutdown` implementation |
| `crates/core/agent-ssh/src/main.rs` | `resolve_shutdown` helper and `SshAgentHandler::on_shutdown` implementation |
| `crates/core/mqtt/src/main.rs` | `resolve_shutdown` helper and `MqttHandler::on_shutdown` implementation |
