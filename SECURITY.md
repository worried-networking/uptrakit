# Security Overview

## Supported Versions

Uptrakit is pre-release software. Security fixes apply to the latest development branch only.

## Reporting

Report vulnerabilities via GitHub's [Report a vulnerability](https://github.com/uptrakit/uptrakit/security/advisories/new) workflow or open a minimal
issue requesting a private channel. Do **not** post exploit details publicly. Expect acknowledgement within 48 hours and an initial assessment within
a week.

## Security stance

- Agents run as unprivileged users, execute updates via sudo allowlists, and connect outbound only.
- The optional embedded agent (`embedded-agent` feature) runs inside the controller process and
  inherits its privileges. Use sudo allowlists to constrain update commands, and prefer a separate
  agent binary in multi-tenant or hardened deployments.
- The optional embedded SSH agent (`embedded-ssh-agent` feature) runs inside the controller
  process and manages remote hosts over SSH. SSH private keys are stored encrypted in the
  controller's shared database using the controller's master key and data key ring. An
  ephemeral ECIES P-256 key pair is generated at startup for extension parameter decryption.
  Prefer a separate `uptrakit-agent-ssh` binary in multi-tenant or hardened deployments.
- The scheduler never runs automatic updates; all actions require user confirmation.
- Secrets are encrypted at rest, never logged, and sensitive endpoints are rate limited.
- Master key consistency is verified at startup to prevent silent decryption failures in HA deployments.
- Infrastructure credentials (database URL, NATS URL, master key) are delivered only to services with
  explicit credential capabilities, exclusively via authenticated WebSocket — never via NATS or other channels.

## Detailed guidance

- PKI, certificates, and revocation flows: [docs/security/pki-certificates.md](docs/security/pki-certificates.md)
- Authentication and permissions: [docs/security/auth-and-authorization.md](docs/security/auth-and-authorization.md)
- Secrets, encryption, and master key handling: [docs/security/secrets-and-encryption.md](docs/security/secrets-and-encryption.md)
- Reverse proxy security: [docs/security/reverse-proxy-security.md](docs/security/reverse-proxy-security.md)
- TOFU & TLS hardening: [docs/security/tofu-tls.md](docs/security/tofu-tls.md)
- Filesystem/dependency hardening: [docs/security/filesystem-dependency-security.md](docs/security/filesystem-dependency-security.md)
- Secure development practices: [docs/security/secure-development.md](docs/security/secure-development.md)
- Notification security (secret storage, callback verification, action tokens): [docs/security/notifications-security.md](docs/security/notifications-security.md)
- Audit log security (data scope, tenant isolation, retention, GDPR): [docs/security/audit-logs.md](docs/security/audit-logs.md)

## Disclosure policy

We follow coordinated disclosure and release fixes promptly after verification. Reporters receive credit in release notes unless anonymity is
requested.
