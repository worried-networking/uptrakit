# Update History

Update history is a permanent, append-only log of every software update that has been attempted
through Uptrakit. Each entry records what was updated, on which host, by whom, when it happened,
and whether it succeeded.

History entries are immutable once created. They are never edited or deleted, providing an
auditable record of all update activity in your environment.

## Viewing Update History

### Web UI

Navigate to **History** in the main navigation. The page shows all update history entries for
your account, newest first. Each row displays:

- The software item name
- The target host
- The target version
- The current status
- The timestamp
- The initiating user (or `scheduler` / `mqtt`)

Use the filter controls at the top of the page to narrow the results:

- **Host** — show only updates for a specific host
- **Software item** — show only updates for a specific software item
- **Status** — show only entries with a particular status (e.g. only failures)

Filters can be combined. Click any row to expand the entry and view output in an xterm.js terminal
with full ANSI color support. For in-progress or pending updates, the terminal streams output in
real time via SSE — a pulsing "Live" badge indicates an active stream. For completed or failed
updates, the stored output is rendered in the terminal.

> **Output size limit:** Uptrakit stores up to 50 MB of output per update. Updates that generate
> more output (such as large Docker image pulls) will show a truncation notice at the end of the
> terminal stream and an amber warning banner in the detail view. Only the first 50 MB is
> retained; the update itself continues to run normally regardless of the cap.

### CLI

```bash
# List all update history (paginated, newest first)
uptrakit history list

# Filter by host
uptrakit history list --host <HOST_ID>

# Filter by software item
uptrakit history list --software-item <ITEM_ID>

# Filter by status
uptrakit history list --status completed
uptrakit history list --status failed

# Combine filters
uptrakit history list --host <HOST_ID> --status failed --page 1 --per-page 5

# Show full details for a specific history entry (includes command output)
uptrakit history show <HISTORY_ID>

# Tail the live output of an in-progress update
uptrakit history tail <HISTORY_ID>
```

The `tail` command streams output in real time. ANSI escape codes pass through natively to
your terminal. Press Ctrl+C to detach — the update continues in the background.

## Update Statuses

Each history entry has one of the following statuses:

| Status | Meaning |
| --- | --- |
| `pending` | Update has been queued but not yet dispatched to the agent |
| `in_progress` | Update command is currently executing on the host |
| `completed` | Update finished successfully |
| `failed` | Update encountered an error; see the entry details for the captured output |

An update entry moves from `pending` to `in_progress` when the agent acknowledges and begins
execution, then to `completed` or `failed` when the agent reports the result.

If the agent is offline when an update is triggered, the entry remains `pending` until the agent
reconnects and processes the queued command.

## Triggering a Software Update

Updates in Uptrakit are **always manual and user-initiated**. The scheduler triggers version
checks only — it never installs or upgrades software.

### Web UI

1. Navigate to **History**.
2. Click the **Trigger Update** button (or use the context menu on a software item in the
   **Software** page).
3. Select the target host and the target version.
4. Confirm the update. A new history entry with status `pending` appears immediately.

When triggering from the **Software** detail page, a live terminal modal opens automatically
after the update is submitted, showing real-time output with ANSI color support. You can close
the modal at any time — the update continues in the background.

On the **History** page, expand any in-progress entry to see a live terminal with streaming
output.

### CLI

```bash
# Trigger an update for a software item on a specific host
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0"

# Trigger and follow the output in real time
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0" --follow

# Include optional release metadata for traceability
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0" \
  --release-tag "v2.0.0" \
  --release-url "https://github.com/example/repo/releases/tag/v2.0.0"
```

With `--follow` / `-f`, the CLI connects to the SSE stream after triggering and prints output
lines to stdout in real time. Status changes print to stderr. Press Ctrl+C to detach.

## Initiated By

Each history entry records who initiated the update:

| Initiator | Meaning |
| --- | --- |
| User UUID | A specific user triggered the update from the Web UI or CLI |
| `scheduler` | Reserved; the scheduler does not trigger updates |
| `mqtt` | Update was triggered via the Home Assistant / MQTT integration |

## Related Documentation

- [Update Workflow](update-workflow.md) — how updates work end-to-end.
- [Update History Entity](../architecture/update-history-entity.md) — underlying data model.
- [CLI Usage Guide](cli-usage.md) — `history` and `update` commands.
- [Home Assistant and MQTT](home-assistant-mqtt.md) — triggering updates via Home Assistant.
