# Security Overview

## Supported Versions

Uptrakit is pre-release software. Security fixes apply to the latest development branch only.

## Reporting

Report vulnerabilities via GitHub's [Report a vulnerability](https://github.com/uptrakit/uptrakit/security/advisories/new) workflow or open a minimal
issue requesting a private channel. Do **not** post exploit details publicly. Expect acknowledgement within 48 hours and an initial assessment within
a week.

## Security stance

- Agents run as unprivileged users, execute updates via sudo allowlists, and connect outbound only.
- The scheduler never runs automatic updates; all actions require user confirmation.
- Secrets are encrypted at rest, never logged, and sensitive endpoints are rate limited.

## Detailed guidance

- PKI, certificates, and revocation flows: [docs/security/pki-certificates.md](docs/security/pki-certificates.md)
- Authentication and permissions: [docs/security/auth-and-authorization.md](docs/security/auth-and-authorization.md)
- Secrets, encryption, and master key handling: [docs/security/secrets-and-encryption.md](docs/security/secrets-and-encryption.md)
- Reverse proxy security: [docs/security/reverse-proxy-security.md](docs/security/reverse-proxy-security.md)
- TOFU & TLS hardening: [docs/security/tofu-tls.md](docs/security/tofu-tls.md)
- Filesystem/dependency hardening: [docs/security/filesystem-dependency-security.md](docs/security/filesystem-dependency-security.md)
- Secure development practices: [docs/security/secure-development.md](docs/security/secure-development.md)

## Disclosure policy

We follow coordinated disclosure and release fixes promptly after verification. Reporters receive credit in release notes unless anonymity is
requested.
