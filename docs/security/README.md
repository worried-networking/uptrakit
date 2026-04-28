---
title: Overview
weight: 1
description: Security architecture, cryptography, PKI, authentication, secret handling, and secure deployment guidance for Uptrakit.
---

# Security Documentation

This folder contains security architecture, cryptography, PKI, authentication, secret handling, and secure deployment guidance for Uptrakit.

## Contents

| Document | Description |
| --- | --- |
| [Security Architecture](security-architecture.md) | Threat model and defense-in-depth principles. |
| [Cryptography](cryptography.md) | Cryptographic primitives, key handling, and protocol-level crypto details. |
| [PKI and Certificates](pki-certificates.md) | Managed CA lifecycle, certificate issuance, renewal, OCSP, and CRL behavior. |
| [Auth and Authorization](auth-and-authorization.md) | Authentication flows, role/permission model, and auth middleware behavior. |
| [Secrets and Encryption](secrets-and-encryption.md) | Encryption-at-rest, master key handling, and secret redaction conventions. |
| [TOFU and TLS](tofu-tls.md) | TOFU behavior and TLS trust bootstrap considerations. |
| [Filesystem and Dependency Security](filesystem-dependency-security.md) | Filesystem permissions, hardening defaults, and dependency safeguards. |
| [Secure Development](secure-development.md) | Secure coding expectations for contributors. |
| [Reverse Proxy Security](reverse-proxy-security.md) | Reverse proxy trust model, header validation, revocation strategy, and per-proxy guide links. |
| [SSH Agent Secrets](ssh-agent-secrets.md) | SSH credential encryption, master key management, bootstrap security, and TOFU vs pinned fingerprints. |
| [Sudoers Management](sudoers-management.md) | Per-command sudoers generation, sudo policy, detecting/persisting sudo state, and operator guidance. |
| [Notification Subsystem Security](notifications-security.md) | Secret storage, webhook HMAC signing, Telegram callback verification, action tokens, and tenant isolation. |

## Related Documentation

- Top-level docs catalogue: [`docs/README.md`](../README.md)
- End-user deployment guide: [`docs/end-user/deployment/README.md`](../end-user/deployment/README.md)
- API and protocol docs: [`docs/api/README.md`](../api/README.md)
- Development standards: [`docs/development/README.md`](../development/README.md)
