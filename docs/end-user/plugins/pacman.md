# Pacman Plugin

The `package_manager_pacman` plugin tracks and updates packages managed by **Pacman** on
Arch Linux and derivatives (Manjaro, EndeavourOS, etc.). It integrates with the local
`pacman` toolchain to detect installed versions, resolve the latest available versions, and
perform updates.

## What the Plugin Tracks

The Pacman plugin tracks packages installed and managed by the Arch Linux package manager.
For each tracked package, Uptrakit:

- Reports the **installed version** from the local Pacman database (`pacman -Q`).
- Resolves the **latest available version** from the configured repositories via
  `pacman -Si`.
- Executes package updates using `sudo pacman -S --noconfirm <package>`.

### Version Format

Pacman version strings use the format `<upstream_version>-<pkgrel>`, for example:
`1.26.3-1`, `3.12.4-1`, or `2:8.12.1-1` (epoch-prefixed). The full string including
the package release (`-<pkgrel>`) is stored for tracking.

> **Note:** Pacman always installs the latest available version from the repository.
> Unlike APT, there is no version-pinning mechanism in standard Pacman usage. When
> Uptrakit triggers an update, the `to_version` field is validated but only the package
> name is passed to `pacman -S --noconfirm`.

## Host Compatibility Detection

The Pacman plugin implements `PluginCapability::DetectHostCompatibility`. On each host, it
checks whether `pacman` is available by running `which pacman`. If the command is not found,
the plugin reports itself as incompatible and is skipped for that host.

This means Pacman plugin configs are automatically skipped on Debian, Ubuntu, macOS, or
any system that does not use Pacman.

## Configuration

### `discovery_filter` field

| Value | Description |
| --- | --- |
| *(omitted — default `{}`)* | All installed packages (`pacman -Q`). Plugin config is auto-created on first discovery. |
| `"all"` | All installed packages (`pacman -Q`) — explicit; uses pre-existing plugin config. |
| `"explicit"` | Only explicitly installed packages (`pacman -Qe`). |

**Default config** — no `discovery_filter` key, serialises as `{}`:

```json
{}
```

When the config is `{}` the plugin discovers every package reported by `pacman -Q` and emits
`DiscoveryTarget` values so the controller can auto-create the plugin config on the first run.
Subsequent runs use the auto-created config ID.

**Restrict to explicitly installed packages:**

```json
{ "discovery_filter": "explicit" }
```

Use `"explicit"` when you want to limit discovery to packages you intentionally installed via
`pacman -S`, omitting dependencies that were installed automatically.

## Auto-Created Plugin Config

When an agent discovers Pacman packages and no matching plugin config exists yet, Uptrakit
automatically creates one named **`Pacman`** with the default configuration (`{}`).

## Package Identifier Format

The `package_identifier` for Pacman packages is the **Arch Linux package name** as it appears
in the Pacman database:

- 1 to 128 characters long.
- Must start with a lowercase letter or digit (`[a-z0-9]`).
- May only contain lowercase letters, digits, `@`, `.`, `_`, `+`, and `-`.
- Must not contain `..`.
- Examples: `nginx`, `python`, `git`, `lib32-glibc`, `python3.11`, `my_package`.

## Required `sudoers` Entries

The agent runs as an unprivileged user (typically `uptrakit`). The `pacman` command
requires `sudo` access without a password for database sync and package installation:

```text
uptrakit ALL=(ALL) NOPASSWD: /usr/bin/pacman
```

Add this entry to `/etc/sudoers.d/uptrakit` on each managed host. Use `visudo` to
validate the syntax before saving:

```bash
sudo visudo -c -f /etc/sudoers.d/uptrakit
```

Unlike APT, Pacman does **not** require `SETENV` in the sudoers rule — no environment
variables need to be preserved.

> **Security note:** This rule grants the agent permission to run any `pacman` command as
> root. Consider restricting to specific subcommands (`-S`, `-Sy`) if your security policy
> requires it. See
> [Filesystem and Dependency Security](../../security/filesystem-dependency-security.md)
> for background on the agent's privilege model.

## Creating a Pacman Plugin Config via CLI

```bash
# Create a plugin config with the default filter (discovers all packages)
uptrakit plugin-configs create \
  --name "Pacman" \
  --type package_manager_pacman \
  --config '{}'

# Create a plugin config that discovers only explicitly installed packages
uptrakit plugin-configs create \
  --name "Pacman (Explicit)" \
  --type package_manager_pacman \
  --config '{"discovery_filter": "explicit"}'
```

## How It Works

### Package Database Sync

Before resolving upstream versions, the agent runs:

```bash
sudo pacman -Sy
```

This refreshes the Pacman repository databases so that `pacman -Si` returns current
version information.

### Autodiscovery

The plugin discovers installed packages in a single step:

- **`discovery_filter: null` or `"all"`:** Runs `pacman -Q` to list all installed packages.
- **`discovery_filter: "explicit"`:** Runs `pacman -Qe` to list only explicitly installed
  packages (excludes automatic dependencies).

### Version Detection

Runs `pacman -Q <package>` for the specific package. Exit code `1` (package not installed)
maps to `installed_version = null`.

### Latest Version Resolution

Runs `pacman -Si <package>` and extracts the `Version` field from the multi-line output.
Returns an empty list if the package is not found in any configured repository.

### Update Execution

Runs:

```bash
sudo pacman -S --noconfirm <package>
```

This installs the latest available version from the configured repositories.

### Batch Operations

- **Batch version detection:** A single `pacman -Q pkg1 pkg2 ...` call is used. Packages
  not found in the database are returned with `installed_version = null`.
- **Batch release fetching:** A single `pacman -Si pkg1 pkg2 ...` call is used. Output is
  parsed as blank-line-separated blocks, one per package.
- **Batch updates:** A single `pacman -S --noconfirm pkg1 pkg2 ...` call is used. Pacman
  treats the batch as a single transaction — all packages succeed or none do.

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — managing plugin configs, CRUD,
  and autodiscovery overview.
- [Autodiscovery](../autodiscovery.md) — discovery workflow, review process, and ignore
  list.
- [Filesystem and Dependency Security](../../security/filesystem-dependency-security.md) —
  agent privilege model and `sudoers` guidance.
