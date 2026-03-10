# DNF Plugin

The `package_manager_dnf` plugin tracks and updates packages managed by **DNF**
(Dandified YUM) on Fedora, RHEL, CentOS Stream, Rocky Linux, AlmaLinux, and other
RPM-based distributions. It integrates with the local `rpm` and `dnf` toolchain to
detect installed versions, resolve the latest available versions, and perform updates.

## What the Plugin Tracks

The DNF plugin tracks RPM packages installed and managed by the system package manager.
For each tracked package, Uptrakit:

- Reports the **installed version** (as `version-release`, e.g. `1.24.0-1.fc40`) from
  the RPM database via `rpm -q`.
- Resolves the **latest available version** from the configured DNF repository index
  via `dnf check-update`.
- Executes package updates using `sudo dnf install -y <package>-<version>-<release>`.

### Version Format

RPM version strings include a **release suffix** that identifies the specific package
build, for example: `1.24.0-1.fc40`, `8.0.1-2.el9`, or `2:8.1.2269-1.el9`.

The plugin stores and displays the full `version-release` string. Install commands
use the `pkg-version-release` form (e.g. `nginx-1.26.0-1.fc40`) to pin to a specific
build.

## Host Compatibility Detection

The DNF plugin implements `PluginCapability::DetectHostCompatibility`. On each host, it
checks whether `dnf` is available by running `which dnf`. If the command is not found,
the plugin reports itself as incompatible and is skipped for that host.

This means DNF plugin configs are automatically skipped on Debian/Ubuntu, macOS, or any
system that does not use DNF.

## Post-Update Hook

DNF does not use `/var/run/reboot-required`. The `PostUpdateHook` capability is
**not** implemented for this plugin; no post-update notification is emitted.

## Configuration

### `discovery_filter` field

| Value | Description |
| --- | --- |
| *(omitted — default `{}`)* | All installed RPM packages reported by `rpm -qa`. Plugin config is auto-created on first discovery. |
| `"all"` | All installed RPM packages reported by `rpm -qa` (explicit; uses pre-existing plugin config). |
| `"user_installed"` | Only packages installed by the user (`dnf repoquery --userinstalled`). |

**Default config** (no `discovery_filter` key, serialises as `{}`):

```json
{}
```

When the config is `{}` the plugin runs in **discover-all mode**: it discovers every
package reported by `rpm -qa` and emits `DiscoveryTarget` values so the controller can
auto-create the plugin config on the first run. Subsequent runs use the auto-created
config ID.

**Restrict to user-installed packages:**

```json
{ "discovery_filter": "user_installed" }
```

Use `"user_installed"` when you want to limit discovery to packages you intentionally
installed, omitting libraries and transitive dependencies auto-installed by the system.

## Auto-Created Plugin Config

When an agent discovers DNF packages and no matching plugin config exists yet, Uptrakit
automatically creates one named **`DNF`** with the default configuration (`{}`).

## Package Identifier Format

The `package_identifier` for DNF packages is the **RPM package name** as it appears in
the RPM database:

- 1 to 128 characters long.
- Must start with a letter or digit (`[a-zA-Z0-9]`).
- May only contain letters, digits, `.`, `_`, and `-`.
- Must not contain `..` (path traversal protection).
- Examples: `nginx`, `python3`, `httpd`, `httpd-devel`, `python3.11`, `2ping`.

## Required `sudoers` Entries

The agent runs as an unprivileged user (typically `uptrakit`). The `dnf` command
requires `sudo` access without a password for package installation and cache refresh:

```text
uptrakit ALL=(ALL) NOPASSWD: /usr/bin/dnf makecache *
uptrakit ALL=(ALL) NOPASSWD: /usr/bin/dnf install *
```

Add these entries to `/etc/sudoers.d/uptrakit` on each managed host. Use `visudo` to
validate the syntax before saving:

```bash
sudo visudo -c -f /etc/sudoers.d/uptrakit
```

> **Security note:** These rules restrict the allowed `sudo` invocations to `dnf
> makecache` and `dnf install` only. See
> [Filesystem and Dependency Security](../../security/filesystem-dependency-security.md)
> for background on the agent's privilege model.

## Creating a DNF Plugin Config via CLI

```bash
# Create a plugin config with the default filter (discovers all packages)
uptrakit plugin-configs create \
  --name "DNF" \
  --type package_manager_dnf \
  --config '{}'

# Create a plugin config that discovers only user-installed packages
uptrakit plugin-configs create \
  --name "DNF (User-installed)" \
  --type package_manager_dnf \
  --config '{"discovery_filter": "user_installed"}'
```

## How It Works

### Package Index Refresh

Before resolving upstream versions, the agent runs:

```bash
sudo dnf makecache -q
```

This refreshes the DNF repository metadata so that `dnf check-update` returns current
version information.

### Autodiscovery

The plugin discovers installed packages in up to two steps:

1. **Query all installed packages:** Runs `rpm -qa --queryformat '%{NAME}\t%{VERSION}-%{RELEASE}\n'`
   to get all packages with non-empty versions.
2. **Filter (only when `discovery_filter: "user_installed"`):** Runs
   `dnf repoquery --userinstalled --queryformat '%{name}'` to get the set of
   user-installed packages and filters results to that set. This step is skipped when
   the filter is `"all"` or when using the default empty config.

### Version Detection

Runs `rpm -q --queryformat '%{VERSION}-%{RELEASE}' <package>` for the specific package.
Exit code `1` (package not found) maps to `installed_version = null`.

### Latest Version Resolution

Runs `dnf check-update --quiet <package>`. DNF exit codes:

- **0** — package is up to date; returns an empty release list.
- **100** — an update is available; parses the output line to extract the
  `version-release` and repository name.
- **1** — fatal error.

Output lines have the form `name.arch    version-release    repository`. The repository
name is checked for the substring `"security"` to classify the update as
[`UpdateCategory::Security`].

### Update Execution

Runs:

```bash
sudo dnf install -y <package>-<version>-<release>
```

This pins the installation to the exact build selected by the user.

### Batch Operations

- **`batch_detect_installed_version`**: Single `rpm -qa <pkg1> <pkg2> ...
  --queryformat ...` call. A non-zero exit code (any package unknown) is treated as
  partial success; found packages are matched by name from the output.
- **`batch_fetch_releases`**: Single `dnf check-update --quiet <pkg1> <pkg2> ...`.
  Exit code 0 = all up to date (empty releases). Exit code 100 = parse output per
  package. Exit code 1 = error.
- **`execute_batch_update`**: Single `sudo dnf install -y <pkg1>-<ver1> <pkg2>-<ver2>
  ...`. DNF handles atomic multi-package installs natively.

## Related Documentation

- [Plugin Configurations](../plugin-configs.md) — managing plugin configs, CRUD,
  and autodiscovery overview.
- [Autodiscovery](../autodiscovery.md) — discovery workflow, review process, and ignore
  list.
- [Filesystem and Dependency Security](../../security/filesystem-dependency-security.md) —
  agent privilege model and `sudoers` guidance.
