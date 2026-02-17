# CLI Usage Guide

The `uptrakit-cli` binary provides a command-line interface for interacting with the Uptrakit controller. It
supports authentication, resource inspection, version checks, update triggering, and scheduler management.

## Global Options

Every command accepts these global flags:

| Flag | Description |
| --- | --- |
| `--server <URL>` | Controller URL (overrides stored config) |
| `--token <TOKEN>` | API token (overrides stored credentials) |
| `--insecure` | Skip TLS certificate verification (development only) |
| `-o`, `--output <FORMAT>` | Output format: `human` (default), `json`, `yaml` |
| `--version` | Show version and build metadata |

## Authentication

Before using most commands you must authenticate. The CLI uses a device authorization flow (RFC 8628).

```sh
# Log in via browser
uptrakit-cli auth login

# Check current auth status
uptrakit-cli auth status

# Manage API tokens
uptrakit-cli auth token create --name "ci-token"
uptrakit-cli auth token list
uptrakit-cli auth token revoke <TOKEN_ID>
```

See also: [Auth Flows](../api/auth-flows.md), [Auth and Authorization](../security/auth-and-authorization.md).

## Services

List, inspect, and manage services (agents, MQTT, SSH agents) registered
with the controller.

```sh
# List all services (paginated)
uptrakit-cli services list
uptrakit-cli services list --page 2 --per-page 10

# Filter by type or status
uptrakit-cli services list --type agent
uptrakit-cli services list --status pending
uptrakit-cli services list --type mqtt --status approved

# Show service details
uptrakit-cli services show <SERVICE_ID>

# Approve a pending service
uptrakit-cli services approve <SERVICE_ID>

# Reject a pending service
uptrakit-cli services reject <SERVICE_ID>

# Remove (deactivate) a service
uptrakit-cli services remove <SERVICE_ID>

# Merge a pending source service into an approved target service
uptrakit-cli services merge <TARGET_ID> <SOURCE_ID>
```

See also: [Service Operations](../api/services-operations.md),
[HTTP Web API](../api/http-web-api.md).

## Hosts

List and inspect hosts registered with the controller.

```sh
# List all hosts (paginated)
uptrakit-cli hosts list
uptrakit-cli hosts list --page 2 --per-page 10

# Show host details
uptrakit-cli hosts show <HOST_ID>
```

See also: [Host Entity](../architecture/host-entity.md).

## Software Items

List and inspect software items configured on the controller.

```sh
# List all software items (paginated)
uptrakit-cli software-items list
uptrakit-cli software-items list --page 1 --per-page 50

# Show software item details (includes host assignments and version info)
uptrakit-cli software-items show <ITEM_ID>
```

See also: [Software Item Entity](../architecture/software-item-entity.md).

## Version Checks

Trigger version checks to discover installed and available versions.

```sh
# Trigger bulk version check (all items, all hosts via scheduler)
uptrakit-cli check all

# Trigger version check for a specific software item (all assigned hosts)
uptrakit-cli check installed <ITEM_ID>
uptrakit-cli check available <ITEM_ID>

# Scope to a specific host
uptrakit-cli check installed <ITEM_ID> --host <HOST_ID>
uptrakit-cli check available <ITEM_ID> --host <HOST_ID>
```

`check installed` and `check available` use the same API endpoint (the agent checks both installed and available versions). They are separated for clarity.

`check all` finds the `version_check` scheduler task and triggers it.

See also: [Wire Protocol](../api/wire-protocol.md), [HTTP Web API](../api/http-web-api.md).

## Updates

Trigger a software update on a specific host. Updates are always manual and user-initiated.

```sh
# Trigger an update
uptrakit-cli update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0"

# With optional release metadata
uptrakit-cli update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0" \
  --release-tag "v2.0.0" \
  --release-url "https://github.com/example/repo/releases/tag/v2.0.0"
```

See also: [Update Workflow](update-workflow.md), [Update History Entity](../architecture/update-history-entity.md).

## Update History

View the history of updates across hosts and software items.

```sh
# List all update history (paginated)
uptrakit-cli history list

# Filter by host, software item, or status
uptrakit-cli history list --host <HOST_ID>
uptrakit-cli history list --software-item <ITEM_ID>
uptrakit-cli history list --status completed
uptrakit-cli history list --host <HOST_ID> --status failed --page 1 --per-page 5

# Show details for a specific history entry
uptrakit-cli history show <HISTORY_ID>
```

See also: [Update History Entity](../architecture/update-history-entity.md).

## Scheduler

Inspect and trigger scheduled tasks (version checks, cleanup, CA rotation).

```sh
# List all scheduled tasks
uptrakit-cli scheduler list

# Show task details
uptrakit-cli scheduler show <TASK_ID>

# Trigger immediate execution
uptrakit-cli scheduler trigger <TASK_ID>
```

See also: [Scheduler](../architecture/scheduler.md), [HTTP Web API](../api/http-web-api.md).

## Raw API Access

For advanced use, the `api` command lets you call any REST endpoint directly.

```sh
uptrakit-cli api GET /api/v1/auth/me
uptrakit-cli api POST /api/v1/software-items --data '{"name":"test"}'
uptrakit-cli api GET /api/v1/hosts -o json
```

## Output Formats

All commands support three output formats via `--output` / `-o`:

- **`human`** (default): columnar or line-based text suitable for terminals.
- **`json`**: compact single-line JSON, suitable for `jq` piping.
- **`yaml`**: YAML output for configuration workflows.

```sh
uptrakit-cli hosts list -o json | jq '.[].id'
uptrakit-cli scheduler show <TASK_ID> -o yaml
```

See also: [CLI Output Formatting](../development/cli-output.md).
