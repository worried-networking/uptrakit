# CLI Usage Guide

The `uptrakit` binary provides a command-line interface for interacting with the Uptrakit controller. It
supports authentication, resource inspection, version checks, update triggering, scheduler management,
plugin configuration, autodiscovery management, and server settings management.

## Global Options

Every command accepts these global flags:

| Flag | Description |
| --- | --- |
| `--server <URL>` | Controller URL (overrides stored config and `UPTRAKIT_SERVER` env var) |
| `--token <TOKEN>` | API token (overrides stored credentials and `UPTRAKIT_TOKEN` env var) |
| `--insecure` | Skip TLS certificate verification (development only; prints a warning to stderr) |
| `--timeout <SECONDS>` | API request timeout in seconds (overrides `UPTRAKIT_TIMEOUT`; default: 30) |
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
| `UPTRAKIT_TIMEOUT` | API request timeout in seconds (equivalent to `--timeout`; default: 30). Useful for CI pipelines or long-running controller operations. |

**Priority order:** CLI flag > environment variable > stored credentials file.

## Authentication

Before using most commands you must authenticate. The CLI uses a device authorization flow (RFC 8628).
When the user approves in the browser, the CLI receives the token instantly via SSE. If SSE is
unavailable (e.g. older server), it falls back to polling every 5 seconds.

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

# Update a service's settings (e.g. ping interval)
uptrakit services update <SERVICE_ID> --ping-interval 60

# Clear ping interval override (revert to service-type default)
uptrakit services update <SERVICE_ID> --ping-interval 0

# Merge a pending source service into an approved target service
uptrakit services merge <TARGET_ID> <SOURCE_ID>

# Enable update freeze on a connected service (blocks updates on the agent)
uptrakit services update-freeze <SERVICE_ID> --enable --reason "Incident investigation"

# Disable update freeze
uptrakit services update-freeze <SERVICE_ID> --disable
```

See also: [Service Operations](../api/services-operations.md),
[HTTP Web API](../api/http-web-api.md).

## System Services

List, inspect, and manage system services (MQTT bridge, external scheduler). System services are
tenant-agnostic infrastructure components managed separately from regular tenant services.

```sh
# List all system services (paginated)
uptrakit system-services list
uptrakit system-services list --page 2 --per-page 10

# Filter by capability or status
uptrakit system-services list --capability mqtt_bridge
uptrakit system-services list --status pending
uptrakit system-services list --capability scheduler --status approved

# Show system service details
uptrakit system-services show <SERVICE_ID>

# Approve a pending system service
uptrakit system-services approve <SERVICE_ID>

# Reject a pending system service
uptrakit system-services reject <SERVICE_ID>

# Remove (deactivate) a system service
uptrakit system-services remove <SERVICE_ID>

# Update a system service's settings
uptrakit system-services update <SERVICE_ID> --ping-interval 30
uptrakit system-services update <SERVICE_ID> --cert-lifetime-hours 48

# Clear overrides (revert to global defaults)
uptrakit system-services update <SERVICE_ID> --ping-interval 0
uptrakit system-services update <SERVICE_ID> --cert-lifetime-hours 0
```

`--capability` accepts any capability string, e.g. `mqtt_bridge` or `scheduler`.

`--ping-interval` sets the ping interval in seconds. `0` clears the override and reverts to the
service-profile default. The minimum positive value is `5`.

`--cert-lifetime-hours` sets the certificate lifetime in hours. `0` clears the override and reverts
to the global default. Valid positive range is `1`–`17520` (up to two years).

See also: [System Services Architecture](../architecture/system-services.md),
[HTTP Web API](../api/http-web-api.md#system-services-endpoints).

## System Enrollment Tokens

Manage backend-generated enrollment tokens for infrastructure services. A system service that
presents a valid token at enrollment is automatically approved; without a token it is queued as
`pending` for manual review. Tokens are Argon2id-hashed and shown only once at creation.

All commands require the `manage_system_services` permission.

```sh
# List all system enrollment tokens (paginated)
uptrakit system-enrollment-tokens list
uptrakit system-enrollment-tokens list --page 2 --per-page 10

# Create a new token (token value is shown once — copy it immediately)
uptrakit system-enrollment-tokens create --name "MQTT Bridge Token"
uptrakit system-enrollment-tokens create --name "Scheduler Token" \
    --max-uses 3 \
    --expires-in 604800

# Show metadata for a specific token (no token value — shown only at creation)
uptrakit system-enrollment-tokens show <TOKEN_ID>

# Revoke a token (soft-delete; existing enrolled services are unaffected)
uptrakit system-enrollment-tokens revoke <TOKEN_ID>
```

`--max-uses` limits the number of services that can use this token to auto-enroll. Omit for
unlimited.

`--expires-in` sets the token lifetime in seconds (e.g. `86400` = 24 hours, `604800` = 7 days).
Omit for a non-expiring token.

See also: [System Enrollment Tokens API](../api/http-web-api.md#system-enrollment-token-endpoints),
[Auth and Authorization](../security/auth-and-authorization.md).

## Hosts

List, inspect, and manage hosts registered with the controller.

```sh
# List all hosts (paginated)
uptrakit hosts list
uptrakit hosts list --page 2 --per-page 10

# Show host details
uptrakit hosts show <HOST_ID>

# Update a host's friendly display name
uptrakit hosts update <HOST_ID> --friendly-name "prod-web-01"

# Deactivate (remove) a host from the controller
uptrakit hosts deactivate <HOST_ID>

# Trigger autodiscovery on a specific host
uptrakit hosts discover <HOST_ID>

# List the discovery plugin allowlist for a specific host
uptrakit hosts discovery-allowlist list <HOST_ID>

# Add a plugin type to a host's discovery allowlist
uptrakit hosts discovery-allowlist add <HOST_ID> <PLUGIN_TYPE>

# Remove an entry from a host's discovery allowlist
uptrakit hosts discovery-allowlist remove <HOST_ID> <ENTRY_ID>
```

When a host has its own discovery allowlist entries, only those plugin types run during discovery
for that host. The tenant-wide allowlist is ignored entirely for hosts with their own entries.
Removing all entries from a host allowlist causes the host to fall back to the tenant-wide
allowlist (or the system default of all plugins if the tenant list is also empty).

See also: [Host Entity](../architecture/host-entity.md), [Autodiscovery](autodiscovery.md),
[Discovery Allowlist API](../api/discovery-allowlist.md).

## Host Tags

Manage host tags -- user-defined labels for organizing and classifying hosts.

### Tag management

```sh
# List all host tags (paginated)
uptrakit host-tags list
uptrakit host-tags list --page 2 --per-page 10

# Search by name
uptrakit host-tags list --search prod

# Show tag details
uptrakit host-tags show <TAG_ID>

# Create a new tag (color is auto-assigned from palette if omitted)
uptrakit host-tags create --name "production"
uptrakit host-tags create --name "staging" --color "#10B981" --description "Staging environment"

# Update a tag
uptrakit host-tags update <TAG_ID> --name "prod"
uptrakit host-tags update <TAG_ID> --color "#EF4444"
uptrakit host-tags update <TAG_ID> --description "Updated description"

# Clear the description
uptrakit host-tags update <TAG_ID> --clear-description

# Delete a tag (soft-delete; removes all host assignments)
uptrakit host-tags delete <TAG_ID>
```

### Assigning tags to hosts

```sh
# Set tags on a host (replaces all existing tag assignments)
uptrakit host-tags set <HOST_ID> --tags <TAG_ID_1>,<TAG_ID_2>

# Remove all tags from a host
uptrakit host-tags set <HOST_ID> --tags ""
```

### Batch actions

```sh
# Delete multiple tags at once
uptrakit host-tags batch delete <TAG_ID_1> <TAG_ID_2>
```

See also: [Host Tags API](../api/host-tags.md),
[Host Tags Architecture](../architecture/host-tags.md).

## Software Ignores

Manage ignore rules that suppress software from future discovery runs.

```sh
# List all ignore rules
uptrakit software-ignores list

# Create a tenant-wide ignore rule (suppresses by name across all hosts)
uptrakit software-ignores add --name telnet

# Create a host-specific ignore rule
uptrakit software-ignores add --host <HOST_ID> --plugin-config <PLUGIN_CONFIG_ID> --package nginx

# Delete an ignore rule
uptrakit software-ignores delete <IGNORE_ID>
```

See also: [Autodiscovery Guide](autodiscovery.md),
[Autodiscovery API](../api/autodiscovery.md),
[Unified Software Tracking](../architecture/unified-software-tracking.md).

## Software Items

List, inspect, and manage software items configured on the controller.

```sh
# List all software items (paginated)
uptrakit software-items list
uptrakit software-items list --page 1 --per-page 50

# Show software item details (includes host assignments and version info)
uptrakit software-items show <ITEM_ID>

# Create a new software item
uptrakit software-items create --name "my-app"
uptrakit software-items create --name "my-app" --enabled true

# Update a software item's name or enabled state
uptrakit software-items update <ITEM_ID> --name "my-app-renamed"
uptrakit software-items update <ITEM_ID> --enabled false
uptrakit software-items update <ITEM_ID> --name "new-name" --enabled true

# Delete a software item
uptrakit software-items delete <ITEM_ID>

# Assign a host to a software item
uptrakit software-items assign <ITEM_ID> --host <HOST_ID>
uptrakit software-items assign <ITEM_ID> --host <HOST_ID> --plugin-config <PLUGIN_CONFIG_ID>
uptrakit software-items assign <ITEM_ID> --host <HOST_ID> --plugin-config <PLUGIN_CONFIG_ID> --package "my-package"

# Unassign a host from a software item
uptrakit software-items unassign <ITEM_ID> --host <HOST_ID>

# Unassign a host and create a software ignore rule for this package
uptrakit software-items unassign <ITEM_ID> --host <HOST_ID> --ignore

# Trigger an update to the latest known version on a specific host
# (resolves the current latest_version automatically; errors if not yet known)
uptrakit software-items update-latest <ITEM_ID> --host <HOST_ID>
```

`software-items list` output includes an `UPDATE` column showing `yes` when any assigned host has
an available update.

`software-items show` output includes `Latest Version` and `Update Available` fields at the item
level, and `LATEST` / `STATUS` columns in the host table showing `up-to-date` or
`update-available` per host. The `STATUS` column only has a value when both `installed_version`
and `latest_version` are known.

`latest_version` is populated for agent-side plugins (Homebrew, PHS) immediately after a version
check. For controller-side plugins (GitHub Releases, Docker Registry), it is populated by the
scheduled upstream resolver. Run `uptrakit check item <ITEM_ID>` to trigger a fresh check before
using `update-latest`.

See also: [Software Item Entity](../architecture/software-item-entity.md),
[Autodiscovery](autodiscovery.md), [Plugin Configurations](plugin-configs.md).

## Plugin Configurations

Manage plugin configurations that define how software packages are tracked and where upstream
versions are resolved. See [Plugin Configurations](plugin-configs.md) for a full explanation
of plugin types and their fields.

```sh
# List all plugin configs (paginated)
uptrakit plugin-configs list
uptrakit plugin-configs list --page 1 --per-page 20

# Show plugin config details
uptrakit plugin-configs show <PLUGIN_CONFIG_ID>

# Create a new plugin config
uptrakit plugin-configs create --name "My App Releases" --type releases_github \
  --config '{"owner":"example","repo":"my-app"}'

uptrakit plugin-configs create --name "My Docker Image" --type releases_docker \
  --config '{"tracking_mode":"semver_tags"}'

uptrakit plugin-configs create --name "My Formula" --type package_manager_homebrew \
  --config '{"package_type":"formula","formula":"my-formula"}'

# Update a plugin config
uptrakit plugin-configs update <PLUGIN_CONFIG_ID> --name "Updated Name"
uptrakit plugin-configs update <PLUGIN_CONFIG_ID> --config '{"owner":"example","repo":"new-repo"}'

# Delete a plugin config
uptrakit plugin-configs delete <PLUGIN_CONFIG_ID>

# Trigger autodiscovery for a plugin config (discovery-capable plugins only)
uptrakit plugin-configs discover <PLUGIN_CONFIG_ID>

```

See also: [Plugin Configurations](plugin-configs.md), [Autodiscovery](autodiscovery.md).

## Discovery Allowlist

Control which plugin types participate in automatic host discovery. By default, all
discovery-capable plugins run on every host. Adding entries to an allowlist restricts discovery
to only the listed plugin types.

Two scopes exist: tenant-wide (applies to all hosts by default) and host-specific (overrides the
tenant-wide list for a single host). See [Autodiscovery: Controlling Which Plugins Run
Discovery](autodiscovery.md#controlling-which-plugins-run-discovery) for a full explanation.

```sh
# List the tenant-wide discovery allowlist
uptrakit discovery-allowlist list

# Add a plugin type to the tenant-wide allowlist
uptrakit discovery-allowlist add <PLUGIN_TYPE>

# Remove an entry from the tenant-wide allowlist
uptrakit discovery-allowlist remove <ENTRY_ID>
```

`<PLUGIN_TYPE>` must be one of the discovery-capable plugin types: `package_manager_apt`,
`package_manager_homebrew`, `releases_docker`, or `discovery_proxmox_helper_scripts`. Supplying
an unknown type or a type without the `DiscoverLocalSoftware` capability returns an error.

Adding an entry that already exists returns the existing entry without creating a duplicate.

See also: [Discovery Allowlist API](../api/discovery-allowlist.md), [Autodiscovery](autodiscovery.md).

## Version Checks

Trigger version checks to discover installed and available versions.

```sh
# Trigger bulk release fetch (all items, all hosts via scheduler)
uptrakit check all

# Trigger version check for a specific software item (all assigned hosts)
uptrakit check item <ITEM_ID>

# Scope to a specific host
uptrakit check item <ITEM_ID> --host <HOST_ID>
```

`check item` triggers a version check for a specific software item. The agent checks both installed
and available versions in a single operation.

`check all` locates the `fetch_releases` scheduler task and triggers it immediately. This causes the
controller to call external APIs for controller-side plugins (GitHub Releases, Docker Registry) and
dispatch release-index queries to agents for agent-side plugins (APT, Homebrew, npm). It does
**not** trigger installed-version detection — that runs on its own `detect_version` schedule (daily
by default). To trigger installed-version detection immediately, use
`uptrakit scheduler trigger <DETECT_VERSION_TASK_ID>`.

See also: [Wire Protocol](../api/wire-protocol.md), [HTTP Web API](../api/http-web-api.md),
[Scheduler](../architecture/scheduler.md).

## Updates

Trigger a software update on a specific host. Updates are always manual and user-initiated.

```sh
# Trigger an update to a specific version
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0"

# Follow the update output in real time (streams output until the update completes)
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0" --follow

# With optional release metadata
uptrakit update trigger <ITEM_ID> <HOST_ID> --to-version "2.0.0" \
  --release-tag "v2.0.0" \
  --release-url "https://github.com/example/repo/releases/tag/v2.0.0"
```

The `--follow` / `-f` flag connects to the real-time output stream after triggering the update.
Output lines (including ANSI escape codes) print to stdout; status messages print to stderr. Press
Ctrl+C to detach — the update continues in the background.

For a convenience shortcut that resolves the current `latest_version` automatically, use the
`software-items update-latest` subcommand (see the [Software Items](#software-items) section above).
That command errors with a clear message if no latest version is known yet, prompting you to run
`uptrakit check item <ITEM_ID>` first.

See also: [Update Workflow](update-workflow.md), [Update History Entity](../architecture/update-history-entity.md).

## Batch Updates

Trigger multiple updates in a single request. Two modes are available: host-wide (update
everything on a host) and item-wide (roll out a version to multiple hosts).

```sh
# Update all outdated software on a host
uptrakit update batch-host <HOST_ID>

# Only apply security patches
uptrakit update batch-host <HOST_ID> --category security

# Exclude specific items
uptrakit update batch-host <HOST_ID> --exclude <ITEM_ID_1>,<ITEM_ID_2>

# Follow batch progress in real time
uptrakit update batch-host <HOST_ID> --follow

# Roll out a software item to all assigned hosts
uptrakit update batch-item <ITEM_ID> --to-version "3.0.0"

# Limit to specific hosts
uptrakit update batch-item <ITEM_ID> --to-version "3.0.0" --host <HOST_ID_1>,<HOST_ID_2>

# Follow batch progress
uptrakit update batch-item <ITEM_ID> --to-version "3.0.0" --follow
```

The `--follow` / `-f` flag connects to the batch progress SSE stream after triggering.
Per-item status updates print to stderr. Press Ctrl+C to detach — the batch continues
in the background. Exit codes: 0 = completed, 1 = partially completed, 2 = other.

### Managing Batches

```sh
# List all update batches (paginated)
uptrakit update-batches list
uptrakit update-batches list --status in_progress --page 1 --per-page 10

# Show batch details with per-item status
uptrakit update-batches show <BATCH_ID>

# Follow batch progress via SSE
uptrakit update-batches follow <BATCH_ID>
```

See also: [Update Workflow](update-workflow.md#batch-updates),
[Update History Entity](../architecture/update-history-entity.md).

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

# Tail the live output of an in-progress update
uptrakit history tail <HISTORY_ID>
```

The `tail` subcommand streams update output in real time via SSE. Output lines print to
stdout with native ANSI escape code pass-through. Status changes print to stderr. Press
Ctrl+C to detach without aborting the update. Exit codes: 0 = completed, 1 = failed,
2 = detached or other.

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
NATS URL, and system alerts.

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

# Create with a custom CA certificate for private brokers (inline PEM)
uptrakit settings mqtt create --host broker --port 8883 --transport tls --ca-pem "$(cat ca.pem)"

# Create with a custom CA certificate from a file
uptrakit settings mqtt create --host broker --port 8883 --transport tls --ca-pem-file /path/to/ca.pem

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

### NATS URL (feature: `nats`)

Configure the NATS server URL used for cross-controller messaging. Changes require a controller restart.

```sh
# Show the current NATS URL (password is masked in output)
uptrakit settings nats get

# Set the NATS URL
uptrakit settings nats set --url nats://host:4222
uptrakit settings nats set --url nats://user:password@host:4222

# Clear the stored NATS URL (disables NATS transport on next restart)
uptrakit settings nats clear
```

> **Note:** The NATS URL is encrypted at rest. After a `set` or `clear`, the CLI prints a reminder that the
> change takes effect after the controller is restarted.

### System alerts

```sh
uptrakit settings alerts
```

See also: [Settings Runtime](../api/settings-runtime.md), [HTTP Web API](../api/http-web-api.md),
[PKI and Certificate Lifecycle](../security/pki-certificates.md),
[Auth and Authorization](../security/auth-and-authorization.md).

## Enrollment Tokens

Manage enrollment tokens that allow services to auto-enroll with approved status.

```sh
# List all enrollment tokens (paginated)
uptrakit enrollment-tokens list
uptrakit enrollment-tokens list --page 2 --per-page 10

# Create a new enrollment token
uptrakit enrollment-tokens create --name "CI Deploy Token"
uptrakit enrollment-tokens create --name "Agent Token" \
  --capabilities "software_discovery,update_hooks" \
  --max-uses 50 \
  --expires-in 604800

# Show a specific enrollment token
uptrakit enrollment-tokens show <TOKEN_ID>

# Revoke an enrollment token
uptrakit enrollment-tokens revoke <TOKEN_ID>
```

- `--capabilities`: comma-separated list of capabilities to restrict the token to.
  Omit for a wildcard token that matches any service type.
- `--max-uses`: maximum number of enrollments. Omit for unlimited.
- `--expires-in`: TTL in seconds. Omit for no expiration.

The `create` command displays the plaintext token value once. Save it immediately;
it cannot be retrieved later.

See also: [Enrollment Tokens API](../api/enrollment-tokens.md),
[Auth and Authorization](../security/auth-and-authorization.md).

## Notifications

Manage notification channels, rules, and view the delivery log. Notifications alert you when
events occur (updates available, updates completed/failed, new software discovered, new services
enrolled, CA rotation).

### Channels

```sh
# List all notification channels (paginated)
uptrakit notifications channels list
uptrakit notifications channels list --page 1 --per-page 10

# Show channel details
uptrakit notifications channels get <CHANNEL_ID>

# Create a webhook channel
uptrakit notifications channels create --name "My Webhook" --type webhook \
  --config '{"url":"https://example.com/hooks/uptrakit","secret":"my-hmac-secret"}'

# Create a Telegram channel
uptrakit notifications channels create --name "Ops Telegram" --type telegram \
  --config '{"bot_token":"123456:ABC-DEF","chat_id":"-100123456789","webhook_secret":"random"}'

# Update a channel
uptrakit notifications channels update <CHANNEL_ID> --name "New Name"
uptrakit notifications channels update <CHANNEL_ID> --enabled false

# Delete a channel
uptrakit notifications channels delete <CHANNEL_ID>

# Test a channel (sends a test notification)
uptrakit notifications channels test <CHANNEL_ID>
```

### Rules

```sh
# List all rules (paginated, with optional filters)
uptrakit notifications rules list
uptrakit notifications rules list --channel-id <CHANNEL_ID>
uptrakit notifications rules list --event-type update_available

# Show rule details
uptrakit notifications rules get <RULE_ID>

# Create a rule
uptrakit notifications rules create --channel-id <CHANNEL_ID> --event-type update_available
uptrakit notifications rules create --channel-id <CHANNEL_ID> --event-type update_failed \
  --host-id <HOST_ID>

# Update a rule
uptrakit notifications rules update <RULE_ID> --enabled false

# Delete a rule
uptrakit notifications rules delete <RULE_ID>
```

### Delivery Log

```sh
# View delivery history (paginated)
uptrakit notifications log
uptrakit notifications log --page 1 --per-page 50
```

See also: [Notifications Guide](notifications.md), [Notifications API](../api/notifications.md).

## Extensions

Extensions are dynamic UI elements contributed by plugins (built-in) or connected
services. The CLI exposes them through the `extensions` command group.

### Listing extensions

```sh
# List all registered extensions.
uptrakit extensions list

# List connected service instances providing an extension.
uptrakit extensions providers <EXTENSION_ID>
```

### Raw invocation

```sh
# Invoke an action with raw JSON params.
uptrakit extensions invoke <EXTENSION_ID> <ACTION_ID> --params '{"key":"value"}'

# Targeted extensions require --service-id.
uptrakit extensions invoke ssh-agent.hosts list-hosts --service-id <UUID>
```

### Dynamic (manifest-driven) invocation

Extensions register a manifest describing their UI, actions, and form fields.
The CLI builds subcommands dynamically from this manifest so you can invoke
actions with typed arguments instead of raw JSON.

```sh
# Dynamic invocation — the CLI fetches the manifest and builds args.
uptrakit extensions <EXTENSION_ID> <ACTION_ID> [--arg value ...]

# Example: Proxmox VE discovery with a specific plugin config.
uptrakit extensions proxmox.hosts discover --plugin-config-id <UUID>

# Example: list Proxmox VE hosts.
uptrakit extensions proxmox.hosts list --plugin-config-id <UUID>
```

Extensions with a **context selector** (e.g., a dropdown in the web UI to pick a
plugin configuration) expose it as a global `--<param>` flag. The selected value
is injected into every action's params automatically.

Actions that define an `api_submit` target (e.g., "Add Configuration") call the
REST API directly instead of routing through the extension proxy.

See also: [Extensions Guide](extensions.md),
[Extensions Development](../development/extensions.md).

## Batch Actions

Most resource commands support a `batch` subcommand for performing the same operation on
multiple entities at once. The general syntax is:

```sh
uptrakit <resource> batch <ACTION> <UUID1> <UUID2> ...
```

The action name matches the single-entity command (e.g., `approve`, `delete`, `deactivate`).
Results are reported per-item, showing which succeeded and which failed.

### Services

```sh
# Approve multiple pending services
uptrakit services batch approve <UUID1> <UUID2> <UUID3>

# Reject multiple services
uptrakit services batch reject <UUID1> <UUID2>

# Deactivate multiple services
uptrakit services batch deactivate <UUID1> <UUID2>
```

### System services

```sh
uptrakit system-services batch approve <UUID1> <UUID2>
uptrakit system-services batch deactivate <UUID1> <UUID2>
```

### Hosts

```sh
# Deactivate multiple hosts
uptrakit hosts batch deactivate <UUID1> <UUID2>
```

### Software items

```sh
# Delete multiple software items
uptrakit software-items batch delete <UUID1> <UUID2>
```

### Plugin configs

```sh
uptrakit plugin-configs batch delete <UUID1> <UUID2>
```

### Host tags

```sh
uptrakit host-tags batch delete <UUID1> <UUID2>
```

### Software ignores

```sh
uptrakit software-ignores batch delete <UUID1> <UUID2>
```

See also: [Batch Actions API](../api/batch-actions.md),
[HTTP Web API](../api/http-web-api.md#batch-action-endpoints).

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
