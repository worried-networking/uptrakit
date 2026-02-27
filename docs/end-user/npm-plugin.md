# npm Plugin (`package_manager_npm`)

The npm plugin tracks globally installed Node.js packages that are managed via
`npm install -g`. It supports version detection, controller-side release fetching from
the npm registry, autodiscovery of globally installed packages, and privileged updates.

## When to Use

Use the npm plugin for software installed via `npm install -g`, such as:

- `n8n` — workflow automation
- `pm2` — process manager
- `@angular/cli` — Angular command-line interface

Do **not** use the npm plugin for:

- Packages installed locally (non-global) — they are not tracked by this plugin.
- Applications distributed as Docker containers or GitHub releases — use `releases_docker`
  or `releases_github` instead.

## Package Identifier Format

The `package_identifier` for an npm software item must be a valid npm package name.

**Plain packages:**

- Must start with a lowercase letter or digit (`[a-z0-9]`).
- May only contain lowercase letters, digits, hyphens (`-`), dots (`.`), and underscores (`_`).
- Maximum 214 characters.
- Must not contain `..` (path traversal protection).
- Must not start with `.` or `_`.

Examples: `n8n`, `pm2`, `typescript`, `my-tool`

**Scoped packages:**

- Must start with `@`.
- Format: `@scope/name` where both `scope` and `name` follow the plain package rules above.
- Maximum 214 characters total.

Examples: `@angular/cli`, `@nestjs/cli`, `@scope/my-tool`

## Configuration

| Field | Required | Default | Description |
| --- | --- | --- | --- |
| `include_prereleases` | No | `false` | Include pre-release dist-tags (`next`, `beta`, `alpha`, `rc`, `canary`) in upstream release results. When `false`, only the `latest` dist-tag is returned. |

**Minimal configuration (all defaults):**

```json
{}
```

**With pre-releases enabled:**

```json
{ "include_prereleases": true }
```

## How It Works

### Version Detection (agent-side)

The agent runs:

```bash
npm list -g <package> --depth=0 --json
```

and parses the `dependencies.<package>.version` field from the JSON output. If the package
is not installed globally, the command exits non-zero and the agent reports the version as
absent.

### Release Fetching (controller-side)

The controller queries the npm registry directly:

```text
GET https://registry.npmjs.org/<package>
```

Scoped packages are URL-encoded: `@scope/name` → `https://registry.npmjs.org/@scope%2Fname`.

The plugin reads:

- **`dist-tags.latest`** — always returned as the primary release (`is_prerelease: false`).
- **Pre-release dist-tags** (`next`, `beta`, `alpha`, `rc`, `canary`) — returned only when
  `include_prereleases: true`, deduplicated against `latest`.

Published timestamps are parsed from the `time` object in the registry response.

### Updates (agent-side, privileged)

The agent executes:

```bash
sudo npm install -g <package>@<version>
```

The `sudo` invocation is handled automatically by the `SudoAwareCommandExecutor` — see
[sudo Requirements](#sudo-requirements) below.

### Autodiscovery

The plugin runs:

```bash
npm list -g --depth=0 --json
```

and reports all globally installed packages as discovered software. The following
package-manager infrastructure packages are filtered out and never surfaced as software items:

`npm`, `n`, `nvm`, `yarn`, `pnpm`, `corepack`

## sudo Requirements

The `npm install -g` command requires root on most Linux systems. Uptrakit generates a
minimal sudoers entry for the agent user:

```sudoers
uptrakit ALL=(root) NOPASSWD: /usr/bin/npm
```

> [!NOTE]
> The exact `npm` binary path (`/usr/bin/npm`, `/usr/local/bin/npm`, etc.) depends on your
> installation. The `update-sudoers` bootstrap command generates the correct entry for your
> system automatically. See [SSH Agent Bootstrap](ssh-agent-bootstrap.md) for setup.

## Proxmox Helper Scripts Integration

The PHS discovery plugin (`discovery_proxmox_helper_scripts`) detects npm-managed containers
by scanning CT scripts for `npm install -g <pkg>` lines. When a match is found (and the
package is globally installed), the PHS plugin emits a `PackageManagerNpm` discovery target
with the auto-detected package name.

Detection priority:

1. GitHub release management → `releases_github` + `generic_shell` targets
2. **npm global install → `package_manager_npm` target** ← this plugin
3. APT direct install → `package_manager_apt` target

For each npm-managed PHS container, a single `NPM (auto)` plugin config is created covering
all three roles (`detect_version`, `fetch_releases`, `execute_update`).

## Related Documentation

- [Plugin Configurations](plugin-configs.md) — overview of all plugin types.
- [SSH Agent Bootstrap](ssh-agent-bootstrap.md) — setting up sudo allowlists.
- [Proxmox Helper Scripts](../architecture/plugins/phs.md) — how PHS discovery integrates with npm.
