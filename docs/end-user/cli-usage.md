# CLI Usage Guide

The `uptrakit` binary provides a command-line interface for interacting with the Uptrakit controller. It
supports authentication, resource inspection, version checks, update triggering, scheduler management, and
server settings management.

## Global Options

Every command accepts these global flags:

| Flag | Description |
| --- | --- |
| `--server <URL>` | Controller URL (overrides stored config and `UPTRAKIT_SERVER` env var) |
| `--token <TOKEN>` | API token (overrides stored credentials and `UPTRAKIT_TOKEN` env var) |
| `--insecure` | Skip TLS certificate verification (development only; prints a warning to stderr) |
| `-o`, `--output <FORMAT>` | Output format: `human` (default), `json`, `yaml` |
| `--version` | Show version and build metadata |

## Entity IDs

All entity ID arguments (host IDs, service IDs, software item IDs, task IDs,
etc.) must be valid UUIDs. The CLI validates UUID format at parse time and
rejects invalid values immediately with a descriptive error message.

## Environment Variables

The CLI reads the following environment variables. CLI flags take precedence over environment
variables, which take precedence over stored configuration.

| Variable | Description |
| --- | --- |
| `UPTRAKIT_SERVER` | Controller URL (equivalent to `--server`) |
| `UPTRAKIT_TOKEN` | API token (equivalent to `--token`; preferred over `--token` for automation to avoid exposing tokens in process listings) |

**Priority order:** CLI flag > environment variable > stored credentials file.

## Authentication

Before using most commands you must authenticate. The CLI uses a device authorization flow (RFC 8628).

```sh
# Log in via browser
uptrakit auth login

# Check current auth status
uptrakit auth status

# Manage API tokens
uptrakit auth token create --name "ci-token"
uptrakit auth token list
uptrakit auth token revoke <TOKEN_ID>
```

See also: [Auth Flows](../api/auth-flows.md), [Auth and Authorization](../security/auth-and-authorization.md).

## Services

List, inspect, and manage services (agents, MQTT, SSH agents) registered
with the controller.

```sh
# List all services (paginated)
uptrakit services list
uptrakit services list --page 2 --per-page 10

# Filter by type or status
uptrakit services list --type agent
uptrakit services list --status pending
uptrakit services list --type mqtt --status approved

# Show service details
uptrakit services show <SERVICE_ID>

# Approve a pending service
uptrakit services approve <SERVICE_ID>

# Reject a pending service
uptrakit services reject <SERVICE_ID>

# Remove (deactivate) a service
uptrakit services remove <SERVICE_ID>

# Merge a pending source service into an approved target service
uptrakit services merge <TARGET_ID> <SOURCE_ID>
```

See also: [Service Operations](../api/services-operations.md),
[HTTP Web API](../api/http-web-api.md).

## Hosts

List and inspect hosts registered with the controller.

```sh
# List all hosts (paginated)
uptrakit hosts list
uptrakit hosts list --page 2 --per-page 10

# Show host details
uptrakit hosts show <HOST_ID>
```

See also: [Host Entity](../architecture/host-entity.md).

## Software Items

List and inspect software items configured on the controller.

```sh
# List all software items (paginated)
uptrakit software-items list
uptrakit software-items list --page 1 --per-page 50

# Show software item details (includes host assignments and version info)
uptrakit software-items show <ITEM_ID>
```

See also: [Software Item Entity](../architecture/software-item-entity.md).

## Version Checks

Trigger version checks to discover installed and available versions.

```sh
# Trigger bulk version check (all items, all hosts via scheduler)
uptrakit check all

# Trigger version check for a specific software item (all assigned hosts)
uptrakit check item <ITEM_ID>

# Scope to a specific host
uptrakit check item <ITEM_ID> --host <HOST_ID>
```

`check item` triggers a version check for a specific software item. The agent checks both installed and available versions in a single operation.

`check all` finds the `version_check` scheduler task and triggers it.

See also: [Wire Protocol](../api/wire-protocol.md), [HTTP Web API](../api/http-web-api.md).

## Updates

Trigger a software update on a specific host. Updates are always manual and user-initiated.

```sh
# Trigger an update
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0"

# With optional release metadata
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0" \
  --release-tag "v2.0.0" \
  --release-url "https://github.com/example/repo/releases/tag/v2.0.0"
```

See also: [Update Workflow](update-workflow.md), [Update History Entity](../architecture/update-history-entity.md).

## Update History

View the history of updates across hosts and software items.

```sh
# List all update history (paginated)
uptrakit history list

# Filter by host, software item, or status
uptrakit history list --host <HOST_ID>
uptrakit history list --software-item <ITEM_ID>
uptrakit history list --status completed
uptrakit history list --host <HOST_ID> --status failed --page 1 --per-page 5

# Show details for a specific history entry
uptrakit history show <HISTORY_ID>
```

See also: [Update History Entity](../architecture/update-history-entity.md).

## Scheduler

Inspect and trigger scheduled tasks (version checks, cleanup, CA rotation).

```sh
# List all scheduled tasks
uptrakit scheduler list

# Show task details
uptrakit scheduler show <TASK_ID>

# Trigger immediate execution
uptrakit scheduler trigger <TASK_ID>
```

See also: [Scheduler](../architecture/scheduler.md), [HTTP Web API](../api/http-web-api.md).

## Settings

View and manage server settings: registration, authentication, certificates, network, MQTT, OIDC providers,
and system alerts.

### Combined overview

```sh
# Show all settings at a glance
uptrakit settings show
```

### Registration

```sh
uptrakit settings registration show
uptrakit settings registration update --mode invite --token "my-secret"
uptrakit settings registration update --mode closed
uptrakit settings registration update --mode invite --require-token-for-oidc true
```

### Authentication

```sh
uptrakit settings authentication show
uptrakit settings authentication update --password-auth-enabled false
```

### Certificates

```sh
uptrakit settings certificates show
uptrakit settings certificates update --lifetime-days 365 --renewal-window-hours 72
```

### Network

```sh
uptrakit settings network show
uptrakit settings network update --trusted-proxies "10.0.0.0/8,172.16.0.0/12"
uptrakit settings network update --real-ip-header X-Real-IP --https-addr 0.0.0.0:8443
uptrakit settings network update --extra-sans "alt.example.com,10.0.0.1"
uptrakit settings network update --pki-addr "https://pki.example.com"
```

The `--trusted-proxies` and `--extra-sans` flags accept comma-separated values.

### CA rotation and server certificate renewal

```sh
uptrakit settings rotate-ca
uptrakit settings renew-server-cert
```

### MQTT

```sh
# List and inspect MQTT client configurations
uptrakit settings mqtt list
uptrakit settings mqtt show <ID>

# Create a new MQTT configuration
uptrakit settings mqtt create --url "mqtt://broker:1883" --enabled true
uptrakit settings mqtt create --host broker --port 8883 --transport tls --client-id uptrakit-1

# Update an existing configuration
uptrakit settings mqtt update <ID> --enabled false
uptrakit settings mqtt update <ID> --host new-broker --port 8883

# Delete a configuration
uptrakit settings mqtt delete <ID>

# Manage MQTT client limit
uptrakit settings mqtt limit show
uptrakit settings mqtt limit update --max 10
```

### OIDC providers

```sh
# List and inspect OIDC providers
uptrakit settings oidc list
uptrakit settings oidc show <ID>

# Create a provider (role-mapping is not supported via CLI; use `uptrakit api` for that)
uptrakit settings oidc create \
  --name "Google" \
  --slug google \
  --issuer-url "https://accounts.google.com" \
  --client-id "cid-123" \
  --client-secret "cs-456"

# Update a provider
uptrakit settings oidc update <ID> --name "Google Workspace" --auto-create-users false

# Delete a provider
uptrakit settings oidc delete <ID>

# Activate / deactivate
uptrakit settings oidc activate <ID>
uptrakit settings oidc deactivate <ID>
```

### System alerts

```sh
uptrakit settings alerts
```

See also: [Settings Runtime](../api/settings-runtime.md), [HTTP Web API](../api/http-web-api.md),
[PKI and Certificate Lifecycle](../security/pki-certificates.md),
[Auth and Authorization](../security/auth-and-authorization.md).

## Raw API Access

For advanced use, the `api` command lets you call any REST endpoint directly.

```sh
uptrakit api GET /api/v1/auth/me
uptrakit api POST /api/v1/software-items --data '{"name":"test"}'
uptrakit api GET /api/v1/hosts -o json
```

## Output Formats

All commands support three output formats via `--output` / `-o`:

- **`human`** (default): columnar or line-based text suitable for terminals.
- **`json`**: compact single-line JSON, suitable for `jq` piping.
- **`yaml`**: YAML output for configuration workflows.

```sh
uptrakit hosts list -o json | jq '.[].id'
uptrakit scheduler show <TASK_ID> -o yaml
```

See also: [CLI Output Formatting](../development/cli-output.md).
