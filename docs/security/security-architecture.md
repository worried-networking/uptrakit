# Security Architecture

Uptrakit follows a defense-in-depth model for agents, controller, and proxies.

- Agents run as an unprivileged account (e.g., `uptrakit`) and never accept inbound connections.
- All update execution is manual; the scheduler only triggers version checks.
- Sudo allowlists gate privileged agent commands. Custom scripts are treated as untrusted input.
- Public authentication endpoints and WebSocket connections are rate limited via the database-backed limiter.
- Secrets are never logged, and full command output is never stored internally; logs contain high-level summaries only.

See the other security docs for implementation detail on PKI, cryptography, secrets, reverse proxies, and developer expectations.

## Supply-Chain Verification

GitHub Releases are optionally verified against
[GitHub Actions attestations](github-attestation.md). Verification is two-stage:
the controller checks at fetch time and the agent independently re-verifies before
install. The `require_attestation` option blocks updates that lack a valid attestation.
See [GitHub Actions Attestation Verification](github-attestation.md) for full details.
