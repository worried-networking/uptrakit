# Update hooks

Update hooks allow running commands before and after software updates. They support two configuration formats:
structured hooks (with predefined templates) and custom commands.

## Configuration format

Hooks are configured in the plugin config or software item's `config_override` under a `hooks` key:

```json
{
  "hooks": {
    "pre_update": { ... },
    "post_update": { ... }
  }
}
```

Each hook phase (`pre_update`, `post_update`) can use:

1. **Predefined templates** — structured actions that map directly to commands
1. **Custom commands** — arbitrary shell commands

## Predefined hook templates

### Systemd service

Manages systemd services with explicit actions:

```json
{
  "hooks": {
    "pre_update": {
      "predefined": {
        "systemd_service": {
          "service_name": "myapp",
          "action": "stop"
        }
      }
    },
    "post_update": {
      "predefined": {
        "systemd_service": {
          "service_name": "myapp",
          "action": "start"
        }
      }
    }
  }
}
```

**Available actions:** `start`, `stop`, `restart`, `reload`

Resolved to: `Exec { program: "systemctl", args: [action, service_name] }` — executed directly without shell
interpretation.

### Docker Compose

Manages docker-compose deployments with explicit actions:

```json
{
  "hooks": {
    "pre_update": {
      "predefined": {
        "docker_compose": {
          "action": "down",
          "project_dir": "/opt/myapp"
        }
      }
    },
    "post_update": {
      "predefined": {
        "docker_compose": {
          "action": "up",
          "project_dir": "/opt/myapp"
        }
      }
    }
  }
}
```

**Available actions:** `up`, `down`, `restart`, `pull`

**Optional fields:**

- `project_dir` — directory to run the command in
- `compose_file` — path to compose file (uses `-f` flag)

Resolved to: `Exec { program: "docker-compose", args: [-f compose_file, action, -d], working_dir: project_dir }` —
executed directly without shell interpretation. The `-d` flag is added automatically for `up`.

## Custom commands

For commands not covered by predefined templates:

```json
{
  "hooks": {
    "pre_update": {
      "commands": ["echo 'Starting backup'", "backup.sh"],
      "shell": "bash"
    },
    "post_update": {
      "commands": ["systemctl restart myapp"],
      "shell": "bash"
    }
  }
}
```

## Shell types

The `shell` field controls which shell interpreter and fail-early settings are used:

| Shell | Fail-early settings | Description |
| :--------------- | :-------------------------------- | :---------------------------------------------- |
| `bash` (default) | `set -euo pipefail` | Exit on error, undefined vars, pipe failures. |
| `sh` | `set -eu` | POSIX-compatible exit on error, undefined vars. |
| `powershell` | `$ErrorActionPreference = 'Stop'` | Future Windows support. |

Commands are wrapped with fail-early settings before execution to ensure hooks fail fast on errors.

## Input validation

Predefined hook parameters are validated at the API boundary when plugin configs and software items are created or
updated:

- **Service names** (systemd): `[a-zA-Z0-9._@:-]+`, max 256 chars. Rejects shell metacharacters (`;`, `|`, `$`,
  backticks, etc.).
- **Paths** (project_dir, compose_file): `[a-zA-Z0-9._/ ~-]+`, max 4096 chars. Rejects `..` traversal and shell
  metacharacters.

Custom commands are not validated (they are intentionally arbitrary shell commands).

## Merge strategy

When both plugin config and software item `config_override` define hooks:

1. If override has a `hooks` key, it completely replaces the base config's hooks
1. If override doesn't have `hooks`, fall back to base config's hooks

## Phase markers in output

Hook output includes clear phase markers for debugging:

```text
[pre-hook] Starting pre-update hooks...
[pre-hook] Running: systemctl stop myapp
[pre-hook] (exit code 0)
[update] Executing update to version 2.0.0...
[post-hook] Starting post-update hooks...
[post-hook] Running: systemctl start myapp
[post-hook] (exit code 0)
[update] Update completed successfully
```

## Key files

| File | Purpose |
| :------------------------------------------------ | :-------------------------------------------------- |
| `crates/shared/web-api-types/src/update_hooks.rs` | Hook config types; re-exports `HookShell`. |
| `crates/ui/web-api/src/update_hooks.rs` | Hook resolution and merge logic. |
| `crates/core/agent/src/update.rs` | Hook execution with shell wrapper. |
| `crates/shared/wire/asyncapi.yaml` | Wire protocol docs (includes `hookCommand` schema). |
