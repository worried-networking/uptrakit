# Interactive Updates Development Guide

This document covers the implementation details of the interactive updates feature,
including the feature gate strategy, PTY allocation, stdin forwarding, and testing.

For the API reference, see [Interactive Updates API](../api/interactive-updates.md).
For security considerations, see [Interactive Updates Security](../security/interactive-updates.md).

## Feature Gate Strategy

The entire interactive updates subsystem is gated behind the `interactive` Cargo feature.
This feature is **enabled by default** in all binary crates (`uptrakit-agent`,
`uptrakit-agent-ssh`, `uptrakit-controller`) so standard builds include interactive
update support out of the box. Library crates keep it opt-in.

### Feature Propagation Chain

| Crate | Feature | Enables |
| --- | --- | --- |
| `uptrakit-command` | `interactive` | PTY support via `rustix`, `InteractiveHandle`, `execute_interactive()` |
| `uptrakit-agent-core` | `interactive` | Interactive update execution path, stdin forwarding |
| `uptrakit-agent` | `interactive` (default) | `UpdateStdinData` handler, attention polling, `InteractiveUpdates` capability |
| `uptrakit-agent-ssh` | `interactive` (default) | SSH PTY request, `UpdateStdinData` handler, `InteractiveUpdates` capability |
| `uptrakit-web-api` | `interactive` | Interactive WS endpoint, `InteractiveSessionRegistry` |
| `uptrakit-controller` | `interactive` (default) | Propagates to `web-api/interactive` |

### Wire Types Are Unconditional

The wire protocol types (`UpdateStdinDataPayload`, `StdinAttentionPayload`,
`ExecuteUpdatePayload.interactive`) are always compiled regardless of the `interactive`
feature. This ensures a non-interactive controller can still deserialize messages from
an interactive agent (and vice versa). Only the *behavior* is feature-gated.

### Additive-Only Pattern

All `#[cfg(feature = "interactive")]` blocks are additive -- they add new code paths
without removing existing ones. The one exception is `AUTHORIZED_KEYS_RESTRICTIONS` in
the SSH agent bootstrap, where `no-pty` is omitted when `interactive` is enabled.
This uses two independent constant definitions (one per feature state) rather than
subtraction from a base definition.

## Architecture

```text
Frontend (xterm.js)  <--WebSocket-->  Controller  <--WebSocket-->  Agent  <--PTY-->  Process
       | input                        | relay                      | stdin
       | output                       | relay                      | stdout/stderr
```

### Local Agent PTY Flow

1. `openpty()` allocates a PTY pair (master/slave) via `rustix`.
2. Child process spawns with the slave fd as stdin/stdout/stderr.
3. Child gets its own session via `setsid()` in `pre_exec`.
4. Reader task on master fd sends output lines to `output_tx`.
5. Writer task reads from `stdin_tx` and writes to master fd.
6. Signal sender uses `kill(pid, signal)` on the child process group.

### SSH Agent PTY Flow

1. `channel_open_session()` opens an SSH session channel.
2. `channel.request_pty("xterm-256color", 80, 24, ...)` allocates a PTY on the remote.
3. `channel.exec(command)` executes the update command.
4. Stdin remains open (no `channel.eof()` call).
5. Writer task reads from `stdin_tx` and writes via `channel.data()`.
6. Signals are translated to terminal control characters (e.g., SIGINT becomes `\x03`).

**Important**: The SSH agent bootstrap writes `authorized_keys` entries. When the
`interactive` feature is enabled, the `no-pty` restriction is omitted so `sshd` allows
PTY allocation. Running `host sync` with an interactive-enabled build updates existing
hosts.

### Stdin Attention Detection

The attention detector is a heuristic timer:

- Resets on each output line received from the process.
- After 10 seconds of silence (while the process is still alive), fires a notification
  on the `attention_rx` channel.
- The agent sends a `StdinAttention` message to the controller.
- The controller broadcasts a `stdin_attention` event via SSE and the interactive
  WebSocket, and dispatches a notification (if rules are configured).

## Key Types

### `InteractiveHandle` (in `uptrakit-command`)

```rust
pub struct InteractiveHandle {
    pub stdin_tx: mpsc::Sender<Vec<u8>>,
    pub signal_tx: mpsc::Sender<i32>,
    pub completion: JoinHandle<Result<CommandOutput>>,
    pub attention_rx: mpsc::Receiver<()>,
}
```

### `InFlightUpdate` Extensions (in `uptrakit-agent-core`)

When `interactive` is enabled, `InFlightUpdate` gains three optional fields:

- `stdin_tx: Option<mpsc::Sender<Vec<u8>>>` -- stdin writer.
- `signal_tx: Option<mpsc::Sender<i32>>` -- signal sender.
- `attention_rx: Option<mpsc::Receiver<()>>` -- attention detector.

### `InteractiveSessionRegistry` (in `uptrakit-web-api`)

Tracks active interactive WebSocket sessions with single-writer enforcement.
Uses `parking_lot::Mutex` per project convention.

```rust
pub struct InteractiveSessionRegistry {
    sessions: parking_lot::Mutex<HashMap<Uuid, InteractiveSession>>,
}
```

## Testing

### Unit Tests

- Wire protocol serde round-trips for `UpdateStdinDataPayload` and `StdinAttentionPayload`.
- `WireValidate` tests for data length limits (`MAX_STDIN_DATA_LEN = 64 KB`).
- `InteractiveSessionRegistry` claim/release and single-writer enforcement.
- Notification `StdinAttention` message builder output.
- CLI `--interactive` flag parsing.

### Integration Tests

Interactive WebSocket endpoint tests are gated on `#[cfg(all(test, feature = "db-sqlite"))]`
in the web-api crate:

- Authentication and permission checks.
- Single-writer rejection (409 Conflict).
- Agent connectivity validation.

### Manual Testing

To test the full interactive flow end-to-end:

1. Build the controller and agent with default features (interactive is included).
2. Start the controller and an agent.
3. Trigger an interactive update via the CLI or API.
4. Connect via the WebSocket endpoint and verify stdin forwarding.

## Capability Advertisement

Agents compiled with the `interactive` feature (enabled by default in all binary crates)
include `Capability::InteractiveUpdates` in their capability set at enrollment.
The controller checks this before:

1. Sending `UpdateStdinData` to an agent.
2. Setting `interactive: true` on `ExecuteUpdatePayload`.
3. Allowing interactive WebSocket connections for updates owned by that agent.

## See Also

- [Command Executor](command-executor.md) -- base executor trait
- [Service Lifecycle](service-lifecycle.md) -- agent event loop
- [Coding Standards](coding-standards.md) -- feature gate conventions
- [Wire Protocol](../api/wire-protocol.md) -- message reference
