# Update Lifecycle Plugins

Update lifecycle plugins run commands before and after software updates. They are standalone,
first-class plugin assignments — not embedded in the update plugin's config JSON.

## Overview

Two plugin types handle update lifecycle hooks:

| Plugin type | Wire value | Purpose |
| :--- | :--- | :--- |
| `HookSystemd` | `hook_systemd` | Stops/starts a systemd service around updates |
| `HookShell` | `hook_shell` | Runs arbitrary shell commands before/after updates |

These plugins are assigned to host software items via the `PreUpdateHook` (`pre_update_hook`)
and `PostUpdateHook` (`post_update_hook`) plugin roles. Multiple hooks can be assigned per
role; the `ordinal` column on `host_software_item_plugins` controls execution order.

## Plugin roles

| Role | String value | Phase | Abort semantics |
| :--- | :--- | :--- | :--- |
| `PreUpdateHook` | `pre_update_hook` | Before `execute_update` | First failure aborts the update |
| `PostUpdateHook` | `post_update_hook` | After `execute_update` | Errors logged as warnings, non-fatal |

## Systemd hook plugin (`hook_systemd`)

Manages a systemd service around updates.

### Configuration

```json
{
  "service_name": "nginx"
}
```

| Field | Type | Required | Description |
| :--- | :--- | :---: | :--- |
| `service_name` | string | yes | Systemd service unit name (e.g. `"nginx"`, `"my-app.service"`) |

### Validation

Service names must match `[a-zA-Z0-9._@:-]+` (max 256 chars). Shell metacharacters
(`;`, `|`, `$`, backticks, etc.) are rejected.

### Behaviour

- **Pre-hook**: `systemctl stop <service_name>` (privileged). Failure aborts the update.
- **Post-hook**: `systemctl start <service_name>` (privileged). Always runs, even after a
  failed update, to restore service state.

### Sudo requirements

The systemd hook plugin declares two sudo commands:

- `systemctl stop *`
- `systemctl start *`

## Shell hook plugin (`hook_shell`)

Runs arbitrary shell commands before and/or after an update.

### Configuration

```json
{
  "pre_command": "echo 'Starting backup'",
  "post_command": "systemctl restart myapp",
  "on_failure": true,
  "shell": "bash"
}
```

| Field | Type | Required | Default | Description |
| :--- | :--- | :---: | :--- | :--- |
| `pre_command` | string | no | — | Shell command to run before the update |
| `post_command` | string | no | — | Shell command to run after the update |
| `on_failure` | bool | no | `true` | Whether to run `post_command` even when the update fails |
| `shell` | string | no | `"bash"` | Shell interpreter: `"bash"` or `"sh"` |

At least one of `pre_command` or `post_command` must be set. Commands are limited to 4096
characters.

### Shell types

| Shell | Fail-early settings | Description |
| :--- | :--- | :--- |
| `bash` (default) | `set -euo pipefail` | Exit on error, undefined vars, pipe failures |
| `sh` | `set -eu` | POSIX-compatible exit on error, undefined vars |

Commands are wrapped with fail-early settings before execution.

### Behaviour

- **Pre-hook**: runs `pre_command` if set. Non-zero exit aborts the update.
- **Post-hook**: runs `post_command` if set. Respects `on_failure` flag. Errors are logged
  as warnings (non-fatal).

### Sudo requirements

None. Shell hook plugins run commands without privilege escalation. Users are responsible
for ensuring their commands have the necessary permissions.

## Execution flow

During a software update, hook plugins execute in this order:

1. **Pre-update hook plugins** (ordered by `ordinal` ASC): each plugin's `execute_pre_hook()`
   is called. The first failure aborts the entire update.
2. **Attestation gate** (if applicable).
3. **Main update** (`execute_update()`).
4. **Post-update hook plugins** (ordered by `ordinal` ASC): each plugin's `execute_post_hook()`
   is called with `update_succeeded` set. Errors are logged but do not fail the update.
5. **Version detection** (`detect_installed_version()`).

### Context

Hook plugins receive an `UpdateLifecycleContext` with:

| Field | Type | Description |
| :--- | :--- | :--- |
| `package_identifier` | `String` | The package being updated |
| `to_version` | `String` | Target version |
| `from_version` | `Option<String>` | Installed version before the update (if detected) |
| `release_info` | `Option<ReleaseInfo>` | Release metadata from upstream |
| `update_succeeded` | `Option<bool>` | `None` during pre-hooks, `Some(true/false)` during post-hooks |

### Phase markers in output

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

## Wire protocol

Hook plugins are sent as `PluginAssignment` entries in `ExecuteUpdatePayload`:

```json
{
  "pre_update_hook_plugins": [
    {
      "plugin_type": "hook_systemd",
      "package_identifier": "",
      "config": { "service_name": "nginx" }
    }
  ],
  "post_update_hook_plugins": [
    {
      "plugin_type": "hook_systemd",
      "package_identifier": "",
      "config": { "service_name": "nginx" }
    }
  ]
}
```

The controller loads hook plugin assignments from `host_software_item_plugins` (roles
`pre_update_hook` and `post_update_hook`), resolves effective configs via three-layer merge,
and populates these arrays. The agent instantiates each plugin via the plugin registry and
calls the lifecycle methods.

## Web UI

Hook plugins are assigned via the same plugin assignment modals used for core roles
(`detect_version`, `fetch_releases`, `execute_update`). The **Configure Plugins** modal
(opened from the host context menu on a software item detail page) and the **Assign to Host**
modal both include `Pre-Update Hook` and `Post-Update Hook` role sections.

The plugin config dropdown is filtered per role:

- Hook roles only show plugin configs whose plugin type has `pre_update_hook` or
  `post_update_hook` capability (i.e. `hook_systemd` and `hook_shell` configs).
- Core roles exclude hook-type plugin configs.

Hook roles do not show the **Package ID** or **Execution Site** fields, since hooks do not
use package identifiers and always run on the agent.

## CLI

The CLI uses the same `PluginRole` enum as the API. Hook roles work with all existing
commands:

```bash
# Create a systemd hook config
HOOK_ID=$(uptrakit plugin-configs create \
  --name "My Service Hook" \
  --type hook_systemd \
  --config '{"service_name":"myapp"}' \
  --output json | jq -r '.id')

# Assign pre-update hook
uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$HOOK_ID" \
  --role pre_update_hook

# Assign post-update hook
uptrakit software-items assign "$ITEM_ID" \
  --host "$HOST_ID" \
  --plugin-config "$HOOK_ID" \
  --role post_update_hook
```

## Key files

| File | Purpose |
| :--- | :--- |
| `crates/plugins/hooks/systemd/` | Systemd hook plugin crate |
| `crates/plugins/hooks/shell/` | Shell hook plugin crate |
| `crates/plugins/infrastructure/core/src/plugin_base.rs` | `UpdateLifecyclePlugin` trait |
| `crates/plugins/infrastructure/core/src/traits.rs` | `UpdateLifecycleContext`, `PreUpdateHookResult` |
| `crates/shared/agent-core/src/update.rs` | Hook execution in update pipeline |
| `crates/shared/wire/asyncapi.yaml` | Wire protocol schema |

## Related documentation

- [Plugin System Architecture](plugin-system.md) — overall plugin architecture
- [Plugin Guidelines](plugin-guidelines.md) — plugin development conventions
- [Command Executor](command-executor.md) — `CommandSpec` and `CommandExecutor` trait
- [Security: Shell Injection](../security/shell-injection.md) — security considerations for shell commands
