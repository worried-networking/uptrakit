# Plugin Configurations

A **plugin configuration** (plugin config) defines a software source that Uptrakit uses to
resolve upstream versions, check installed versions, and optionally discover packages automatically.
Each software item host assignment has up to three **role-based plugin assignments**
(`detect_version`, `fetch_releases`, `execute_update`), each linking to a plugin config.
This tells Uptrakit which plugin logic to run for each concern and which remote package to query.

Plugin configs are tenant-scoped, reusable objects. Multiple software items can reference the
same plugin config.

## Plugin Types

Uptrakit ships with five built-in plugin types:

| Plugin type | Description | Autodiscovery |
| --- | --- | --- |
| `github_releases` | Tracks releases published on GitHub. Resolves the latest release tag and optionally filters by asset or pre-release status. | No |
| `docker` | Tracks image tags or SHA digests in a Docker/OCI registry. Can pull images via the local or remote Docker daemon, and discovers running/stopped containers. | Yes |
| `homebrew` | Tracks Homebrew formulae and casks. Installed version is read from the local Homebrew installation on the agent host. | Yes |
| `proxmox_helper_scripts` | Discovery-only. Scans the container's update script, fetches each CT script, and synthesizes downstream `github_releases` or `apt` plugin configs automatically. Does not perform version detection or updates directly. | Yes |
| `apt` | Tracks Debian/Ubuntu packages managed by APT. Installed and latest versions are resolved locally by the agent using `dpkg` and `apt-cache`. Requires `sudo` access for updates and index refresh. | Yes |

### `github_releases` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `owner` | Yes | GitHub repository owner (username or organisation) |
| `repo` | Yes | GitHub repository name |
| `asset_pattern` | No | Regex pattern to match a release asset name |
| `include_prereleases` | No | Include pre-release tags when resolving latest (default: `false`) |
| `detect_installed_version_command` | No | Shell command run on the agent to detect the installed version. The first non-empty line of stdout is used. Supports `{package_identifier}` placeholder. Auto-set on PHS-synthesized configs. |
| `install_command` | No | Shell command to execute after an update. Supports `{version}`, `{tag}`, `{asset_url}`, `{asset_name}` placeholders. Auto-set on PHS-synthesized configs. |

### `docker` configuration fields

The `docker` plugin requires no mandatory fields — an empty config `{}` is valid. For the full
field reference, see [Docker Plugin](plugins/docker.md).

| Field | Required | Description |
| --- | --- | --- |
| `tracking_mode` | No | `"semver_tags"` (default) or `"digest_tracking"` |
| `tag_patterns` | No | Regex patterns to filter tags in semver mode |
| `tracked_tag` | No | Tag to track in digest mode (default: `"latest"`) |
| `auth` | No | Registry credentials (`basic` or `bearer`) |
| `docker_host` | No | Docker daemon endpoint override |
| `compose_restart` | No | Run `docker compose up -d` after pulling |
| `post_pull_command` | No | Custom shell command after pulling |

### `homebrew` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `package_type` | Yes | Either `formula` or `cask` |
| `formula` | No | Homebrew formula name (required when `package_type` is `formula`) |
| `cask` | No | Homebrew cask token (required when `package_type` is `cask`) |

### `proxmox_helper_scripts` configuration fields

The `proxmox_helper_scripts` plugin requires no configuration fields — its config is always an
empty object `{}`. Uptrakit auto-creates a config named `"Proxmox Helper Scripts"` when the first
supporting agent connects.

**Important:** The PHS plugin is discovery-only. It does not track installed or upstream
versions itself, and it does not execute updates. Instead, when a PHS container is discovered,
the controller automatically creates one of the following plugin configs for version tracking
and update execution:

- A `github_releases` config for each `(owner, repo)` pair found in the CT script — pre-configured
  with the installed-version detection command and the unattended update command.
- A shared `APT (auto)` config for containers whose scripts install software via APT (e.g.
  Grafana, Plex).

You may rename or adjust these synthesized configs as needed. Re-running discovery will reuse
existing configs if they already exist.

### `apt` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | `manual` (default) or `all`. Controls which packages are surfaced during autodiscovery. `manual` surfaces only packages explicitly installed by the user (`apt-mark showmanual`); `all` surfaces every installed package. |

Uptrakit auto-creates a config named `"APT"` when the first agent with APT support
connects and no matching plugin config exists.

For full details and `sudoers` configuration requirements, see
[APT Plugin](plugins/apt.md).

## Role-Based Host Assignments

When a software item is assigned to a host, Uptrakit creates up to three **plugin assignments** —
one per plugin role:

| Role | What it does |
| --- | --- |
| `detect_version` | Detects the currently installed version on the host. |
| `fetch_releases` | Fetches the latest available version from an upstream source. |
| `execute_update` | Executes the actual software update on the host. |

By default, all three roles use the same plugin config. However, you can **mix and match** plugins
per role. For example, you could use an APT plugin for detecting the installed version and executing
updates, but a GitHub Releases plugin for fetching upstream releases.

Each plugin assignment carries its own `package_identifier`, `config_override`, and
`execution_site`, giving you fine-grained control over how each role operates on each host.

### Execution site

The `execution_site` field controls where a plugin role's operation runs:

| Value | Behaviour |
| --- | --- |
| `auto` | **Default.** The system decides automatically. For `fetch_releases`, plugins that support controller-side execution (GitHub Releases, Docker) run on the controller; plugins that need a local package index (Homebrew, APT) run on the agent. For `detect_version` and `execute_update`, the agent always runs them. |
| `agent` | Force the operation to run on the agent, even if the plugin supports controller-side execution. Useful when the upstream source is only reachable from the agent's network. |
| `controller` | Force the operation to run on the controller. Only valid for `fetch_releases`. Use this when you want the controller to fetch releases centrally, avoiding redundant API calls across multiple agents. |

In most cases, `auto` is the right choice. The system automatically deduplicates controller-side
`fetch_releases` calls: when multiple hosts share the same `(plugin_config, package_identifier)`
combination, the controller fetches once and propagates the result to all of them.

### Latest version tracking

Latest version information is tracked per-host in the host assignment (`host_software_items` table),
not globally. This means different hosts can report different latest versions if their
`fetch_releases` plugins or execution sites differ.

## Managing Plugin Configs

### Web UI

Navigate to **Plugin Configs** in the main navigation. The page lists all plugin configs for
your account with their type, name, and assigned software items count.

**Creating a plugin config:**

1. Click **New Plugin Config**.
2. Select the plugin type.
3. Enter a name and fill in the type-specific fields.
4. Click **Save**.

**Editing a plugin config:**

Open the context menu (three-dot button) on any plugin config row and select **Edit**. You can
update the name and any configuration fields. Changes take effect on the next version check cycle.

**Deleting a plugin config:**

Open the context menu and select **Delete**. A plugin config cannot be deleted while software
items reference it. Remove or reassign those items first.

### CLI

```bash
# List all plugin configs
uptrakit plugin-configs list

# Show details for a specific plugin config
uptrakit plugin-configs show <PLUGIN_CONFIG_ID>

# Create a GitHub Releases plugin config
uptrakit plugin-configs create \
  --name "my-app GitHub Releases" \
  --type github_releases \
  --config '{"owner":"example","repo":"my-app"}'

# Create a Docker plugin config (semver tags)
uptrakit plugin-configs create \
  --name "my-image Docker" \
  --type docker \
  --config '{"tracking_mode":"semver_tags","tag_patterns":["^[0-9]+\\.[0-9]+\\.[0-9]+$"]}'

# Create a Homebrew formula plugin config
uptrakit plugin-configs create \
  --name "git Homebrew" \
  --type homebrew \
  --config '{"package_type":"formula","formula":"git"}'

# Update a plugin config's name
uptrakit plugin-configs update <PLUGIN_CONFIG_ID> --name "New Name"

# Update configuration fields
uptrakit plugin-configs update <PLUGIN_CONFIG_ID> \
  --config '{"owner":"example","repo":"updated-repo"}'

# Delete a plugin config
uptrakit plugin-configs delete <PLUGIN_CONFIG_ID>
```

## Autodiscovery

The `docker`, `homebrew`, `proxmox_helper_scripts`, and `apt` plugin types support
**autodiscovery**: the agent queries the local runtime (Docker daemon or package manager) and reports installed
packages back to the controller, which creates pending software items for your review.

If no plugin config exists for a discovery-capable type when a host registers, Uptrakit
automatically creates one. Auto-created configs are named:

- `Docker`
- `Homebrew (Formulae)`
- `Homebrew (Casks)`
- `Proxmox Helper Scripts`
- `APT`

**PHS auto-created configs:** In addition to the `"Proxmox Helper Scripts"` config used as a
discovery anchor, the PHS plugin triggers creation of downstream `github_releases` and
`APT (auto)` configs during discovery (see [PHS configuration](#proxmox_helper_scripts-configuration-fields)
for details). These synthesized configs are what appear as parent configs on your approved PHS software items.

### Triggering discovery

Discovery runs automatically when an agent registers a new host. You can also trigger it on demand.

**Web UI** — Go to **Plugin Configs**, open the context menu on a discovery-capable config, and
select **Trigger Discovery**. Alternatively, go to **Hosts**, open the host context menu, and
select **Trigger Discovery** to run all discovery-capable plugins for that host.

**CLI:**

```bash
# Trigger discovery for a specific plugin config (all connected agents)
uptrakit plugin-configs discover <PLUGIN_CONFIG_ID>

# Trigger discovery for a specific host (all discovery-capable plugins)
uptrakit hosts discover <HOST_ID>
```

### Discarding discovered items

After discovery, pending items appear in the **Software → Pending** tab for your review. If you
want to clear all pending items at once without reviewing them individually:

**Web UI** — Go to **Plugin Configs**, open the context menu, and select **Discard Discovered**.

**CLI:**

```bash
# Discard all pending discovered items for a plugin config
uptrakit plugin-configs discard-discovered <PLUGIN_CONFIG_ID>

# Discard all pending discovered items for a specific host
uptrakit hosts discard-discovered <HOST_ID>

# Discard pending items for a specific plugin config on a specific host
uptrakit hosts discard-discovered <HOST_ID> --plugin-config <PLUGIN_CONFIG_ID>
```

Discard performs a soft-delete with no ignore rules created. Discarded packages can be
re-discovered on the next discovery run.

## Autodiscovery Ignore Rules

An ignore rule permanently suppresses a specific package from appearing in future discovery
results. Ignore rules are keyed on a `(plugin_config, package_identifier)` pair and apply
across all hosts.

### Managing ignore rules in the Web UI

Navigate to **Plugin Configs**, then select a plugin config to view its ignore rules. From
there you can:

- View all ignore rules for the plugin config.
- Add a new ignore rule by entering a package identifier.
- Delete an existing ignore rule to re-enable future discovery of that package.

You can also create an ignore rule implicitly by using **Delete & Ignore** in the context menu
on a software item host assignment (on the **Software** page).

### Managing ignore rules via the CLI

```bash
# List all ignore rules
uptrakit autodiscovery ignores list

# Show a specific ignore rule
uptrakit autodiscovery ignores show <IGNORE_ID>

# Create an ignore rule to pre-suppress a package
uptrakit autodiscovery ignores create \
  --plugin-config <PLUGIN_CONFIG_ID> \
  --package "unwanted-package"

# Delete an ignore rule (re-enables future discovery)
uptrakit autodiscovery ignores delete <IGNORE_ID>
```

You can also create an ignore rule when unassigning a host from a software item:

```bash
uptrakit software-items unassign <ITEM_ID> --host <HOST_ID> --ignore
```

## Related Documentation

- [Autodiscovery](autodiscovery.md) — full discovery workflow, review process, and ignore list
  concepts.
- [CLI Usage Guide](cli-usage.md) — all `plugin-configs` and `autodiscovery` commands.
- [Software Item Entity](../architecture/software-item-entity.md) — data model and plugin
  config relationships.
- [API Reference: Autodiscovery](../api/autodiscovery.md) — REST endpoint details for discovery
  and ignore rules.
