# Security Policy

## Supported Versions

| Version | Supported |
| --- | --- |
| Latest on `main` | Yes |
| Older commits | No |

Uptrakit is pre-release software. Security fixes are applied to the latest development branch only.

## Reporting a Vulnerability

If you discover a security vulnerability in Uptrakit, please report it responsibly:

1. **Preferred**: Use GitHub's [Report a vulnerability](https://github.com/uptrakit/uptrakit/security/advisories/new) feature (Security tab > Advisories > New draft).
2. **Alternative**: Open a minimal issue requesting a private disclosure channel. Keep technical details out of the public thread.

Please **do not** post exploit details, proof-of-concept code, or affected credentials in public issues or discussions.

We aim to acknowledge reports within 48 hours and provide an initial assessment within one week.

## Security Design Overview

Uptrakit is designed with a defence-in-depth approach. Key principles:

- **Agents are unprivileged.** They run as a dedicated system user (e.g. `uptrakit`) with no elevated capabilities by default.
- **Outbound-only connections.** Agents connect to the controller via secure WebSocket. They never listen on any port or accept inbound connections.
- **No automatic updates.** The scheduler triggers version *checks* only. All update execution requires explicit user action via the Web UI, CLI, or Home Assistant.
- **Sudo allowlists.** Privileged operations on agents are constrained to specific commands granted `NOPASSWD` sudo access. No blanket root access.
- **No shell injection.** Any path that constructs or executes shell commands validates inputs. Custom scripts are treated as untrusted input.
- **No secrets in logs.** Tokens, passwords, API keys, and other credentials are never logged.

## Cryptographic Details

| Component | Implementation | Purpose |
| --- | --- | --- |
| TLS library | [Rustls](https://github.com/rustls/rustls) with [aws-lc-rs](https://github.com/aws/aws-lc-rs) backend | All controller HTTPS and agent WebSocket connections |
| CA key algorithm | ECDSA P-256 | Internal CA and all issued certificates |
| Certificate hashing | SHA-256 | Certificate signing and CRL generation |
| Password hashing | [Argon2id](https://crates.io/crates/argon2) (OWASP-recommended: 19 MiB, 2 iterations) | User password storage |
| JWT signing | [jsonwebtoken](https://crates.io/crates/jsonwebtoken) | Access and refresh tokens |
| Session tokens | SHA-256 hashed, 7-day expiry, 30-min sliding window | Stateful user sessions |

No custom cryptographic implementations are used.

## Certificate Lifecycle

Uptrakit operates an internal PKI for mTLS agent authentication. The controller acts as the Certificate Authority.

| Asset | Default lifetime | Renewal window |
| --- | --- | --- |
| CA certificate | 5 years | 6 months before expiry (automatic rotation) |
| Server HTTPS certificate | 90 days | 30 days before expiry (automatic renewal) |
| Agent client certificate | 365 days (configurable) | Configurable via `renewal_window_hours` |

CA rotation produces a dual-CA trust bundle so agents signed by the previous CA remain trusted during the transition period. CRLs are partitioned per CA -- each CA signs a CRL only for certificates it issued. Combined PEM CRLs are publicly available at `GET /api/v1/ca.crl`.

At startup, the controller validates that `--san` CLI values are present in the managed server certificate's SANs. Mismatches trigger automatic regeneration when the cert was signed by the active CA, or a guided error when the cert was signed by a previous CA (preventing accidental re-signing under a rotated CA).

For the full operational flow (rotation steps, bundle distribution, `CaSnapshot` sharing), see [AGENTS.md](AGENTS.md) section "PKI & CA rotation".

## Authentication Methods

| Method | Scope | Details |
| --- | --- | --- |
| Password (Argon2id) | User login | Local accounts with hashed passwords |
| OIDC | User login | External identity providers; auto-create or link accounts |
| JWT access tokens | API requests | Short-lived; carry resolved permissions (not roles) |
| Device authorization | CLI login | Short-lived user code + browser approval; results in API token |
| API tokens | Programmatic access | Long-lived bearer tokens; revocable |
| mTLS client certificates | Agent connections | Issued during enrollment; validated on every WebSocket connection |
| Enrollment tokens | Agent registration | One-time tokens with expiry for initial agent enrollment |

Authorization is permission-based (typed `Permission` enum), not role-string-based. See [AGENTS.md](AGENTS.md) section "Permissions model" for the full RBAC design.

## Secrets Handling

- Passwords are hashed with Argon2id before storage; plaintext is never persisted.
- Session tokens are SHA-256 hashed in the database.
- JWT signing keys are held in memory only.
- Agent private keys are generated and stored locally on each agent; they never leave the host.
- CA private keys are stored on the controller filesystem only.
- No secrets appear in log output, error messages, or API responses.

## Dependency Security

- **[cargo-deny](https://github.com/EmbarkStudios/cargo-deny)**: Runs in CI to check for known vulnerabilities (RustSec advisory database), license compliance, and dependency issues.
- **[Dependabot](https://docs.github.com/en/code-security/dependabot)**: Monitors Cargo, npm, and GitHub Actions dependencies weekly with automatic pull requests for updates.
- Dependencies affecting command execution, untrusted input parsing, cryptography, or networking receive extra scrutiny during review.

## Disclosure Policy

- We follow coordinated disclosure practices.
- Security fixes will be released as soon as practical after verification.
- Reporters will be credited in release notes (unless they prefer anonymity).
