---
title: Secure Development
weight: 80
description: Secure coding expectations for contributors, including plugin input validation, SSRF prevention, and security-sensitive code review guidance.
---

# Secure Development

Developers must consult [Coding Standards](https://github.com/worried-networking/uptrakit/tree/main/docs/development/) for panic policies and design
boundaries, and [Error Handling](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for rootcause/thiserror patterns and the full
decision guide. Security-sensitive changes should also reference:

- [PKI and certificates](pki-certificates.md)
- [Secrets and encryption](secrets-and-encryption.md)
- [Reverse proxy security](reverse-proxy-security.md)
- [Filesystem and dependency security](filesystem-dependency-security.md)
- [CLI output formatting](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)

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

See [Plugin Guidelines — Package identifier validation](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the implementation pattern.

### Path traversal in plugin configs

Plugin config fields that represent filesystem paths (`compose_file`, `working_dir`,
`project_dir`) must reject `..` path segments to prevent directory traversal. The Docker plugin's
`DockerConfig::validate()` enforces this for both `compose_restart.compose_file` and
`compose_restart.working_dir`.

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

See [Plugin Guidelines — Version string validation](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the implementation pattern and testing requirements.

### Atomic ordering for security flags

`AtomicBool` flags that control security-sensitive behavior (such as `PLAINTEXT_MODE` in
`uptrakit-crypto`) must use `Ordering::Release` for stores and `Ordering::Acquire` for loads.
See [Coding Standards — Atomic Ordering Requirements](https://github.com/worried-networking/uptrakit/tree/main/docs/development/).

Build metadata exposed by `--version` is intentionally non-secret (crate version, enabled build features, target/cfg/profile). Never include
credentials, tokens, private keys, or runtime secret material in any version/build output.

## Freeze File Guard for Update Execution

Both `uptrakit-agent` and `uptrakit-agent-ssh` check for a freeze file at
`<state-dir>/update-freeze` before processing `ExecuteUpdate` or
`ExecuteBatchUpdate` messages. When the file exists, the message is
silently dropped and a `tracing::warn!` is emitted. This is an emergency stop
mechanism — not a per-command review gate.

The freeze file can be created in two ways:

1. **Locally:** `touch <state-dir>/update-freeze` on the agent host.
2. **Remotely:** The controller sends a `set_update_freeze` message (see
   [Wire Protocol — `set_update_freeze`](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)).

Any new `ControllerMessage` variant that triggers command execution on agents
**must** include the freeze file check in its handler. See:

- `crates/core/agent/src/main.rs` — local agent freeze check
- `crates/core/agent-ssh/src/main.rs` — SSH agent freeze check

## Agent-Side Execution Hardening

### Per-hook plugin timeout

Individual pre/post-update lifecycle hook plugin executions have a 5-minute
timeout (`HOOK_TIMEOUT = 300s`). On timeout, the hook's child process is killed
via `kill_on_drop(true)` and an error is returned. This prevents a single
malicious or stuck hook from consuming the entire update timeout budget.

### Update rate limiting

Both agents enforce an `UPDATE_COOLDOWN` of 5 seconds between consecutive
update executions. For the SSH agent, cooldown is tracked per-host. Updates
arriving within the cooldown window are rejected and emitted as
`system.service.update_gate` semantic audit events. This limits the damage
rate from a compromised controller while preserving an auditable gate
decision trail.

### Hook plugin execution logging

Before executing pre/post-update lifecycle hook plugins, agents emit a
structured `tracing::info!` event listing the hook plugin count. This
provides operational visibility for hook execution flow.

## Wire Protocol Payload Validation

Wire protocol payloads are validated after deserialization via the
`WireValidate` trait (`crates/shared/wire/src/limits.rs`). Per-collection and
per-string size limits prevent O(N) and O(N*M) processing attacks within the
1 MB WebSocket frame limit.

`AuditEventPayload` is an intentional exception: `ServiceMessage::AuditEvent`
is forwarded without wire-layer field validation so the controller can enforce
the canonical semantic-audit contract in one place
(`ingest_service_audit_event` / `validate_audit_event_payload` in
`crates/ui/web-api/src/routes/service_ws/handler/mod.rs`).

Any new wire protocol payload struct with `Vec<T>` or `String` fields **must**
implement `WireValidate` in `crates/shared/wire/src/wire_validate_impls.rs`
unless it is intentionally controller-validated like `AuditEventPayload`.

See [Wire Protocol — Payload Size Limits](https://github.com/worried-networking/uptrakit/tree/main/docs/api/)
for the full limits table.

## Semantic Audit Logging for Privileged Operations

All mutations to command-bearing plugin configs are logged via structured
semantic audit entries (`plugin_config.create`, `plugin_config.update`,
`plugin_config.delete`) with allow/deny outcomes. This creates an observable
trail for operations that grant effective RCE on managed hosts.

See [Coding Standards — Security Audit Logging](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the implementation pattern and required fields.

## Dangerous Command Pattern Rejection

Dangerous command pattern rejection is **enabled by default**. Plugin config
create/update requests containing known dangerous patterns are rejected with HTTP 400
before the database write. Operators who need to bypass this protection can use the
`--allow-dangerous-commands` CLI flag (or `UPTRAKIT_ALLOW_DANGEROUS_COMMANDS`
environment variable) to downgrade detection to advisory-only.

Detected patterns include:

- Pipe-to-shell (`curl|bash`, `wget|sh`, including `sudo`/`env`/`doas`/`run0` wrappers)
- Destructive filesystem operations (`rm -rf /`, `dd if=`, `mkfs.`)
- Fork bombs (`:(){ :|:& };:`)
- Bash network sockets (`/dev/tcp/`, `/dev/udp/`)

The detection logic lives in `uptrakit-web-api-types::command_validation::detect_dangerous_patterns`.
The rejection gate is in `crates/ui/web-api/src/routes/plugin_configs.rs`
(`collect_dangerous_patterns`, `format_dangerous_pattern_rejection`).

When disabled via `--allow-dangerous-commands`, detected patterns still appear in
semantic audit details for plugin config create/update outcomes. The underlying threat: an authenticated user with `manage_commands`
permission can craft plugin configs (shell commands, Docker `post_pull_command`, or hook plugin
commands) that execute arbitrary code on managed hosts. Mitigations include permission separation, dangerous
pattern rejection, command length limits, and semantic audit logging.

## SSRF Protection

All `reqwest::Client` instances that send requests to user-controlled URLs must use the
`SsrfSafeResolver` custom DNS resolver to prevent DNS rebinding attacks.

### The problem

`is_private_host()` validates hostnames at config-save time, but a DNS rebinding
attack can cause a hostname to resolve to a private IP at HTTP-request time.
For example, `evil.com` resolves to `1.2.3.4` during validation, then to
`169.254.169.254` when the actual request is made.

### The solution

`SsrfSafeResolver` (in `uptrakit_shared_types::ssrf`, feature `http-ssrf`) implements
`reqwest::dns::Resolve` and filters every resolved IP address through `is_private_ip()`.
If all resolved addresses are private, the request fails. If there is a mix of public
and private addresses, only public addresses are returned.

### Usage

```rust
use std::sync::Arc;
use uptrakit_shared_types::ssrf::SsrfSafeResolver;

// Standard mode: blocks private IPs
let client = reqwest::Client::builder()
    .dns_resolver(Arc::new(SsrfSafeResolver::new()))
    .connect_timeout(std::time::Duration::from_secs(10))
    .timeout(std::time::Duration::from_secs(60))
    .build()?;

// Permissive mode: allows all (for --allow-private-notification-urls)
let client = reqwest::Client::builder()
    .dns_resolver(Arc::new(SsrfSafeResolver::permissive()))
    .build()?;
```

### Where it is applied

All plugin HTTP clients and the webhook notification channel use `SsrfSafeResolver`:

- Release plugins: GitHub, GitLab, Forgejo, Docker registry
- Package manager plugins: npm
- Discovery plugins: Proxmox Helper Scripts
- Notification channels: Webhook (permissive when `--allow-private-notification-urls`)

The Docker plugin additionally validates the registry hostname in `validate_identifier()`
via `is_private_host()`, providing defence-in-depth at config-save time.

### When to add it

Any new `reqwest::Client` that sends requests to URLs derived from user configuration
must use `SsrfSafeResolver::new()`. Add `uptrakit-shared-types = { workspace = true,
features = ["http-ssrf"] }` to the crate's `Cargo.toml`.

Internal-only clients (e.g. the openapi-client connecting to a known controller URL, or
the CA certificate fetch in service-sdk) do not need SSRF protection because their
target URLs are not user-controlled.

## NATS Plugin Config Protection

Plugin configs published to NATS JetStream are encrypted with AES-256-GCM
using the shared master key before publication. This prevents NATS subscribers
(including compromised infrastructure) from reading API tokens, registry
passwords, and other credentials embedded in plugin configurations.

Any new `ControllerMessage` variant that carries plugin config fields with
credentials **must** be added to the `encrypt_message_configs()` /
`decrypt_message_configs()` match arms in
`crates/shared/nats/src/config_protection.rs`.

See [NATS Integration — Plugin Config Protection](https://github.com/worried-networking/uptrakit/tree/main/docs/development/)
for the full mechanism.
