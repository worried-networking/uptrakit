# Autodiscovery

Uptrakit can automatically discover software installed on your hosts when an agent connects, and
surface those packages for your review. Instead of manually creating a software item for every
installed package, autodiscovery lets the agent do the initial inventory work.

Discovered items are held in a "pending" state until you decide what to do with them. You are
always in control: Uptrakit never begins tracking or checking for updates on a discovered package
without your explicit approval.

## How It Works

When an agent registers a new host (or reconnects with a previously unseen host), the controller
sends a discovery request to that agent. The agent queries each of its discovery-capable plugins
and returns a list of installed packages. The controller then creates software items in a `pending`
state for any packages it has not seen before.

Discovery results carry structured **discovery targets** that tell the controller exactly which
plugin config and roles to create. When a discovered item is approved, the controller creates
role-based plugin assignments for the host based on the targets:

| Role | Assignment |
| --- | --- |
| `detect_version` | Uses the target plugin config to detect the installed version on the host. |
| `fetch_releases` | Uses the target plugin config to fetch the latest upstream version. |
| `execute_update` | Uses the target plugin config to run updates. |

All assignments default to `execution_site: auto`, meaning the system decides where each
operation runs based on the plugin's capabilities. Plugins that support controller-side release
fetching (GitHub Releases, Docker) will have their `fetch_releases` role executed on the controller
automatically. Plugins that require a local package index (Homebrew, APT) always run on the agent.

Plugin assignment information is stored on the host assignment — not on the software item itself —
so the same software item (e.g. "git") can later appear on multiple hosts via different plugins,
all under one catalog entry. You can later customize the role assignments per host if needed (for
example, switching the `fetch_releases` role to a different plugin config).

Discovery-capable plugins currently supported:

| Plugin | What it discovers |
| --- | --- |
| APT | Debian/Ubuntu packages installed via APT |
| Docker | Running and stopped containers on the host, grouped by image reference |
| Homebrew (Formulae) | Homebrew formula packages installed on the host |
| Homebrew (Casks) | Homebrew cask packages installed on the host |
| Proxmox Helper Scripts | Applications managed by community Proxmox VE helper scripts |

If no plugin config exists for a discovery-capable plugin when a host registers, the plugin
emits **discovery targets** that tell the controller which plugin configs to create automatically
(for example, `"Docker"`, `"Homebrew (Formulae)"`, or `"Proxmox Helper Scripts"`). This means
the feature works out of the box on supported hosts with no manual configuration required.

### Proxmox Helper Scripts discovery

PHS discovery works differently from other plugins. Instead of creating software items linked
to the PHS plugin config, the PHS plugin analyses each container's CT script to identify the
upstream source and emits **discovery targets** that tell the controller which plugin config to
create:

- **GitHub-managed apps** (e.g. Booklore, Radarr, Sonarr, Pangolin, Uptime Kuma): The PHS plugin
  emits a target for the `releases_github` plugin type, pre-configured with the repository owner
  and name, the installed version detection command, and the unattended update script
  (`sudo /usr/local/bin/uptrakit-phs-update`). The controller auto-creates the GitHub Releases
  plugin config from the target. The software item's plugin config will be the auto-created GitHub
  config, not the PHS config.

- **APT-managed apps** (e.g. Grafana, Plex): The PHS plugin emits a target for the `package_manager_apt` plugin
  type. The controller finds or creates a shared `APT (auto)` plugin config. The software item's
  `package_identifier` is the Debian package name.

- **Undetectable apps**: Apps whose scripts contain neither a GitHub release source nor a specific
  `apt install` line are skipped. A warning is logged on the agent. Check agent logs
  (`journalctl -u uptrakit-agent`) if you expect to see an app but it does not appear as pending.

After approving PHS-discovered items, version checking and updates are handled by the target
`releases_github` or `APT` configs — not by the PHS plugin itself.

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

## Software Item States

Every software item in Uptrakit has a `discovery_state` that describes its origin and review
status:

| State | Meaning | Tracking active? |
| --- | --- | --- |
| `null` (manual) | Created manually by a user | Yes |
| `pending` | Discovered by an agent, awaiting your review | No |
| `approved` | Discovered item you have reviewed and approved for tracking | Yes |

Pending items have version tracking disabled (`enabled: false`). Approving an item enables
tracking and begins checking for available updates.

## Reviewing Discovered Items

Pending items appear in the **Software** page of the Web UI under the **Pending** tab. You can
also identify them by their `discovery_state: "pending"` field when using the CLI or API.

For each pending item you have three options:

**Approve** — Enables the item for version tracking. Uptrakit will begin checking for installed
and available versions on the next scheduled check cycle. Use this for packages you want to
monitor and update through Uptrakit.

**Delete** — Soft-deletes the item. The package is removed from your list but can be re-discovered
in the future if autodiscovery runs again for that host. Use this when you do not need to track a
specific package right now but may want it discovered again later.

**Ignore** — Permanently suppresses the package from future discovery. Adding a package to the
ignore list prevents Uptrakit from ever creating a pending item for it again. Use this for packages
you will never want to track.

### Reviewing items in the Web UI

1. Go to **Software** → **Pending** tab to see all items awaiting review.
2. Use the **⋯** context menu on any item to **Approve**, **Delete**, or **Delete & Ignore** it.
3. Approved items move to the **Active** tab and version tracking begins immediately.

## The Ignore List

The ignore list lets you permanently suppress specific packages from appearing in future discovery
runs. An ignore rule is keyed on a `(plugin_config, package_identifier)` pair — meaning it is
scoped to the specific plugin and package name, and applies across all hosts.

Once an ignore rule exists, autodiscovery will skip that package entirely when it would otherwise
create a pending item for it.

### Adding a package to the ignore list

You can add a package to the ignore list in two ways:

**When removing a host assignment** — Use **Delete & Ignore** in the Web UI context menu on a host
assignment, or pass `?ignore=true` when deleting a host assignment via the API
(`DELETE /api/v1/software-items/{id}/hosts/{host_id}?ignore=true`). This removes the host
assignment and simultaneously creates an ignore rule so the package will not be re-discovered
on any host.

**Directly via the API** — Create an ignore rule for any `(plugin_config_id,
package_identifier)` combination without needing to remove an existing assignment first. This is
useful for pre-suppressing packages you know you will never want before they appear.

### Removing an ignore rule

Delete the ignore rule by its ID. After removal, the next discovery run for the relevant host and
plugin will be able to create a pending item for that package again.

See the [API reference](../api/autodiscovery.md#get-apiv1autodiscoveryignores) for managing
ignore rules via the API.

## Triggering Discovery Manually

Autodiscovery runs automatically when an agent registers a new host. You can also trigger it on
demand:

- **Web UI** — Go to **Hosts**, open the **⋯** context menu for any host, and select
  **Trigger Discovery**. A toast notification confirms how many plugins were queued.
- **Trigger for a specific host** — runs discovery across all plugin configs associated
  with that host.
- **Trigger for a specific plugin config** — runs discovery for that plugin across all
  connected agents. Returns an error if the plugin type does not support discovery.

See [POST /api/v1/hosts/{id}/discover](../api/autodiscovery.md#post-apiv1hostsiddiscover) and
[POST /api/v1/plugin-configs/{id}/discover](../api/autodiscovery.md#post-apiv1plugin-configsiddiscover)
in the API reference.

## Bulk Discard

If you want to clear out all pending discovered items at once — for example after an initial host
registration produces a large list you do not want to review individually — you can bulk-discard
them:

- **By host** — removes all pending items for a specific host. Optionally filter to a single
  plugin config.
- **By plugin config** — removes all pending items across all hosts for a specific plugin
  config.

Bulk discard performs a soft-delete. No ignore rules are created, so discarded packages can be
re-discovered in a future run.

See [DELETE /api/v1/hosts/{id}/discovered](../api/autodiscovery.md#delete-apiv1hostsdiscovered)
and
[DELETE /api/v1/plugin-configs/{id}/discovered](../api/autodiscovery.md#delete-apiv1plugin-configsiddiscovered)
in the API reference.

## Typical Workflow

1. An agent connects and registers a new host.
2. Uptrakit sends a discovery request. The agent queries Homebrew, Proxmox Helper Scripts, or
   other supported plugins.
3. Discovered packages appear in the **Software → Pending** tab with a **Pending** badge.
4. You review the list and choose for each item:
   - Approve the items you want Uptrakit to track and update.
   - Delete items you do not need right now (re-discoverable later).
   - Ignore items you never want to see again.
5. Approved items immediately become active: role-based plugin assignments are created per the
   discovery targets (typically all three: `detect_version`, `fetch_releases`, `execute_update`)
   with `execution_site: auto`, and version checking begins on the next scheduler cycle.

## Related Documentation

- [API Reference: Autodiscovery](../api/autodiscovery.md) — endpoint details, request/response
  shapes, and ignore rule management.
- [Software Item Entity](../architecture/software-item-entity.md) — underlying data model and
  database schema.
- [System Overview](system-overview.md) — agent and plugin architecture.
- [Update Workflow](update-workflow.md) — what happens after an item is approved and version
  tracking begins.
