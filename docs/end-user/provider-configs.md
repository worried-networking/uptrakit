# Provider Configurations

A **provider configuration** (provider config) defines a software source that Uptrakit uses to
resolve upstream versions, check installed versions, and optionally discover packages automatically.
Each software item host assignment links to a provider config, so Uptrakit knows which provider
logic to run and which remote package to query.

Provider configs are tenant-scoped, reusable objects. Multiple software items can reference the
same provider config.

## Provider Types

Uptrakit ships with five built-in provider types:

| Provider type | Description | Autodiscovery |
| --- | --- | --- |
| `github_releases` | Tracks releases published on GitHub. Resolves the latest release tag and optionally filters by asset or pre-release status. | No |
| `docker_registry` | Tracks image tags in a Docker/OCI registry. Resolves the latest tag according to a configurable tag pattern. | No |
| `homebrew` | Tracks Homebrew formulae and casks. Installed version is read from the local Homebrew installation on the agent host. | Yes |
| `proxmox_helper_scripts` | Tracks applications managed by Proxmox VE community helper scripts. Installed and latest versions are resolved locally by the agent. | Yes |
| `apt` | Tracks Debian/Ubuntu packages managed by APT. Installed and latest versions are resolved locally by the agent using `dpkg` and `apt-cache`. Requires `sudo` access for updates and index refresh. | Yes |

### `github_releases` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `owner` | Yes | GitHub repository owner (username or organisation) |
| `repo` | Yes | GitHub repository name |
| `asset_pattern` | No | Regex pattern to match a release asset name |
| `include_prereleases` | No | Include pre-release tags when resolving latest (default: `false`) |

### `docker_registry` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `image` | Yes | Image name, e.g. `nginx` or `ghcr.io/example/myapp` |
| `registry` | No | Registry URL override (defaults to Docker Hub) |
| `tag_pattern` | No | Regex pattern to filter tags when resolving latest |

### `homebrew` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `package_type` | Yes | Either `formula` or `cask` |
| `formula` | No | Homebrew formula name (required when `package_type` is `formula`) |
| `cask` | No | Homebrew cask token (required when `package_type` is `cask`) |

### `proxmox_helper_scripts` configuration fields

The `proxmox_helper_scripts` provider requires no explicit configuration fields. Uptrakit
auto-creates a config named `"Proxmox Helper Scripts"` when the first supporting agent
connects.

### `apt` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | `manual` (default) or `all`. Controls which packages are surfaced during autodiscovery. `manual` surfaces only packages explicitly installed by the user (`apt-mark showmanual`); `all` surfaces every installed package. |

Uptrakit auto-creates a config named `"APT"` when the first agent with APT support
connects and no matching provider config exists.

For full details and `sudoers` configuration requirements, see
[APT Provider](providers/apt.md).

## Managing Provider Configs

### Web UI

Navigate to **Provider Configs** in the main navigation. The page lists all provider configs for
your account with their type, name, and assigned software items count.

**Creating a provider config:**

1. Click **New Provider Config**.
2. Select the provider type.
3. Enter a name and fill in the type-specific fields.
4. Click **Save**.

**Editing a provider config:**

Open the context menu (three-dot button) on any provider config row and select **Edit**. You can
update the name and any configuration fields. Changes take effect on the next version check cycle.

**Deleting a provider config:**

Open the context menu and select **Delete**. A provider config cannot be deleted while software
items reference it. Remove or reassign those items first.

### CLI

```bash
# List all provider configs
uptrakit provider-configs list

# Show details for a specific provider config
uptrakit provider-configs show <PROVIDER_CONFIG_ID>

# Create a GitHub Releases provider config
uptrakit provider-configs create \
  --name "my-app GitHub Releases" \
  --type github_releases \
  --config '{"owner":"example","repo":"my-app"}'

# Create a Docker Registry provider config
uptrakit provider-configs create \
  --name "my-image Docker" \
  --type docker_registry \
  --config '{"image":"example/my-image","tag_pattern":"^[0-9]+\\.[0-9]+\\.[0-9]+$"}'

# Create a Homebrew formula provider config
uptrakit provider-configs create \
  --name "git Homebrew" \
  --type homebrew \
  --config '{"package_type":"formula","formula":"git"}'

# Update a provider config's name
uptrakit provider-configs update <PROVIDER_CONFIG_ID> --name "New Name"

# Update configuration fields
uptrakit provider-configs update <PROVIDER_CONFIG_ID> \
  --config '{"owner":"example","repo":"updated-repo"}'

# Delete a provider config
uptrakit provider-configs delete <PROVIDER_CONFIG_ID>
```

## Autodiscovery

The `homebrew`, `proxmox_helper_scripts`, and `apt` provider types support
**autodiscovery**: the agent queries the local package manager and reports installed
packages back to the controller, which creates pending software items for your review.

If no provider config exists for a discovery-capable type when a host registers, Uptrakit
automatically creates one. Auto-created configs are named:

- `Homebrew (Formulae)`
- `Homebrew (Casks)`
- `Proxmox Helper Scripts`
- `APT`

### Triggering discovery

Discovery runs automatically when an agent registers a new host. You can also trigger it on demand.

**Web UI** — Go to **Provider Configs**, open the context menu on a discovery-capable config, and
select **Trigger Discovery**. Alternatively, go to **Hosts**, open the host context menu, and
select **Trigger Discovery** to run all discovery-capable providers for that host.

**CLI:**

```bash
# Trigger discovery for a specific provider config (all connected agents)
uptrakit provider-configs discover <PROVIDER_CONFIG_ID>

# Trigger discovery for a specific host (all discovery-capable providers)
uptrakit hosts discover <HOST_ID>
```

### Discarding discovered items

After discovery, pending items appear in the **Software → Pending** tab for your review. If you
want to clear all pending items at once without reviewing them individually:

**Web UI** — Go to **Provider Configs**, open the context menu, and select **Discard Discovered**.

**CLI:**

```bash
# Discard all pending discovered items for a provider config
uptrakit provider-configs discard-discovered <PROVIDER_CONFIG_ID>

# Discard all pending discovered items for a specific host
uptrakit hosts discard-discovered <HOST_ID>

# Discard pending items for a specific provider config on a specific host
uptrakit hosts discard-discovered <HOST_ID> --provider-config <PROVIDER_CONFIG_ID>
```

Discard performs a soft-delete with no ignore rules created. Discarded packages can be
re-discovered on the next discovery run.

## Autodiscovery Ignore Rules

An ignore rule permanently suppresses a specific package from appearing in future discovery
results. Ignore rules are keyed on a `(provider_config, package_identifier)` pair and apply
across all hosts.

### Managing ignore rules in the Web UI

Navigate to **Provider Configs**, then select a provider config to view its ignore rules. From
there you can:

- View all ignore rules for the provider config.
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
  --provider-config <PROVIDER_CONFIG_ID> \
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
- [CLI Usage Guide](cli-usage.md) — all `provider-configs` and `autodiscovery` commands.
- [Software Item Entity](../architecture/software-item-entity.md) — data model and provider
  config relationships.
- [API Reference: Autodiscovery](../api/autodiscovery.md) — REST endpoint details for discovery
  and ignore rules.
