---
title: Update Workflow
weight: 60
description: The scheduler runs periodic version checks but never installs updates automatically; users trigger updates manually via the Web UI, CLI, or MQTT integration.
---

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
- **Interactive updates**: when a package manager may prompt for input (e.g., APT config file
  conflicts), updates can be triggered in interactive mode with `--interactive`. This allocates
  a PTY on the target host and keeps stdin open for bidirectional terminal I/O. See
  [Interactive Updates](interactive-updates.md) for details.
- **In-progress badge**: on the software detail page, the per-host status badge changes from
  yellow "Update Available" to blue "In Progress" as soon as an update is queued, pending, or
  running for that host. Clicking the blue badge opens the live terminal output directly instead
  of the update confirmation dialog. The badge reverts to its previous state once the update
  completes or fails.

## Update Categories

Each available update can be classified by category (e.g. security, bugfix, feature) based on the
upstream source. Currently the APT plugin classifies updates from `*-security` repositories as
`security`. Other plugins leave the category as `unknown`. The category is stored on both
`host_software_items` and `update_history` records and exposed in API responses.

Categories enable filtering during batch updates — for example, applying only security patches
across a host without touching feature updates.

## Batch Updates

Batch updates allow triggering multiple updates in a single request. Two modes are supported:

### Host-wide batch update

Update all outdated software on a single host. Optionally filter by update category (e.g.
`security`) or exclude specific software items.

```sh
# Update everything outdated on a host
uptrakit update batch-host <HOST_ID>

# Only apply security patches
uptrakit update batch-host <HOST_ID> --category security

# Exclude specific items
uptrakit update batch-host <HOST_ID> --exclude <ITEM_ID_1>,<ITEM_ID_2>

# Follow progress in real time
uptrakit update batch-host <HOST_ID> --follow
```

### Item-wide batch update

Roll out a specific software item version to all (or selected) hosts.

```sh
# Roll out to all assigned hosts
uptrakit update batch-item <ITEM_ID> --to-version 3.0.0

# Limit to specific hosts
uptrakit update batch-item <ITEM_ID> --to-version 3.0.0 --host <HOST_ID_1>,<HOST_ID_2>

# Follow progress
uptrakit update batch-item <ITEM_ID> --to-version 3.0.0 --follow
```

### Batch execution model

- Updates within a batch are dispatched **sequentially per host** — only one update runs at
  a time on each host. After one completes (or fails), the next is dispatched.
- Items with an already-active update are **soft-skipped** (included in the response's
  `skipped` list) rather than failing the entire batch.
- No agent-side changes are needed — the agent receives individual `ExecuteUpdate` messages
  as usual.
- Batch progress can be monitored via the SSE stream endpoint or the CLI `--follow` flag.

### Monitoring batches

```sh
# List all batches
uptrakit update-batches list

# Filter by status
uptrakit update-batches list --status in_progress

# Show batch details with per-item status
uptrakit update-batches show <BATCH_ID>

# Follow batch progress via SSE
uptrakit update-batches follow <BATCH_ID>
```

See also: [Batch Update API](https://github.com/worried-networking/uptrakit/tree/main/docs/api/),
[Update History Entity](https://github.com/worried-networking/uptrakit/tree/main/docs/architecture/),
[CLI Usage](cli-usage.md#batch-updates).
