# Host packages

Host packages track system-level software updates per-host. When a package manager plugin (APT, Homebrew, npm)
discovers packages on a host, they appear in the host's Packages view with version information and update
availability.

## How it works

1. **Discovery**: Package manager plugins scan the host and report installed packages
2. **Tracking**: Each package is recorded with its installed version and latest available version
3. **Monitoring**: The host detail page shows aggregate update counts (total and security)
4. **Updates**: Batch updates execute a single package manager command for efficiency

## Viewing packages

### Web UI

- **Hosts list**: The "Updates" and "Security" columns show aggregate counts with colored badges
- **Host detail**: The "Package Updates" card shows summary counts with links to filtered views
- **Packages page**: Click through to `/hosts/{id}/packages` for the full package list with
  filtering by name, update status, and category

### CLI

```bash
# List all packages on a host
uptrakit host-packages list <host_id>

# Filter to packages with available updates
uptrakit host-packages list <host_id> --has-update true

# Filter to security updates only
uptrakit host-packages list <host_id> --has-update true --category security

# Search by name
uptrakit host-packages list <host_id> --search nginx

# Show package detail
uptrakit host-packages show <host_id> <package_id>
```

## Managing packages

### Enable/disable

Disabled packages are excluded from version checks and batch updates.

```bash
uptrakit host-packages enable <host_id> <package_id>
uptrakit host-packages disable <host_id> <package_id>
```

### Delete and ignore

Deleting a package removes it from tracking. Use `--ignore` to also prevent re-discovery:

```bash
# Delete only
uptrakit host-packages delete <host_id> <package_id>

# Delete and create an ignore rule
uptrakit host-packages delete <host_id> <package_id> --ignore
```

### Ignore rules

Ignore rules prevent specific packages from being re-discovered by a plugin:

```bash
# List ignore rules
uptrakit host-packages ignore list <host_id>

# Add an ignore rule
uptrakit host-packages ignore add <host_id> --plugin-config <uuid> --package nginx

# Remove an ignore rule
uptrakit host-packages ignore remove <host_id> <ignore_id>
```

## Update categories

Packages are classified by update category:

| Category | Description |
| :------- | :---------- |
| `security` | Security updates (e.g., from APT security repositories) |
| `standard` | Regular updates |
| `unknown` | Category not determined |

The APT plugin detects security updates by checking if the package source is a `*-security` repository.

## Relationship to software items

Host packages and [software items](update-workflow.md) are complementary tracking systems:

- **Software items** are for specific items you want to track closely (Docker images, GitHub releases).
  They appear in the main Software list and require approval after discovery.
- **Host packages** are for aggregate system update monitoring. They are created immediately on discovery
  (no approval step) and shown per-host.

The same package can exist in both systems simultaneously. Updates through either system work independently.

## Related documentation

- [Autodiscovery](autodiscovery.md) — how packages are discovered
- [Update workflow](update-workflow.md) — targeted software item updates
- [CLI usage](cli-usage.md) — full CLI reference
- [API: host packages](../api/host-packages.md) — REST API reference
