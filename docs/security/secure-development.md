# Secure Development

Developers must consult [Coding Standards](../development/coding-standards.md) for panic policies and design
boundaries, and [Error Handling](../development/error-handling.md) for rootcause/thiserror patterns and the full
decision guide. Security-sensitive changes should also reference:

- [PKI and certificates](pki-certificates.md)
- [Secrets and encryption](secrets-and-encryption.md)
- [Reverse proxy security](reverse-proxy-security.md)
- [Filesystem and dependency security](filesystem-dependency-security.md)
- [CLI output formatting](../development/cli-output.md)

Document any new behavior or configuration in the appropriate `docs/` area and ensure tests cover both success and failure paths.

## Plugin Input Validation

Plugins are a security boundary — they interpolate user-controlled values (package identifiers,
version strings) into system commands. All input that flows into `CommandSpec` arguments must be
validated at the plugin level before command construction.

### Package identifier validation

Every plugin that accepts a `package_identifier` parameter must validate it via a
`validate_identifier()` function before any use. The validation enforces:

- Character whitelist specific to the package manager's naming rules
- Length bounds (typically 2–128 characters)
- Path traversal rejection (`..` segments)
- First-character constraints (e.g., must start with a letter or digit)

See [Plugin Guidelines — Package identifier validation](../development/plugin-guidelines.md#package-identifier-validation)
for the implementation pattern.

### Path traversal in plugin configs

Plugin config fields that represent filesystem paths (`compose_file`, `working_dir`,
`project_dir`) must reject `..` path segments to prevent directory traversal. The Docker plugin's
`DockerConfig::validate()` enforces this for both `compose_restart.compose_file` and
`compose_restart.working_dir`. The structured hook system's `DockerComposeHook::validate()`
applies the same check to `compose_file` and `project_dir`.

Any new plugin config field that accepts a path must apply the same validation pattern:

```rust
if path.split('/').any(|seg| seg == "..") {
    bail!(PluginError::Configuration("field must not contain '..' path segments".into()));
}
```

### Version string validation

Plugins that interpolate a `to_version` parameter into install commands (e.g.,
`npm install -g pkg@version`, `apt-get install pkg=version`) must validate the version string.
Even though `CommandSpec::exec()` mode prevents shell injection, package managers have their
own argument parsing:

- **npm:** A version like `file:../malicious` or `git+https://attacker.com` could install
  attacker-controlled packages.
- **apt:** A version like `1.0 --allow-unauthenticated` could alter command behavior through
  flag injection.

Validation rules per plugin:

| Plugin | Allowed characters | Rejected patterns |
| :--- | :--- | :--- |
| npm | `[a-zA-Z0-9._+-]` | Empty, >256 chars, `file:`, `git+`, `http:`, `https:` prefixes |
| apt | `[a-zA-Z0-9.+~:-]` | Empty, >256 chars, leading `-` (flag injection) |

See [Plugin Guidelines — Version string validation](../development/plugin-guidelines.md#version-string-validation)
for the implementation pattern and testing requirements.

### Atomic ordering for security flags

`AtomicBool` flags that control security-sensitive behavior (such as `PLAINTEXT_MODE` in
`uptrakit-crypto`) must use `Ordering::Release` for stores and `Ordering::Acquire` for loads.
See [Coding Standards — Atomic Ordering Requirements](../development/coding-standards.md#atomic-ordering-requirements).

Build metadata exposed by `--version` is intentionally non-secret (crate version, enabled build features, target/cfg/profile). Never include
credentials, tokens, private keys, or runtime secret material in any version/build output.

## Freeze File Guard for Update Execution

Both `uptrakit-agent` and `uptrakit-agent-ssh` check for a freeze file at
`<state-dir>/update-freeze` before processing `ExecuteUpdate` or
`ExecuteBatchHostPackageUpdate` messages. When the file exists, the message is
silently dropped and a `tracing::warn!` is emitted. This is an emergency stop
mechanism — not a per-command review gate.

The freeze file can be created in two ways:

1. **Locally:** `touch <state-dir>/update-freeze` on the agent host.
2. **Remotely:** The controller sends a `set_update_freeze` message (see
   [Wire Protocol — `set_update_freeze`](../api/wire-protocol.md#set_update_freeze-payload)).

Any new `ControllerMessage` variant that triggers command execution on agents
**must** include the freeze file check in its handler. See:

- `crates/core/agent/src/main.rs` — local agent freeze check
- `crates/core/agent-ssh/src/main.rs` — SSH agent freeze check

## Agent-Side Execution Hardening

### Per-hook timeout

Individual pre/post-update hooks have a 5-minute timeout (`HOOK_TIMEOUT =
300s`). On timeout, the hook's child process is killed via `kill_on_drop(true)`
and an `UpdateError::HookFailed` is returned. This prevents a single malicious
or stuck hook from consuming the entire update timeout budget.

### Update rate limiting

Both agents enforce an `UPDATE_COOLDOWN` of 5 seconds between consecutive
update executions. For the SSH agent, cooldown is tracked per-host. Updates
arriving within the cooldown window are rejected with a `security_audit:`
warning. This limits the damage rate from a compromised controller.

### Hook audit logging

Before executing pre/post-update hooks, agents emit a `security_audit:` warning
listing the hook count and command summaries. This enables forensic analysis of
all commands executed on managed hosts.

## Wire Protocol Payload Validation

All wire protocol payloads are validated after deserialization via the
`WireValidate` trait (`crates/shared/wire/src/limits.rs`). Per-collection and
per-string size limits prevent O(N) and O(N*M) processing attacks within the
1 MB WebSocket frame limit.

Any new wire protocol payload struct with `Vec<T>` or `String` fields **must**
implement `WireValidate` in `crates/shared/wire/src/wire_validate_impls.rs`.

See [Wire Protocol — Payload Size Limits](../api/wire-protocol.md#payload-size-limits)
for the full limits table.

## Security Audit Logging for Privileged Operations

All mutations to command-bearing plugin configs are logged via structured
`tracing::warn!` events with the `security_audit:` prefix. This creates an
observable trail for operations that grant effective RCE on managed hosts.

See [Coding Standards — Security Audit Logging](../development/coding-standards.md#security-audit-logging)
for the implementation pattern and required fields.

## Dangerous Command Pattern Rejection

The `--reject-dangerous-commands` CLI flag (or `UPTRAKIT_REJECT_DANGEROUS_COMMANDS`
environment variable) upgrades the advisory dangerous pattern detection to a blocking
policy. When enabled, plugin config create/update requests containing known dangerous
patterns are rejected with HTTP 400 before the database write.

Detected patterns include:

- Pipe-to-shell (`curl|bash`, `wget|sh`, including `sudo`/`env`/`doas`/`run0` wrappers)
- Destructive filesystem operations (`rm -rf /`, `dd if=`, `mkfs.`)
- Fork bombs (`:(){ :|:& };:`)
- Bash network sockets (`/dev/tcp/`, `/dev/udp/`)

The detection logic lives in `uptrakit-web-api-types::command_validation::detect_dangerous_patterns`.
The rejection gate is in `crates/ui/web-api/src/routes/plugin_configs.rs`
(`collect_dangerous_patterns`, `format_dangerous_pattern_rejection`).

The flag is off by default for backward compatibility. When disabled, detected
patterns are still logged as `security_audit:` warnings. See
[ATK-16](../hackme/16-rce-plugin-config-manipulation.md) for the threat model.

## NATS Plugin Config Protection

Plugin configs published to NATS JetStream are encrypted with AES-256-GCM
using the shared master key before publication. This prevents NATS subscribers
(including compromised infrastructure) from reading API tokens, registry
passwords, and other credentials embedded in plugin configurations.

Any new `ControllerMessage` variant that carries plugin config fields with
credentials **must** be added to the `encrypt_message_configs()` /
`decrypt_message_configs()` match arms in
`crates/shared/nats/src/config_protection.rs`.

See [NATS Integration — Plugin Config Protection](../development/nats-integration.md#plugin-config-protection)
for the full mechanism.
