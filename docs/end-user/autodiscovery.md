# Autodiscovery

Uptrakit can automatically discover software installed on your hosts when an agent connects.
Instead of manually creating a software item for every installed package, autodiscovery lets
the agent do the initial inventory work.

All discovered items are tracked immediately -- there is no pending state or approval workflow.
The `featured` flag on each item controls how it appears in the UI:

- **Featured items** (Docker, Proxmox Helper Scripts) appear individually in the main Software
  list.
- **Non-featured items** (APT, Homebrew, npm, Mac App Store) appear as aggregated per-host
  package summaries.

## How It Works

When an agent registers a new host (or reconnects with a previously unseen host), the controller
sends a discovery request to that agent. The agent queries each of its discovery-capable plugins
and returns a list of installed packages.

### Featured vs non-featured routing

Each discovered item declares a **featured** flag that determines how the controller presents
it:

| Featured | UI location | Typical plugins |
| --- | --- | --- |
| `true` | Main Software list (individual entries) | Docker, Proxmox Helper Scripts |
| `false` | Host detail page (aggregated package view) | APT, Homebrew, Mac App Store, npm |

Plugins set the `featured` flag explicitly during discovery. The controller routes items based
on this flag -- see [Plugin Guidelines](../development/plugin-guidelines.md) for plugin
implementation details.

### Discovery flow

For all discovered items, the controller creates software items with `enabled: true`
immediately. Discovery results carry structured **discovery targets** that tell the controller
exactly which plugin config and roles to create.

For featured items with discovery targets, the controller creates role-based plugin
assignments:

| Role | Assignment |
| --- | --- |
| `detect_version` | Uses the target plugin config to detect the installed version on the host. |
| `fetch_releases` | Uses the target plugin config to fetch the latest upstream version. |
| `execute_update` | Uses the target plugin config to run updates. |

All assignments default to `execution_site: auto`, meaning the system decides where each
operation runs based on the plugin's capabilities. Plugins that support controller-side release
fetching (GitHub Releases, Docker) will have their `fetch_releases` role executed on the
controller automatically. Plugins that require a local package index (Homebrew, APT) always run
on the agent.

For non-featured items, the `plugin_config_id` and `package_identifier` are stored directly on
the host-software junction row -- no separate role assignments are needed.

Plugin assignment information is stored on the host assignment -- not on the software item
itself -- so the same software item (e.g. "git") can later appear on multiple hosts via
different plugins, all under one catalog entry. You can later customize the role assignments
per host if needed (for example, switching the `fetch_releases` role to a different plugin
config).

Discovery-capable plugins currently supported:

| Plugin | What it discovers |
| --- | --- |
| APT | Debian/Ubuntu packages installed via APT |
| Docker | Running and stopped containers on the host -- one software item per container |
| Homebrew (Formulae) | Homebrew formula packages installed on the host |
| Homebrew (Casks) | Homebrew cask packages installed on the host |
| Mac App Store | Apps installed from the Mac App Store via `mas list` |
| Proxmox Helper Scripts | Applications managed by community Proxmox VE helper scripts |

If no plugin config exists for a discovery-capable plugin when a host registers, the plugin
emits **discovery targets** that tell the controller which plugin configs to create
automatically (for example, `"Docker"`, `"Homebrew (Formulae)"`, or `"Proxmox Helper
Scripts"`). This means the feature works out of the box on supported hosts with no manual
configuration required.

### Proxmox Helper Scripts discovery

PHS discovery works differently from other plugins. Instead of creating software items linked
to the PHS plugin config, the PHS plugin analyses each container's CT script to identify the
upstream source and emits **discovery targets** that tell the controller which plugin config to
create:

- **GitHub-managed apps** (e.g. Booklore, Radarr, Sonarr, Pangolin, Uptime Kuma): The PHS
  plugin emits a target for the `releases_github` plugin type, pre-configured with the
  repository owner and name, the installed version detection command, and the unattended update
  script (`sudo /usr/local/bin/uptrakit-phs-update`). The controller auto-creates the GitHub
  Releases plugin config from the target. The software item's plugin config will be the
  auto-created GitHub config, not the PHS config.

- **npm-managed apps** (e.g. n8n, Zigbee2MQTT): The PHS plugin emits two targets -- one for
  the `package_manager_npm` plugin type (version detection and release fetching) and one for
  the `generic_shell` plugin type (updates via `/usr/bin/update`). Version information comes
  from npm, but updates always use the PHS update script.

- **APT-managed apps** (e.g. Grafana, Plex): The PHS plugin emits two targets -- one for the
  `package_manager_apt` plugin type (version detection and release fetching) and one for the
  `generic_shell` plugin type (updates via `/usr/bin/update`). The software item's
  `package_identifier` is the Debian package name. Version information comes from APT, but
  updates always use the PHS update script.

- **Undetectable apps**: Apps whose scripts contain neither a GitHub release source nor a
  specific `apt install` line are skipped. A warning is logged on the agent. Check agent logs
  (`journalctl -u uptrakit-agent`) if you expect to see an app but it does not appear.

After discovery, version checking is handled by the target plugin configs (`releases_github`,
`NPM`, or `APT`), while updates are always handled by the PHS Shell config (which runs
`/usr/bin/update`). The PHS plugin config itself is not linked directly to software items.

**Migration note**: Existing PHS-discovered npm/APT items retain their old plugin assignments.
To apply the new update routing, delete the affected items and re-trigger discovery, or
manually reassign the `execute_update` role to the GenericShell (PHS Shell) plugin config.

Auto-created config name for the PHS discovery anchor: **`"Proxmox Helper Scripts"`**.

### Docker discovery

The Docker plugin discovers containers by querying the local Docker daemon for all containers
(running and stopped). For each container image that is not a bare SHA digest:

- Images with no registry provenance (locally built, no `RepoDigests`) are skipped.
- Container names are stored as extra metadata.
- The `package_identifier` for discovered items is the full image reference including tag.

Auto-created config name: **`"Docker"`** (one shared config per tenant; no per-host split).

See [Docker Plugin](plugins/docker.md#autodiscovery) for the full discovery behaviour and name
derivation rules.

## The Ignore List

The ignore list lets you suppress specific software from appearing in future discovery runs.
Uptrakit supports two types of ignore rules via the unified `software_ignores` table:

- **Tenant-wide rules**: suppress software by name across all hosts and plugin configs.
- **Host-specific rules**: suppress software on a specific host, scoped to a plugin config
  and package identifier.

Once an ignore rule exists, autodiscovery will skip any matching item when it would otherwise
create or update a software item.

### Adding a name to the ignore list

You can add a name to the ignore list in two ways:

**When removing a host assignment** -- Use **Delete and Ignore** in the Web UI context menu on
a host assignment, or pass `?ignore=true` when deleting a host assignment via the API
(`DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true`). This removes the host
assignment and simultaneously creates an ignore rule so the software item will not be
re-discovered on any host.

**Directly via the API or CLI** -- Create an ignore rule by name without needing to remove an
existing assignment first. This is useful for pre-suppressing software you know you will never
want before it appears.

### Removing an ignore rule

Delete the ignore rule by its ID. After removal, the next discovery run will be able to create
a software item for that name again.

See the [API reference](../api/autodiscovery.md#software-ignore-rules) for managing ignore
rules via the API.

## Controlling Which Plugins Run Discovery

By default, when you have not configured an allowlist, all discovery-capable plugin types run on
every host. Once you add at least one entry to an allowlist, only the listed plugin types will
be used during discovery for the applicable scope.

### Tenant-wide allowlist

The tenant-wide allowlist sets a default for all hosts. For example, if you add
`package_manager_apt` to the tenant-wide allowlist, only APT discovery runs on every host
unless a host has its own allowlist that overrides this.

```sh
# Allow only APT discovery across all hosts by default
uptrakit discovery-allowlist add package_manager_apt

# View the current tenant-wide allowlist
uptrakit discovery-allowlist list

# Remove an entry (restores all-plugins default if the list becomes empty)
uptrakit discovery-allowlist remove <ENTRY_ID>
```

### Host-specific allowlist

You can override the tenant-wide allowlist for individual hosts. If a host has any entries in
its own allowlist, those entries are used exclusively -- the tenant-wide list does not apply to
that host at all.

```sh
# Allow only Homebrew discovery on a specific host
uptrakit hosts discovery-allowlist add <HOST_ID> package_manager_homebrew

# View the allowlist for a specific host
uptrakit hosts discovery-allowlist list <HOST_ID>

# Remove an entry from a host's allowlist
uptrakit hosts discovery-allowlist remove <HOST_ID> <ENTRY_ID>
```

### Priority and defaults

The controller resolves which plugins to run using this priority order:

1. Host-specific allowlist entries -- if the host has any, only those plugin types run.
2. Tenant-wide allowlist entries -- if the tenant has entries but the host has none, the
   tenant-wide list is used.
3. No entries anywhere -- all discovery-capable plugins run (the default out-of-the-box
   behavior).

Removing all entries from a list restores the more permissive fallback for that scope. Removing
all entries from both lists restores the system default of running all plugins.

### Plugin-config-level discovery bypasses the allowlist

The allowlist applies to host-level discovery triggers: automatic discovery on new host
registration and `POST /api/v1/hosts/{id}/discover`. It does **not** apply to
`POST /api/v1/plugin-configs/{id}/discover` (`uptrakit plugin-configs discover`). When you
explicitly invoke a specific plugin config, that config always runs regardless of the allowlist.

See [Discovery Allowlist API](../api/discovery-allowlist.md) for the full endpoint reference.

## Periodic Software Rediscovery

Uptrakit automatically rediscovers software on a recurring schedule via the `discover_software`
scheduled task (default: every 6 hours).

Each cycle, the controller sends a fresh `DiscoverSoftware` message to every active agent-backed
host. The agent runs all applicable discovery plugins and reports back the current state of
installed packages.

### Disappeared packages

If a package that previously appeared in a discovery run is absent from a subsequent run, the
host-software junction row is automatically soft-deleted (`deactivated_at` is set to the current
time). Soft-deleted items are no longer shown in the UI. If the package is reinstalled later, it
will reappear in the next discovery run.

Items that have been explicitly ignored (via an ignore rule) are never deactivated by the
rediscovery process, even if they are absent from the agent's report.

### Configuring the schedule

The `discover_software` task runs on a fixed interval (default: 21 600 seconds / 6 hours,
jitter: 300 seconds). You can adjust the interval or disable the task from the Web UI
(**Settings -> Scheduler**) or via the API.

### Auto-updating packages excluded from discovery

Homebrew casks that declare `"auto_updates": true` (such as Google Chrome or Slack) manage their
own update mechanism and cannot be upgraded via `brew upgrade`. These casks are silently excluded
from all discovery passes -- both the initial registration discovery and the periodic
rediscovery. They will not appear in the software list.

## Triggering Discovery Manually

Autodiscovery runs automatically when an agent registers a new host and periodically every
6 hours. You can also trigger it on demand:

- **Web UI** -- Go to **Hosts**, open the context menu for any host, and select
  **Trigger Discovery**. A toast notification confirms how many plugins were queued.
- **Trigger for a specific host** -- runs discovery across all plugin configs associated
  with that host.
- **Trigger for a specific plugin config** -- runs discovery for that plugin across all
  connected agents. Returns an error if the plugin type does not support discovery.

See [POST /api/v1/hosts/{id}/discover](../api/autodiscovery.md#post-apiv1hostsiddiscover) and
[POST /api/v1/plugin-configs/{id}/discover](../api/autodiscovery.md#post-apiv1plugin-configsiddiscover)
in the API reference.

## Typical Workflow

1. An agent connects and registers a new host.
2. Uptrakit sends a discovery request. The agent queries Homebrew, Proxmox Helper Scripts, or
   other supported plugins.
3. All discovered software is tracked immediately with `enabled: true`.
4. Featured items appear individually in the **Software** list. Non-featured items appear as
   aggregated summaries on each host's detail page.
5. Version checking begins on the next scheduler cycle.
6. You can ignore items you do not want tracked -- ignored items are suppressed from future
   discovery runs.

## Related Documentation

- [API Reference: Autodiscovery](../api/autodiscovery.md) -- endpoint details, request/response
  shapes, and ignore rule management.
- [API Reference: Discovery Allowlist](../api/discovery-allowlist.md) -- full endpoint reference
  for tenant-wide and host-specific allowlist management.
- [Unified Software Tracking](../architecture/unified-software-tracking.md) -- architecture and
  data model.
- [Software Item Entity](../architecture/software-item-entity.md) -- underlying data model and
  database schema.
- [System Overview](system-overview.md) -- agent and plugin architecture.
- [Update Workflow](update-workflow.md) -- what happens after software is discovered and version
  tracking begins.
