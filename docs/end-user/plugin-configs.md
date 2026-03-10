# Plugin Configurations

A **plugin configuration** (plugin config) defines a software source that Uptrakit uses to
resolve upstream versions, check installed versions, and optionally discover packages automatically.
Each software item host assignment has up to three **role-based plugin assignments**
(`detect_version`, `fetch_releases`, `execute_update`), each linking to a plugin config.
This tells Uptrakit which plugin logic to run for each concern and which remote package to query.

Plugin configs are tenant-scoped, reusable objects. Multiple software items can reference the
same plugin config.

## Plugin Types

Uptrakit ships with fifteen built-in plugin types:

| Plugin type | Description | Autodiscovery |
| --- | --- | --- |
| `releases_github` | Fetches releases published on GitHub. Controller-side only — does not detect installed versions or execute updates directly. | No |
| `releases_gitlab` | Fetches releases published on GitLab (or a self-hosted GitLab instance). Controller-side only. Supports nested namespaces (`group/subgroup/project`). | No |
| `releases_forgejo` | Fetches releases from any Forgejo or Gitea instance (Codeberg, self-hosted, etc.). Controller-side only. Auto-detected by the PHS discovery plugin for Codeberg-hosted apps. | No |
| `generic_shell` | Generic agent-side plugin: detects the installed version via a configurable shell command and/or executes updates via a configurable shell command. | No |
| `releases_docker` | Tracks image tags or SHA digests in a Docker/OCI registry. Can pull images via the local or remote Docker daemon, and discovers running/stopped containers. | Yes |
| `package_manager_homebrew` | Tracks Homebrew formulae and casks. Installed version is read from the local Homebrew installation on the agent host. | Yes |
| `discovery_proxmox_helper_scripts` | Discovery-only. Scans the container's update script, fetches each CT script, and synthesizes downstream plugin configs automatically. Does not perform version detection or updates directly. | Yes |
| `package_manager_apt` | Tracks Debian/Ubuntu packages managed by APT. Installed and latest versions are resolved locally by the agent using `dpkg` and `apt-cache`. Requires `sudo` access for updates and index refresh. | Yes |
| `package_manager_dnf` | Tracks RPM packages managed by DNF on Fedora/RHEL/Rocky/AlmaLinux and other RPM-based distributions. Installed and latest versions are resolved locally by the agent using `rpm` and `dnf`. Requires `sudo` access for updates and index refresh. | Yes |
| `package_manager_npm` | Tracks globally installed npm packages. Fetches upstream versions from the npm registry (controller-side). Discovers globally installed packages and executes updates via `npm install -g`. Requires `sudo` access for updates. | Yes |
| `package_manager_mas` | Tracks Mac App Store apps via the `mas` CLI tool. Agent-side only. Discovers installed apps and checks for updates via `mas list` and `mas outdated`. No `sudo` required. Requires `brew install mas`. | Yes |
| `package_manager_pacman` | Tracks Arch Linux packages managed by Pacman. Installed versions are resolved locally by the agent using `pacman -Q`; latest versions are fetched from repositories using `pacman -Si`. Requires `sudo` access for updates and database sync. | Yes |
| `package_manager_pkg` | Tracks packages managed by the BSD `pkg` tool (FreeBSD, TrueNAS SCALE, OPNsense, pfSense, DragonFly BSD). Installed and available versions are resolved locally by the agent. Requires `sudo` access for updates and index refresh. | Yes |
| `package_manager_apk` | Tracks Alpine Linux packages managed by APK. Installed and latest versions are resolved locally by the agent using `apk`. Requires `sudo` access for updates and index refresh. | Yes |
| `package_manager_snap` | Tracks Snap packages managed by `snapd` on Linux. Agent-side only. Discovers installed snaps, resolves upstream versions via `snap info`, and executes updates via `snap refresh`. Requires `sudo` access for updates. | Yes |

### `releases_github` configuration fields

The GitHub Releases plugin fetches upstream release metadata via the GitHub REST API
(controller-side) and can download release assets directly to managed hosts (agent-side).
The `owner` and `repo` are **not** configuration fields — they are expressed as the
`package_identifier` of the software item (format: `"owner/repo"`). A single
`releases_github` config can therefore serve any number of tracked GitHub repositories.

| Field | Required | Description |
| --- | --- | --- |
| `auth_token` | No | Personal access token for authentication. Increases API rate limits. Stored encrypted at rest. |
| `api_base_url` | No | Custom API base URL for GitHub Enterprise (must use HTTPS). Defaults to `https://api.github.com`. |
| `include_prereleases` | No | Include pre-release tags when resolving latest (default: `false`). |
| `tag_strip_prefix` | No | Prefix to strip from tag names when extracting version strings (default: `"v"`). |
| `asset_patterns` | No | List of regex patterns to filter release assets. Only assets whose names match at least one pattern are included. An empty list includes all assets. Used by both `fetch_releases` (controller) and `execute_update` (agent). |
| `install_path` | No | Absolute destination path for the downloaded asset (e.g. `/usr/local/bin/pocket-id`). Required for `execute_update`. Must not contain `..` or null bytes. Max 4096 characters. |
| `make_executable` | No | Set the executable bit on the installed file (default: `true`). When `false`, the file is installed with mode `0644`. |
| `pre_install_command` | No | Shell command to run before installing the asset (e.g. `sudo systemctl stop myapp`). Supports `{version}` and `{tag}` placeholders. |
| `post_install_command` | No | Shell command to run after installing the asset (e.g. `sudo systemctl start myapp`). Supports `{version}` and `{tag}` placeholders. |
| `verify_attestation` | No | Download the release checksums file and query the [GitHub Attestations API](https://docs.github.com/en/rest/repos/repos#list-attestations) for each release (default: `true`). Attestation status is stored in `latest_release_metadata` and shown in the UI. Set to `false` to disable entirely (not recommended for production use of public repositories). |
| `require_attestation` | No | Abort the update on the agent if no GitHub Actions attestation is found for the release (default: `false`). When `true`, the agent independently re-verifies the attestation before install and blocks any release with `attestation_status = NotFound`. See [GitHub Actions Attestation Verification](../security/github-attestation.md) for details. |

**Package identifier:** The software item's `package_identifier` for a GitHub-tracked package
must be set to `"owner/repo"` (e.g. `"octocat/hello-world"`). This value is validated when
a software item is saved.

**Asset download workflow:** When `install_path` is set, `execute_update` performs:
pre-install command (if set) → download asset to temp file → verify SHA-256 checksum →
install to target path via `sudo install` → post-install command (if set). The `asset_patterns`
field selects which asset to download — exactly one must match per host (use per-host
`config_override` on the `execute_update` role assignment to narrow patterns for each
OS/architecture).

**Sudoers entries:** When `install_path` is configured, the plugin declares `install` as a
required sudo command. When `pre_install_command` or `post_install_command` reference
`systemctl`, the plugin additionally declares `systemctl stop *` and `systemctl start *` as
restricted sudo entries (other systemctl subcommands are not permitted).

### `releases_gitlab` configuration fields

The GitLab Releases plugin is **controller-side only**. It fetches upstream release metadata via
the GitLab Projects API. The project path is **not** a configuration field — it is expressed as
the `package_identifier` of the software item. A single `releases_gitlab` config can therefore
serve any number of tracked GitLab projects, including projects in nested namespaces.

| Field | Required | Description |
| --- | --- | --- |
| `auth_token` | No | Personal access token (PAT) for authentication. Must have at least `read_api` scope. Stored encrypted at rest. |
| `api_base_url` | No | Custom API base URL for self-hosted GitLab instances (must use HTTPS). Defaults to `https://gitlab.com`. |
| `include_prereleases` | No | Include upcoming (unreleased/embargoed) releases when resolving latest (default: `false`). GitLab marks these with `upcoming_release: true`. |
| `tag_strip_prefix` | No | Prefix to strip from tag names when extracting version strings (default: `"v"`). |
| `asset_patterns` | No | List of regex patterns to filter release asset links. Only manually-uploaded links whose names match at least one pattern are included. Auto-generated source archives are always excluded. An empty list includes all manually-uploaded links. |

**Package identifier:** Must be the GitLab project path in the form `"namespace/project"` or
`"group/subgroup/project"` for nested namespaces (e.g. `"myorg/myapp"` or
`"platform/backend/api-gateway"`). All path components must be non-empty and may not contain
`..`. The identifier is validated when a software item is saved.

**Self-hosted GitLab:** Set `api_base_url` to your instance root (e.g.
`https://gitlab.corp.com`). The plugin appends `/api/v4/projects/...` automatically.

### `releases_forgejo` configuration fields

The Forgejo Releases plugin is **controller-side only**. It fetches upstream release metadata via
the Forgejo/Gitea API. The `owner` and `repo` are **not** configuration fields — they are expressed
as the `package_identifier` of the software item (format: `"owner/repo"`). A single
`releases_forgejo` config can therefore serve any number of tracked repositories on the same instance.

| Field | Required | Description |
| --- | --- | --- |
| `api_base_url` | **Yes** | Root URL of the Forgejo/Gitea instance (e.g. `https://codeberg.org`). Must use HTTPS and must not point to a private/loopback host. |
| `auth_token` | No | Personal access token for authentication. Stored encrypted at rest. |
| `include_prereleases` | No | Include pre-release tags when resolving latest (default: `false`). |
| `tag_strip_prefix` | No | Prefix to strip from tag names when extracting version strings (default: `"v"`). |
| `asset_patterns` | No | List of regex patterns to filter release assets. Only assets whose names match at least one pattern are included. An empty list includes all assets. |

**Package identifier:** Must be set to `"owner/repo"` (e.g. `"readeck/readeck"`). This value is
validated when a software item is saved.

**PHS auto-discovery:** Proxmox Helper Scripts containers managed via `check_for_codeberg_release`
or `CODEBERG_REPO=` are automatically detected by the `discovery_proxmox_helper_scripts` plugin,
which synthesizes a `releases_forgejo` target (with `api_base_url` set to `https://codeberg.org`)
and the corresponding Shell target automatically.

### `generic_shell` configuration fields

The Shell plugin provides generic agent-side operations using user-supplied shell commands.
Both fields are independently optional, but at least one must be set.

| Field | Required | Description |
| --- | --- | --- |
| `version_command` | Conditionally | Shell command run on the agent to detect the installed version. The first non-empty trimmed line of stdout is used as the version string. If absent, `detect_version` is not supported. Supports `{package_identifier}` placeholder (shell-escaped). |
| `update_command` | Conditionally | Shell command run on the agent to execute an update. If absent, `execute_update` is not supported. Supports `{version}`, `{tag}`, and `{package_identifier}` placeholders (all shell-escaped). `{tag}` falls back to `{version}` when no release metadata is available. |

**Placeholder reference:**

| Placeholder | Replaced with |
| --- | --- |
| `{package_identifier}` | The software item's package identifier (shell-escaped). |
| `{version}` | The target version string (shell-escaped). |
| `{tag}` | The release tag (e.g. `v1.2.3`). Falls back to `{version}` when no release info is available. |

### `releases_docker` configuration fields

The `releases_docker` plugin requires no mandatory fields — an empty config `{}` is valid. For the full
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

### `package_manager_homebrew` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `package_type` | Yes | Either `formula` or `cask` |
| `formula` | No | Homebrew formula name (required when `package_type` is `formula`) |
| `cask` | No | Homebrew cask token (required when `package_type` is `cask`) |

### `discovery_proxmox_helper_scripts` configuration fields

The `discovery_proxmox_helper_scripts` plugin requires no configuration fields — its config is always an
empty object `{}`. Uptrakit auto-creates a config named `"Proxmox Helper Scripts"` when the first
supporting agent connects.

**Important:** The PHS plugin is discovery-only. It does not track installed or upstream
versions itself, and it does not execute updates. Instead, when a PHS container is discovered,
the plugin emits structured `DiscoveryTarget` values that the controller processes to create
plugin configs automatically:

- For **GitHub-managed containers** (CT script sources a GitHub release), two configs are created:
  - A `releases_github` config for `FetchReleases` — keyed on the GitHub settings only
    (no `owner`/`repo` in the config); the `owner/repo` is expressed as the software item's
    `package_identifier`.
  - A `generic_shell` config for `DetectVersion` and `ExecuteUpdate` — pre-configured with the
    PHS-standard version-file read command and the unattended update command.
- For **npm-managed containers** (CT script runs `npm install -g <pkg>`), a shared `NPM (auto)` config
  is created for all three roles. The npm package name is carried as the software item's
  `package_identifier`.
- For **APT-managed containers**, a shared `APT (auto)` config is created for all three roles.

You may rename or adjust these synthesized configs as needed. Re-running discovery will reuse
existing configs if they already exist.

### `package_manager_apt` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | `manual` (default) or `all`. Controls which packages are surfaced during autodiscovery. `manual` surfaces only packages explicitly installed by the user (`apt-mark showmanual`); `all` surfaces every installed package. |

Uptrakit auto-creates a config named `"APT"` when the first agent with APT support
connects and no matching plugin config exists.

For full details and `sudoers` configuration requirements, see
[APT Plugin](plugins/apt.md).

### `package_manager_dnf` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | `all` (default) or `user_installed`. Controls which packages are surfaced during autodiscovery. `all` surfaces every installed RPM package; `user_installed` surfaces only packages explicitly installed by the user (`dnf repoquery --userinstalled`). |

Uptrakit auto-creates a config named `"DNF"` when the first agent with DNF support
connects and no matching plugin config exists.

For full details and `sudoers` configuration requirements, see
[DNF Plugin](plugins/dnf.md).

### `package_manager_npm` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `include_prereleases` | No | Include pre-release dist-tags (`next`, `beta`, `alpha`, `rc`, `canary`) when fetching upstream releases (default: `false`). |

The npm plugin supports both **plain packages** (e.g. `n8n`, `pm2`) and **scoped packages**
(e.g. `@angular/cli`). The `package_identifier` must follow npm naming conventions
(see [npm Plugin](npm-plugin.md) for the full validation rules).

**Package identifier format:**

- Plain: `n8n`, `pm2`, `typescript`
- Scoped: `@angular/cli`, `@nestjs/cli`

Uptrakit auto-creates a config named `"npm"` when the first agent with npm support connects
and no matching plugin config exists.

Upstream release versions are fetched from the npm registry by the controller (no local
package index needed). Updates are executed on the agent via `npm install -g <pkg>@<version>`
and require `sudo` access. See [npm Plugin](npm-plugin.md) for details.

### `package_manager_mas` configuration fields

The `mas` plugin has no configuration fields. All Mac App Store apps use the same config (`{}`).

| Field | Required | Description |
| --- | --- | --- |
| _(none)_ | — | No configuration options. |

**Package identifier format:** The numeric App Store ID (e.g. `497799835` for Xcode).
This is the integer value shown in every App Store URL after `/id`.

- `497799835` (Xcode)
- `1147396723` (WhatsApp)
- `408981434` (iMovie)

**Prerequisites:** Install `mas` on the macOS host before running the agent:

```bash
brew install mas
```

**Discovery behaviour:** When `mas` discovers installed App Store apps, it emits one
`DiscoveryTarget` per app pointing to a shared plugin config named `"Mac App Store"` with an
empty config (`{}`). Uptrakit creates this config once and reuses it across all discovered apps —
no per-app config object is needed.

Installed and upstream versions are resolved entirely on the agent via `mas list` and
`mas outdated`. No controller-side network access is required. Updates run via `mas upgrade <id>`
and do **not** require `sudo`.

### `package_manager_pacman` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | `all` (default) or `explicit`. Controls which packages are surfaced during autodiscovery. `all` surfaces every installed package (`pacman -Q`); `explicit` surfaces only packages explicitly installed by the user (`pacman -Qe`). |

Uptrakit auto-creates a config named `"Pacman"` when the first agent with Pacman support
connects and no matching plugin config exists.

**Package identifier format:** The lowercase Arch Linux package name
(e.g., `nginx`, `git`, `python`, `lib32-glibc`). Valid characters are
`[a-z0-9@._+-]`; must start with `[a-z0-9]`; maximum 128 characters.

Examples:

- `nginx`
- `python`
- `lib32-glibc`
- `python3.11`

**Version format:** Pacman versions use the format `<version>-<pkgrel>` (e.g., `1.26.3-1`).
Epoch-prefixed versions (`2:1.0-1`) are also supported. The full version string including the
package release (`-1`) is stored for tracking.

> **Note:** Pacman does not support installing a pinned version from its standard repositories.
> The update command always installs the latest available version from the configured
> repositories. The `to_version` field is validated but only the package name is passed to
> `pacman -S --noconfirm`.

**Sudo requirements:** The `pacman` binary requires root privileges for database sync and
package installation. Add the following to your `sudoers` configuration on each agent host:

```sudoers
agent_user ALL=(ALL) NOPASSWD: /usr/bin/pacman
```

Unlike APT, Pacman does not require `SETENV` in sudoers (no environment variables need to be
preserved).

For full details and `sudoers` configuration requirements, see
[Pacman Plugin](plugins/pacman.md).

### `package_manager_pkg` configuration fields

The BSD `pkg` plugin tracks packages managed by FreeBSD's `pkgng` package manager. It works
on FreeBSD, TrueNAS SCALE, OPNsense, pfSense, and DragonFly BSD.

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | Controls which packages are surfaced during autodiscovery. `"all"` (default) — all installed packages. `"manual"` — only packages the user explicitly installed (auto-install flag `0`). Omitting this field (or sending `{}`) puts the plugin in discover-all mode so the controller can auto-create the config. |

**Package identifier format:** The FreeBSD package name (e.g., `curl`, `nginx`, `python39`,
`php82-extensions`, `p5-Net-SSLeay`).

Required `sudo` commands: `pkg` (for `pkg update` and `pkg install`).

**Capabilities:** version detection, package index refresh, autodiscovery, batch updates.

**Discovery behaviour:** When the config is empty (`{}`), the plugin emits one `DiscoveryTarget`
per installed package pointing to a shared plugin config named `"BSD pkg"`. Uptrakit creates
this config once and reuses it across all discovered packages. When an explicit
`discovery_filter` is set, the config-ID path is used and no targets are emitted.

Installed versions are resolved on the agent via `pkg query "%v" <name>`. Upstream versions
are read from the local repository database via `pkg rquery "%v" <name>` (no additional
network access required beyond the repository index, which is refreshed by `pkg update -q`).
Updates are executed via `pkg install -y <name>` with `sudo`.
### `package_manager_apk` configuration fields

| Field | Required | Description |
| --- | --- | --- |
| `discovery_filter` | No | `all` (default) or `world`. Controls which packages are surfaced during autodiscovery. `all` surfaces every installed package (`apk list --installed`); `world` surfaces only packages explicitly listed in `/etc/apk/world`. |

Uptrakit auto-creates a config named `"APK"` when the first agent with APK support
connects and no matching plugin config exists.

**Package identifier format:** The Alpine package name (e.g. `busybox`, `openssl`,
`ca-certificates`). Must start with a lowercase letter or digit; may contain lowercase
letters, digits, `.`, `_`, `+`, and `-`. Length: 2–100 characters.

**Discovery behaviour:** In default (discover-all) mode, the plugin runs `apk list --installed`
and emits one `DiscoveryTarget` per package pointing to a shared `"APK"` config. With the
`world` filter, only packages in `/etc/apk/world` are surfaced using `apk info -v`.

**Sudoers entries:** The APK plugin declares two restricted `sudo` entries:

- `apk update` — for refreshing the package index
- `apk add *` — for installing packages

See [APK Plugin](plugins/apk.md) for full details.

### `package_manager_snap` configuration fields

The Snap plugin tracks packages installed via `snapd`. Version detection and updates run on the
agent host; no controller-side network access is required.

| Field | Required | Description |
| --- | --- | --- |
| `channel` | No | Snap channel to track (e.g. `latest/stable`, `1.0/stable`, `edge`). Defaults to `latest/stable`. |

**Package identifier format:** The Snap package name (e.g. `vlc`, `code`, `firefox`).
Snap names are lowercase alphanumeric with hyphens, 2–40 characters, and must not start or end
with a hyphen.

**Discovery behaviour:** When the Snap plugin discovers installed packages, it emits one
`DiscoveryTarget` per user-installed snap pointing to a shared plugin config named `"Snap"` with
an empty config (`{}`). Infrastructure and base snaps (`core*`, `snapd`, `bare`) are excluded
from discovery automatically.

**Updates are channel-based.** `snap refresh <name>` tracks the configured channel rather than
installing a specific version string. The target version shown in Uptrakit is informational;
snap automatically installs the latest revision on the channel.

**Requires `sudo`.** Snap refresh requires root. Uptrakit configures passwordless sudo for the
`snap` command via `uptrakit-agent bootstrap`. See [Snap Plugin](snap-plugin.md) for full details.

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

## Managing Per-Host Plugin Assignments

Each host assigned to a software item carries its own set of role-based plugin assignments.
You can view and edit these assignments from the **Software** item detail page.

### Viewing plugin assignments (Web UI)

Navigate to **Software**, then click a software item name to open its detail view. The hosts
table shows each assigned host with a compact summary of its configured roles below the
hostname — for example:

```text
Detect: APT (prometheus)
Fetch:  GitHub Releases (prometheus/prometheus)
Update: APT (prometheus)
```

If a role is configured to run on `agent` or `controller` instead of the default `auto`, a
small site badge appears next to the config name.

### Editing plugin assignments (Web UI)

1. Open the software item detail page (**Software → [item name]**).
2. In the hosts table, click **Configure** on the row for the host you want to edit.
3. The **Configure Plugins** modal opens with three sections — one per role:
   - **Detect Version** — checks the installed version on the host.
   - **Fetch Releases** — queries upstream for the latest available version.
   - **Execute Update** — runs the update command on the host.
4. For each role:
   - Select the **Plugin Config** to use. Set to `— not configured —` to leave a role unconfigured.
   - Enter the **Package ID** (plugin-specific identifier, e.g. `owner/repo` for GitHub Releases or
     `prometheus` for APT).
   - For **Fetch Releases** only: choose the **Execution Site** (`auto`, `agent`, or `controller`).
     See [Execution site](#execution-site) for details.
   - Optionally expand **Config Override (advanced)** to enter a JSON object of fields to merge on
     top of the plugin config's base settings — useful for per-host customisation without creating
     a separate plugin config. Leave empty to clear any existing override.
5. Click **Save Changes**. The change takes effect on the next version check cycle.

> **Removing a role:** The modal can only add or update role assignments. To remove a role entirely,
> unassign the host from the software item (**Software** list → context menu → **Assign to Hosts**,
> deselect the host) and reassign it without the role you want to drop.

### Assigning hosts to a software item (Web UI)

1. Open the **Software** list.
2. Open the context menu (three-dot button) on the software item and select **Assign to Hosts**.
3. Check the hosts to add or uncheck hosts to remove.
4. For newly added hosts, the **Role assignments for new hosts** table appears. Configure each
   enabled role: choose a **Plugin Config**, enter a **Package ID**, and optionally set the
   **Execution Site** for the **Fetch Releases** role.
5. Click **Save**.

> All newly added hosts receive the same role configuration in a single assignment call. If
> different hosts need different plugin configs or package identifiers, assign them in separate
> operations and use the **Configure** button to adjust each host individually afterward.

### Managing plugin assignments via the CLI

```bash
# Assign a host with a full role setup
uptrakit software-items assign <ITEM_ID> \
  --host <HOST_ID> \
  --plugin-config <CONFIG_ID> \
  --package-identifier "owner/repo" \
  --role fetch_releases

# Update the package identifier for a specific role on a host
uptrakit software-items update-assignment <ITEM_ID> \
  --host <HOST_ID> \
  --role detect_version \
  --package-identifier "prometheus"

# Change the execution site for fetch_releases on a host
uptrakit software-items update-assignment <ITEM_ID> \
  --host <HOST_ID> \
  --role fetch_releases \
  --execution-site controller

# Unassign a host from a software item
uptrakit software-items unassign <ITEM_ID> --host <HOST_ID>
```

## Managing Plugin Configs

### Web UI

Navigate to **Settings** in the main navigation, then select the **Plugin Configs** tab. The
tab lists all plugin configs for your account with their type, name, and assigned software items
count.

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

# Create a GitHub Releases plugin config (owner/repo goes in the software item's
# package_identifier, not here; this config covers all your GitHub-tracked repos)
uptrakit plugin-configs create \
  --name "GitHub Releases" \
  --type releases_github \
  --config '{}'

# Create a GitHub Releases config with an auth token (for higher rate limits)
uptrakit plugin-configs create \
  --name "GitHub Releases (authenticated)" \
  --type releases_github \
  --config '{"auth_token":"ghp_yourtoken"}'

# Create a Shell plugin config for version detection and updates
uptrakit plugin-configs create \
  --name "My App Shell" \
  --type generic_shell \
  --config '{"version_command":"my-app --version","update_command":"apt-get install -y my-app={version}"}'

# Create a Docker plugin config (semver tags)
uptrakit plugin-configs create \
  --name "my-image Docker" \
  --type releases_docker \
  --config '{"tracking_mode":"semver_tags","tag_patterns":["^[0-9]+\\.[0-9]+\\.[0-9]+$"]}'

# Create a Homebrew formula plugin config
uptrakit plugin-configs create \
  --name "git Homebrew" \
  --type package_manager_homebrew \
  --config '{"package_type":"formula","formula":"git"}'

# Update a plugin config's name
uptrakit plugin-configs update <PLUGIN_CONFIG_ID> --name "New Name"

# Update configuration fields
uptrakit plugin-configs update <PLUGIN_CONFIG_ID> \
  --config '{"tag_strip_prefix":"release-","include_prereleases":true}'

# Delete a plugin config
uptrakit plugin-configs delete <PLUGIN_CONFIG_ID>
```

## Autodiscovery

The `releases_docker`, `package_manager_homebrew`, `discovery_proxmox_helper_scripts`,
`package_manager_apt`, `package_manager_dnf`, `package_manager_npm`, and
`package_manager_pacman` plugin types support **autodiscovery**: the
agent queries the local runtime (Docker daemon or package manager) and reports installed
packages back to the controller, which creates pending software items for your review.

If no plugin config exists for a discovery-capable type when a host registers, Uptrakit
automatically creates one. Auto-created configs are named:

- `Docker`
- `Homebrew (Formulae)`
- `Homebrew (Casks)`
- `Proxmox Helper Scripts`
- `APT`
- `DNF`
- `npm`
- `Pacman`

**PHS auto-created configs:** In addition to the `"Proxmox Helper Scripts"` config used as a
discovery anchor, the PHS plugin triggers creation of downstream `releases_github`, `generic_shell`, and
`APT (auto)` configs during discovery (see
[PHS configuration](#discovery_proxmox_helper_scripts-configuration-fields) for details). These synthesized
configs are what appear as parent configs on PHS-discovered software items.

### Triggering discovery

Discovery runs automatically when an agent registers a new host. You can also trigger it on demand.

**Web UI** — Go to **Settings → Plugin Configs**, open the context menu on a discovery-capable config, and
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

**Web UI** — Go to **Settings → Plugin Configs**, open the context menu, and select **Discard Discovered**.

## Software Ignore Rules

An ignore rule permanently suppresses a software item by name from appearing in future discovery
results. Ignore rules can be tenant-wide (keyed on name) or host-specific. A single tenant-wide ignore
rule covers all plugin configs and discovery targets for that name, across all hosts.

### Managing ignore rules in the Web UI

Navigate to **Software** in the main navigation, then select the **Ignore Rules** tab. From
there you can:

- View all ignore rules.
- Add a new ignore rule by entering a software item name.
- Delete an existing ignore rule to re-enable future discovery of that name.

You can also create an ignore rule implicitly by using **Delete & Ignore** in the context menu
on a software item host assignment (on the **Software** page).

### Managing ignore rules via the CLI

```bash
# List all ignore rules
uptrakit software-ignores list

# Create a tenant-wide ignore rule by name (pre-suppress before discovery)
uptrakit software-ignores add --name "unwanted-package"

# Delete an ignore rule (re-enables future discovery)
uptrakit software-ignores delete <IGNORE_ID>
```

You can also create an ignore rule when unassigning a host from a software item:

```bash
uptrakit software-items unassign <ITEM_ID> --host <HOST_ID> --ignore
```

## Related Documentation

- [Manual Software Tracking](manual-software-tracking.md) — step-by-step guide for adding
  tracking for software that cannot be autodiscovered (e.g. standalone binaries from GitHub
  releases). Covers the three-role model, common patterns, and sudoers requirements.
- [Autodiscovery](autodiscovery.md) — full discovery workflow, review process, and ignore list
  concepts.
- [CLI Usage Guide](cli-usage.md) — all `plugin-configs` and `autodiscovery` commands.
- [Software Item Entity](../architecture/software-item-entity.md) — data model and plugin
  config relationships.
- [API Reference: Autodiscovery](../api/autodiscovery.md) — REST endpoint details for discovery
  and ignore rules.
