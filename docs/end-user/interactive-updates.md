# Interactive Updates

Interactive updates enable bidirectional terminal I/O between an admin and a running
update process. When a package manager prompts for input (e.g., an APT configuration
file conflict), the admin can respond directly through the Web UI or CLI instead of the
process receiving EOF and failing.

## When to Use Interactive Mode

- Package managers that prompt for configuration decisions (e.g., `apt` config file
  conflicts, `dpkg` prompts).
- Updates that require manual confirmation steps.
- Situations where you need the ability to send Ctrl+C to abort a stuck process.

For routine, unattended updates, standard (non-interactive) mode remains the default
and recommended approach.

## Prerequisites

- The agent (or SSH agent) must advertise the `InteractiveUpdates` capability. Standard
  builds include this by default. You can verify the capability in the service detail view.
- The admin must have the `ManageSoftware` permission (stdin forwarding is equivalent
  to code execution on the target host).

## Triggering an Interactive Update

### Web UI

When triggering an update from the Software Items page, enable the **Interactive** toggle
in the update dialog. After the update starts, click **Connect** on the update history
row to open an interactive terminal session.

### CLI

```sh
uptrakit update trigger <ITEM_ID> <HOST_ID> \
    --to-version <VERSION> \
    --interactive \
    --follow
```

The `--interactive` (`-i`) flag requests PTY allocation on the target host. Combined
with `--follow` (`-f`), the CLI streams output in real time and forwards your
keystrokes to the remote process.

## How It Works

1. The controller sends `execute_update` with `interactive: true` to the agent.
2. The agent allocates a PTY (pseudo-terminal) instead of piping stdin to `/dev/null`.
3. The process runs inside the PTY, receiving a real terminal environment.
4. Output flows back through the existing output channel (agent to controller to UI).
5. Stdin data (keystrokes) flows from the UI/CLI through a WebSocket to the controller,
   which relays it to the agent over the service WebSocket.

## Stdin Attention Detection

If the update process appears to be waiting for input (no output for 10 seconds while
the process is still running), the system sends a **StdinAttention** notification. This
notification:

- Appears in the Web UI as a pulsing "Waiting for input" indicator.
- Triggers any configured notification rules (e.g., Telegram alert).
- Helps admins notice when a process needs attention, even if they are not actively
  watching the terminal.

## Single-Writer Enforcement

Only one admin can have an active interactive session per update at a time. Other admins
can observe the update output via the standard SSE stream (read-only). If you attempt
to connect to an update that already has an active interactive session, the connection
is rejected with a 409 Conflict response.

## Edge Cases

| Scenario | Behavior |
| --- | --- |
| Update finishes while typing | Terminal goes read-only; WebSocket receives `completed` |
| Agent disconnects mid-session | Controller broadcasts an error; frontend shows disconnect |
| Admin disconnects mid-update | Update continues running; stdin channel closes (process sees EOF) |
| PTY allocation fails | Falls back to non-interactive mode with a warning |
| Agent lacks `InteractiveUpdates` | Controller returns HTTP 409 |

## Batch Updates

Batch updates can be triggered with `interactive: true` on the batch request. The
interactive stdin channel applies to whichever package is currently being updated within
the batch. When one package finishes and the next starts, the PTY resets.

## See Also

- [Update Workflow](update-workflow.md) -- standard update lifecycle
- [Interactive Updates API](../api/interactive-updates.md) -- WebSocket endpoint reference
- [Interactive Updates Security](../security/interactive-updates.md) -- security model
