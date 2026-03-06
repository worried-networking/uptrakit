# Security Architecture

Uptrakit follows a defense-in-depth model for agents, controller, and proxies.

- Agents run as an unprivileged account (e.g., `uptrakit`) and never accept inbound connections.
- All update execution is manual; the scheduler only triggers version checks.
- Sudo allowlists gate privileged agent commands. Custom scripts are treated as untrusted input.
- Public authentication endpoints and WebSocket connections are rate limited via the database-backed limiter.
- Secrets are never logged, and full command output is never stored internally; logs contain high-level summaries only.

See the other security docs for implementation detail on PKI, cryptography, secrets, reverse proxies, and developer expectations.

## Security Response Headers

The controller sets the following security headers on every HTTP response via
the `security_headers` middleware (`crates/ui/web-api/src/middleware/security_headers.rs`).
The middleware is applied as the outermost layer on both the main HTTPS router and the
PKI HTTP router.

| Header | Value | Purpose |
| --- | --- | --- |
| `X-Content-Type-Options` | `nosniff` | Prevents MIME-type sniffing |
| `X-Frame-Options` | `DENY` | Blocks framing (clickjacking protection) |
| `X-XSS-Protection` | `0` | Disables legacy XSS filter (can introduce vulnerabilities) |
| `Referrer-Policy` | `strict-origin-when-cross-origin` | Limits referrer leakage |
| `Content-Security-Policy` | `default-src 'self'; frame-ancestors 'none'` | Restricts resource origins |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` | Enforces HTTPS |
| `Permissions-Policy` | `camera=(), microphone=(), geolocation=()` | Disables unnecessary browser APIs |

See also [coding-standards.md](../development/coding-standards.md) for middleware ordering
conventions.

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
