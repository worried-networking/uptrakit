---
title: APT Plugin
weight: 10
description: Tracks and updates APT packages on Debian and Ubuntu systems.
---

# APT Plugin

The `package_manager_apt` plugin tracks and updates packages managed by **APT** (Advanced Package Tool) on
Debian and Ubuntu systems. It integrates with the local `dpkg`, `apt-get`, and `apt-cache`
toolchain to detect installed versions, resolve the latest available versions, and perform
updates.

## What the Plugin Tracks

The APT plugin tracks Debian packages installed and managed by the system package manager.
For each tracked package, Uptrakit:

- Reports the **installed version** from the `dpkg` database.
- Resolves the **latest available version** from the configured APT repository index via
  `apt-cache madison`.
- Executes package updates using `sudo apt-get install --yes --no-install-recommends
  <package>=<version>`.

### Version Format

APT version strings follow the Debian versioning scheme, for example:
`2:8.1.2269-1ubuntu5`, `1.24.0-2ubuntu7.3`, or `3.11.0-5ubuntu2`.

## Host Compatibility Detection

The APT plugin implements `PluginCapability::DetectHostCompatibility`. On each host, it checks
whether `apt-get` is available by running `which apt-get`. If the command is not found, the plugin
reports itself as incompatible and is skipped for that host.

This means APT plugin configs are automatically skipped on macOS, Windows, or any Linux
distribution that does not use APT (e.g. Fedora, Arch).

## Reboot Detection

To detect whether a reboot is required after an APT update, assign a `hook_shell` lifecycle
plugin to the `post_update_hook` role with a command like `test -f /var/run/reboot-required`.
See [Update Lifecycle Plugins](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) for details.

## Configuration

### `discovery_filter` field

| Value | Description |
| --- | --- |
| *(omitted — default `{}`)* | All installed packages reported by `dpkg`. Plugin config is auto-created on first discovery. |
| `"all"` | All installed packages reported by `dpkg` (explicit; uses pre-existing plugin config). |
| `"manual"` | Only packages the user explicitly installed (`apt-mark showmanual`). |

**Default config** — no `discovery_filter` key, serialises as `{}`):

```json
{}
```

When the config is `{}` the plugin discovers every package reported by `dpkg` and emits
`DiscoveryTarget` values so the controller can auto-create the plugin config on the first run.
Subsequent runs use the auto-created config ID.

**Restrict to manually-installed packages:**

```json
{ "discovery_filter": "manual" }
```

Use `"manual"` when you want to limit discovery to packages you intentionally installed via
`apt install`, omitting libraries and transitive dependencies.

## Auto-Created Plugin Config

When an agent discovers APT packages and no matching plugin config exists yet, Uptrakit
automatically creates one named **`APT`** with the default configuration (`{}`).

## Package Identifier Format

The `package_identifier` for APT packages is the **Debian package name** as it appears
in the `dpkg` database:

- 2 to 64 characters long.
- Must start with a lowercase letter or digit (`[a-z0-9]`).
- May only contain lowercase letters, digits, `+`, `-`, and `.`.
- Examples: `nginx`, `python3`, `apt-utils`, `g++`, `lib32z1`, `python3.11`.

## Required `sudoers` Entries

The agent runs as an unprivileged user (typically `uptrakit`). Two `apt-get` commands
require `sudo` access without a password:

```text
uptrakit ALL=(ALL) NOPASSWD: /usr/bin/apt-get update *
uptrakit ALL=(ALL) NOPASSWD: /usr/bin/apt-get install *
```

Add these entries to `/etc/sudoers.d/uptrakit` on each managed host. Use `visudo` to
validate the syntax before saving:

```bash
sudo visudo -c -f /etc/sudoers.d/uptrakit
```

> [!CAUTION]
> These rules restrict the allowed `sudo` invocations to
> `apt-get update` and `apt-get install` only. See
> [Filesystem and Dependency Security](../../security/filesystem-dependency-security.md)
> for background on the agent's privilege model.

## Creating an APT Plugin Config via CLI

```bash
# Create a plugin config with the default filter (discovers all packages)
uptrakit plugin-configs create \
  --name "APT" \
  --type package_manager_apt \
  --config '{}'

# Create a plugin config that discovers only manually-installed packages
uptrakit plugin-configs create \
  --name "APT (Manual)" \
  --type package_manager_apt \
  --config '{"discovery_filter": "manual"}'
```

## How It Works

### Package Index Refresh

Before resolving upstream versions, the agent runs:

```bash
sudo apt-get update -q
```

This refreshes the APT repository index so that `apt-cache madison` returns current
version information.

### Autodiscovery

The plugin discovers installed packages in up to two steps:

1. **Query all installed packages:** Runs `dpkg-query --show --showformat=...` to get
   all packages with non-empty versions.
2. **Filter (only when `discovery_filter: "manual"`):** Runs `apt-mark showmanual` to get
   the set of manually-installed packages and filters the results to that set. This step
   is skipped when the filter is `"all"` or when using the default empty config.

### Version Detection

Runs `dpkg-query --show --showformat=${Version}\n <package>` for the specific package.
Exit code `1` (package not found) maps to `installed_version = null`.

### Latest Version Resolution

Runs `apt-cache madison <package>` and takes the version from the **first line** of
output (highest-priority candidate according to the configured APT sources). Returns an
empty list if the package is not found in any repository.

### Update Execution

Runs:

```bash
sudo apt-get install --yes --no-install-recommends <package>=<version>
```

This pins the installation to the exact version selected by the user.

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — managing plugin configs, CRUD,
  and autodiscovery overview.
- [Autodiscovery](../autodiscovery.md) — discovery workflow, review process, and ignore
  list.
- [Filesystem and Dependency Security](../../security/filesystem-dependency-security.md) —
  agent privilege model and `sudoers` guidance.
