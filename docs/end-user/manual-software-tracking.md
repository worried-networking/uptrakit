---
title: Manual Software Tracking
weight: 40
description: Uptrakit can track and update software not automatically discoverable by any built-in plugin, covering binaries deployed outside a package manager.
---

# Manual Software Tracking

Uptrakit can track and update software that is **not** automatically discoverable by any built-in
plugin. This covers binaries deployed outside a package manager — for example, a self-hosted web
app installed as a single binary directly from a GitHub release, or a custom-compiled daemon
with a proprietary versioning scheme.

This guide walks through setting up tracking manually, using
[Pocket ID](https://github.com/pocket-id/pocket-id) as a running example throughout.

## When to Use Manual Tracking

Use manual tracking when the software:

- Is not installed via APT, Homebrew, npm, or another supported package manager.
- Does not run as a Docker container (or you prefer not to use Docker-based tracking).
- Is not managed by a Proxmox Helper Script.
- Has a stable way to check the installed version (e.g. a `--version` flag or a version file).

If any autodiscovery plugin already surfaces the software as a pending item, approve it from
the **Software → Pending** tab instead of setting up manual tracking — the plugin will have
already configured the right role assignments.

## Understanding Plugin Roles

Every software item host assignment has up to five **plugin roles**. Each role can use a
different plugin:

| Role | What it does |
| --- | --- |
| `detect_version` | Runs on the agent to detect the currently installed version. |
| `fetch_releases` | Fetches the latest available version from an upstream source. |
| `execute_update` | Runs the actual update command on the agent. |
| `pre_update_hook` | Runs before the update (e.g. stop a service). |
| `post_update_hook` | Runs after the update (e.g. restart a service). |

The three core roles are optional individually, but at least `detect_version` should be
configured to give Uptrakit something to report. Omitting `fetch_releases` means Uptrakit
cannot determine whether an update is available. Omitting `execute_update` means Uptrakit can
detect version drift but you must update the software manually outside Uptrakit.

The hook roles (`pre_update_hook`, `post_update_hook`) are optional and only accept hook-type
plugin configs (`hook_systemd`, `hook_shell`). See [Plugin Configurations](plugin-configs.md)
for details on hook plugin types.

See [Plugin Configurations](plugin-configs.md) for the full reference on plugin types and their
configuration fields.

## Common Patterns

### Pattern A: GitHub-released binary (recommended for most self-hosted apps)

Use this pattern when the software publishes versioned binaries as GitHub release assets and
exposes a `--version`-style command.

| Role | Plugin type | What it does |
| --- | --- | --- |
| `detect_version` | `generic_shell` | Runs a shell command on the agent to read the installed version. |
| `fetch_releases` | `releases_github` | Fetches release tags from the GitHub API (controller-side). |
| `execute_update` | `releases_github` | Downloads a release asset to the agent and installs it. |

#### Example: Pocket ID

Pocket ID is a single binary installed at `/opt/pocket-id/pocket-id`, managed by a systemd
service named `pocketid`, and published on GitHub at `pocket-id/pocket-id`.

Plugin configs needed:

1. **`generic_shell` config** — for version detection:

   ```json
   {
     "version_command": "/opt/pocket-id/pocket-id version | awk '{print $2}'"
   }
   ```

2. **`releases_github` config** — for fetching releases and installing assets. A single shared
   GitHub Releases config (with no `owner`/`repo` in the config itself) works for all
   GitHub-tracked items, since the repository is specified per software item as the
   `package_identifier`:

   ```json
   {
     "tag_strip_prefix": "v",
     "include_prereleases": false,
     "asset_patterns": []
   }
   ```

Role assignments on the host (`auth.uk-home.yantsen.su`):

| Role | Plugin config | Package identifier | Config override |
| --- | --- | --- | --- |
| `detect_version` | `Pocket ID (shell)` | `pocket-id` | _(none)_ |
| `fetch_releases` | `GitHub Releases` | `pocket-id/pocket-id` | `{"tag_strip_prefix": "v"}` |
| `execute_update` | `GitHub Releases` | `pocket-id/pocket-id` | `{"asset_patterns": ["pocket-id-linux-amd64"], "install_path": "/opt/pocket-id/pocket-id", "make_executable": true}` |
| `pre_update_hook` | `Systemd Hook` | — | `{"service_name": "pocketid"}` |
| `post_update_hook` | `Systemd Hook` | — | `{"service_name": "pocketid"}` |

The `execute_update` config override specifies:

- `asset_patterns` — narrows the GitHub release assets to the correct binary for this host's
  OS and architecture. Use a regex that matches exactly one asset per release.
- `install_path` — the absolute path where the binary is written.
- `make_executable` — sets the executable bit after installation.

The `pre_update_hook` and `post_update_hook` roles use the `hook_systemd` plugin to stop and
restart the systemd service around the binary replacement. See
[Update Lifecycle Plugins](../development/update-hooks.md) for details on hook plugins.

Per-host config overrides let you reuse the same `releases_github` plugin config across multiple
hosts or items while supplying host-specific asset patterns (e.g. `amd64` on one host, `arm64`
on another).

> [!NOTE]
> Sudoers note: The `systemctl stop` and `systemctl start` commands used by the systemd
> hook plugin must be allowlisted in the agent's sudoers file. Run the **Sync Host** action in
> the web UI (or `uptrakit surfaces ssh-agent.hosts --target-provider-id <PROVIDER_ID> sync-host <host-id>`)
> after configuring hook plugins to regenerate the sudoers file on the host. See
> [Sudoers Management](../security/sudoers-management.md) for details.

### Pattern B: Generic shell for both detection and updates

Use this pattern when the software has no GitHub releases page (or you prefer not to use the
GitHub API) and you can write a shell command to perform the update.

| Role | Plugin type | What it does |
| --- | --- | --- |
| `detect_version` | `generic_shell` | Reads the installed version via a shell command. |
| `fetch_releases` | `generic_shell` | _(not supported — leave unconfigured or use another release plugin)_ |
| `execute_update` | `generic_shell` | Runs a custom shell command to perform the update. |

A single `generic_shell` plugin config can cover both `detect_version` and `execute_update`
if you set both `version_command` and `update_command`:

```json
{
  "version_command": "my-app --version | head -1",
  "update_command": "sudo apt-get install -y my-app={version}"
}
```

The `{version}` placeholder is shell-escaped and substituted at update time.

When using `generic_shell` for `execute_update` without a `fetch_releases` plugin,
Uptrakit cannot determine whether an update is available — you must trigger updates manually
and supply the target version. Pairing it with a `releases_github`, `releases_gitlab`, or
`releases_forgejo` plugin config for the `fetch_releases` role gives you upstream version
awareness while keeping updates under full custom control.

### Pattern C: Detect-only tracking (no update support)

Use this pattern when you want Uptrakit to show the installed version for a piece of software
but updates are handled entirely outside Uptrakit (e.g. a manually compiled binary or a
proprietary tool).

| Role | Plugin type | What it does |
| --- | --- | --- |
| `detect_version` | `generic_shell` | Reads the installed version. |
| `fetch_releases` | `releases_github` (or similar) | Fetches upstream releases so you can see when a new version is available. |
| `execute_update` | _(unconfigured)_ | Uptrakit shows an update is available but cannot install it. |

Leave `execute_update` unconfigured. Uptrakit will detect version drift and display it in the
UI, but the **Update** button will be absent for that host assignment.

## Step-by-Step: Web UI

### 1. Create the plugin config(s)

Go to **Plugin Configs → New Plugin Config**.

**For version detection (`generic_shell`):**

- Plugin type: `Shell`
- Name: something descriptive, e.g. `Pocket ID (shell)`
- `version_command`: the shell command that prints the version string. The first non-empty
  trimmed line of stdout is used. See [Writing a version command](#writing-a-version-command).

**For upstream release tracking (`releases_github`):**

You can reuse an existing GitHub Releases config — one config serves all GitHub-tracked
repositories, since the `owner/repo` is specified per software item. If a `GitHub Releases`
config already exists, skip this step.

- Plugin type: `GitHub Releases`
- Name: `GitHub Releases`
- `tag_strip_prefix`: `v` (if release tags look like `v1.2.3`)
- Leave `asset_patterns`, `auth_token`, and other fields at their defaults unless needed.

### 2. Create the software item

Go to **Software → New Software Item**.

- **Name**: the display name shown in the UI (e.g. `Pocket ID`).
- **Enabled**: leave checked to include it in scheduled version checks.

The item is created without host assignments. Proceed to step 3.

### 3. Assign the item to a host

Open the context menu on the new software item and select **Assign to Hosts**. Check the host(s)
where the software is installed.

The **Role assignments for new hosts** table appears. Configure:

- **Detect Version** — select the `generic_shell` config, set the package identifier to whatever
  the `{package_identifier}` placeholder in your version command refers to (or any stable string
  if your command doesn't use the placeholder).
- **Fetch Releases** — select the `releases_github` config, set the package identifier to
  `owner/repo` (e.g. `pocket-id/pocket-id`).
- **Execute Update** — select the `releases_github` config, set the same `owner/repo` identifier.

Click **Save**.

### 4. Configure per-host overrides for `execute_update`

After assigning, open the software item detail page and click **Configure** on the host row.

In the **Execute Update** section, paste a JSON config override with the asset-specific fields:

```json
{
  "asset_patterns": ["pocket-id-linux-amd64"],
  "install_path": "/opt/pocket-id/pocket-id",
  "make_executable": true
}
```

To stop and restart a systemd service around the update, assign hook plugins:

- **Pre-Update Hook** — select a `Systemd Hook` config with `{"service_name": "pocketid"}`.
- **Post-Update Hook** — select the same `Systemd Hook` config.

Click **Save Changes**.

### 5. Trigger a version check

Go to **Software**, open the context menu on the item, and select **Check Versions**. After
the check completes, the **Installed** and **Latest** columns will populate.

## Step-by-Step: CLI

```bash
# 1. Create the shell plugin config (version detection)
SHELL_CONFIG_ID=$(uptrakit plugin-configs create \
  --name "Pocket ID (shell)" \
  --type generic_shell \
  --config '{"version_command":"/opt/pocket-id/pocket-id version | awk '"'"'{print $2}'"'"'"}' \
  --output json | jq -r '.id')

# 2. Find or create a shared GitHub Releases config
GH_CONFIG_ID=$(uptrakit plugin-configs list --output json \
  | jq -r '.[] | select(.plugin_type == "releases_github") | .id' | head -1)

# If none exists yet:
GH_CONFIG_ID=$(uptrakit plugin-configs create \
  --name "GitHub Releases" \
  --type releases_github \
  --config '{"tag_strip_prefix":"v","include_prereleases":false,"asset_patterns":[]}' \
  --output json | jq -r '.id')

# 3. Create the software item
ITEM_ID=$(uptrakit software-items create \
  --name "Pocket ID" \
  --output json | jq -r '.id')

# 4. Look up the host ID
HOST_ID=$(uptrakit hosts list --output json \
  | jq -r '.[] | select(.hostname == "auth.uk-home.yantsen.su") | .id')

# 5. Assign the host with all three roles
uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$SHELL_CONFIG_ID" \
  --package-identifier "pocket-id" \
  --role detect_version

uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$GH_CONFIG_ID" \
  --package-identifier "pocket-id/pocket-id" \
  --role fetch_releases

uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$GH_CONFIG_ID" \
  --package-identifier "pocket-id/pocket-id" \
  --role execute_update

# 6. Apply the per-host config override for execute_update
uptrakit software-items update-assignment "$ITEM_ID" \
  --host "$HOST_ID" \
  --role execute_update \
  --config-override '{
    "asset_patterns": ["pocket-id-linux-amd64"],
    "install_path": "/opt/pocket-id/pocket-id",
    "make_executable": true
  }'

# 7. Create a systemd hook config for the service lifecycle
HOOK_CONFIG_ID=$(uptrakit plugin-configs create \
  --name "Pocket ID (systemd hook)" \
  --type hook_systemd \
  --config '{"service_name":"pocketid"}' \
  --output json | jq -r '.id')

# 8. Assign pre/post update hooks
uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$HOOK_CONFIG_ID" \
  --role pre_update_hook

uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$HOOK_CONFIG_ID" \
  --role post_update_hook

# 9. Trigger an immediate version check
uptrakit check item "$ITEM_ID"
```

## Writing a Version Command

The `version_command` runs on the agent as the `uptrakit` user (unprivileged). The **first
non-empty trimmed line** of stdout is used as the version string.

Tips:

- Prefer `--version` or `version` subcommands if the binary supports them.
- Use `awk`, `sed`, or `grep` to extract just the version number from multi-word output:
  - `my-app --version | awk '{print $2}'` — take the second word.
  - `my-app version 2>&1 | grep -oP '\d+\.\d+\.\d+'` — extract a semver pattern.
- If the binary requires root to run, prefix with `sudo` and ensure the command is in the
  agent's sudoers allowlist.
- If the version is stored in a file (e.g. `/opt/my-app/version.txt`), read it directly:
  `cat /opt/my-app/version.txt`.
- Test the command manually on the host as the `uptrakit` user before saving the config:

  ```bash
  sudo -u uptrakit bash -c '/opt/pocket-id/pocket-id version | awk '"'"'{print $2}'"'"''
  ```

## Sudoers Considerations

The GitHub Releases plugin automatically declares `install` as a required sudo command when
`install_path` is configured. Hook plugins (e.g. `hook_systemd`) declare their own sudo
requirements — the systemd hook declares `systemctl stop *` and `systemctl start *`.

After adding or changing plugin configurations that affect sudoers, regenerate the sudoers file
on the host using the **Sync Host** action in the web UI or by running:

```bash
uptrakit surfaces ssh-agent.hosts --target-provider-id <PROVIDER_ID> sync-host <host-id>
```

This rewrites `/etc/sudoers.d/uptrakit` on the managed host to reflect the current plugin
declarations. See [Sudoers Management](../security/sudoers-management.md) for the full details.

## Related Documentation

- [Plugin Configurations](plugin-configs.md) — full reference for all plugin types and their
  config fields, including `generic_shell` and `releases_github`.
- [Autodiscovery](autodiscovery.md) — if the software can be discovered automatically, prefer
  that workflow.
- [Update Workflow](update-workflow.md) — how version checks and updates are scheduled and
  triggered.
- [Sudoers Management](../security/sudoers-management.md) — how sudoers files are generated and
  what commands require allowlisting.
- [GitHub Actions Attestation Verification](../security/github-attestation.md) — verifying the
  integrity of downloaded GitHub release assets.
