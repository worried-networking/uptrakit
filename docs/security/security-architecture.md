# Security Architecture

Uptrakit follows a defense-in-depth model for agents, controller, and proxies.

- Agents run as an unprivileged account (e.g., `uptrakit`) and never accept inbound connections.
- All update execution is manual; the scheduler only triggers version checks.
- Sudo allowlists gate privileged agent commands. Custom scripts are treated as untrusted input.
- Public authentication endpoints and WebSocket connections are rate limited via the database-backed limiter.
- Secrets are never logged, and full command output is never stored internally; logs contain high-level summaries only.

See the other security docs for implementation detail on PKI, cryptography, secrets, reverse proxies, and developer expectations.

## Agent Host Identity (Machine ID)

Each agent identifies its host using a persistent machine identifier read from
the operating system:

- **Linux:** `/etc/machine-id` (standard systemd file).
- **macOS:** `IOPlatformUUID` from `ioreg`.

The machine ID is used to scope message routing in the controller — only messages
addressed to the agent's machine ID are processed.

### Fallback behaviour

When no persistent machine ID can be determined (containers without
`/etc/machine-id`, exotic operating systems, permission errors) the agent
generates a **session-unique fallback** of the form `unknown-<uuidv7>` and
emits a `WARN`-level log:

```text
machine-ID could not be determined; using session-unique fallback.
Host identity will not persist across restarts.
```

**Security implications of the fallback:**

- Each agent restart generates a new machine ID, so the controller cannot
  distinguish restarts from new hosts. This prevents machine-ID-scoped message
  routing from being a reliable security boundary in containerised deployments.
- Two agents that both fall back will get distinct identifiers (UUIDv7 is
  time-ordered and unique), so they do not inadvertently share state.
- Operators **must** provision `/etc/machine-id` in containerised environments
  to restore reliable host identity. On Debian/Ubuntu-based containers:

  ```sh
  systemd-machine-id-setup
  ```

  Or mount a persistent volume containing a pre-generated machine ID file.

See [Sudoers Management](sudoers-management.md) for related host-privilege guidance.

## Supply-Chain Verification

GitHub Releases are optionally verified against
[GitHub Actions attestations](github-attestation.md). Verification is two-stage:
the controller checks at fetch time and the agent independently re-verifies before
install. The `require_attestation` option blocks updates that lack a valid attestation.
See [GitHub Actions Attestation Verification](github-attestation.md) for full details.
