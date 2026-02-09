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
- **Rate limiting.** All public authentication endpoints and WebSocket connections are rate-limited per IP via a database-backed sliding-window counter. The rate limiter uses atomic SQL upserts to prevent TOCTOU bypasses in HA deployments. WebSocket rate limiting fails closed (rejects on DB error) to prevent bypass under database pressure.
- **No secrets in logs.** Tokens, passwords, API keys, and other credentials are never logged.

## Cryptographic Details

| Component | Implementation | Purpose |
| --- | --- | --- |
| TLS library | [Rustls](https://github.com/rustls/rustls) with [aws-lc-rs](https://github.com/aws/aws-lc-rs) backend | All controller HTTPS and agent WebSocket connections |
| CA key algorithm | ECDSA P-256 | Internal CA and all issued certificates |
| Certificate hashing | SHA-256 | Certificate signing, CRL generation, and OCSP response signing |
| Password hashing | [Argon2id](https://crates.io/crates/argon2) (OWASP-recommended: 19 MiB, 2 iterations) | User password storage |
| JWT signing | [jsonwebtoken](https://crates.io/crates/jsonwebtoken) | Access and refresh tokens |
| Session tokens | SHA-256 hashed, 7-day expiry, rotated on each use | Stateful user sessions (refresh token rotation) |
| Encryption at rest | AES-256-GCM ([aes-gcm](https://crates.io/crates/aes-gcm) crate) | Encrypts sensitive DB fields (MQTT passwords, OIDC client secrets, CA private keys) |
| TOFU verification | `TofuVerifier` with SHA-256 fingerprint pinning | Secures initial CA certificate trust on first use |

No custom cryptographic implementations are used.

## Certificate Lifecycle

Uptrakit operates an internal PKI for mTLS authentication of agents and MQTT services. The controller acts as the Certificate Authority.

| Asset | Default lifetime | Renewal window |
| --- | --- | --- |
| CA certificate | 5 years | 6 months before expiry (automatic rotation) |
| Server HTTPS certificate | 90 days | 30 days before expiry (automatic renewal) |
| Agent client certificate | 365 days (configurable) | Configurable via `renewal_window_hours` |
| MQTT service client certificate | 365 days (configurable) | Configurable via `renewal_window_hours` |

### CSR-based certificate issuance

Agent and MQTT service certificates are issued via a CSR (Certificate Signing Request) flow. The controller generates a UUIDv7 `service_id` during enrollment. After approval, the agent/service generates an ECDSA P-256 keypair locally and creates a PKCS#10 CSR with CN set to its `service_id`. The controller validates the CSR signature, verifies the CN matches the authenticated identity, and signs the certificate with controller-controlled parameters (DN, EKU, validity period). **The private key never leaves the agent/service.** A fresh keypair is generated for each CSR, including both initial enrollment and certificate renewals. MQTT services follow the same TOFU CA pinning, enrollment, and certificate issuance flow as agents, connecting to the same WebSocket endpoint (`/api/v1/ws/service`). MQTT enrollment tokens are settings-based (stored under a separate key: `mqtt_enrollment.token_hash`) and managed through the unified services REST API (`/api/v1/services/enrollment-token?type=mqtt`).

CA rotation preserves the full CA history in the database. All non-expired CAs form the trust bundle so agents signed by historical CAs remain trusted during the transition period. CRLs are partitioned per CA -- each CA signs a CRL only for certificates it issued. Combined PEM CRLs are publicly available at `GET /api/v1/pki/ca.crl`.

An OCSP responder is available at `POST /api/v1/pki/ocsp` (and `GET /api/v1/pki/ocsp/{base64}`), providing real-time certificate revocation status per RFC 6960. The responder supports both SHA-1 and SHA-256 hash algorithms in requests (Nginx/OpenSSL uses SHA-1). `ResponderID::ByKey` uses SHA-1 as required by RFC 6960 Section 2.3. OCSP responses are signed with the matching CA's private key using ECDSA P-256 SHA-256.

When `--pki-addr` is configured, certificates embed Authority Information Access (AIA) and CRL Distribution Points (CDP) extensions. These extensions point reverse proxies to the OCSP responder, CA certificate download, and CRL endpoints. At startup, the controller validates that an existing managed CA's embedded URLs match the reconciled `--pki-addr` — mismatches cause a hard startup failure with actionable error messages.

At startup, the controller validates that `--san` CLI values are present in the managed server certificate's SANs. Mismatches trigger automatic regeneration when the cert was signed by the active CA, or a guided error when the cert was signed by a previous CA (preventing accidental re-signing under a rotated CA).

For the full operational flow (rotation steps, bundle distribution, `CaSnapshot` sharing), see [AGENTS.md](AGENTS.md) section "PKI & CA rotation".

## Authentication Methods

| Method | Scope | Details |
| --- | --- | --- |
| Password (Argon2id) | User login | Local accounts with hashed passwords |
| OIDC | User login | External identity providers; auto-create or link accounts |
| JWT access tokens | API requests | Short-lived; carry resolved permissions (not roles); held in-memory only (never persisted to localStorage) |
| Device authorization | CLI login | Short-lived user code + browser approval; results in API token |
| API tokens | Programmatic access | Long-lived bearer tokens; revocable |
| mTLS client certificates | Agent and MQTT service connections | Issued during enrollment; validated on every WebSocket connection |
| Forwarded client certificates | Agent connections via reverse proxy | Trusted proxy forwards cert info/PEM headers; issuer CN verified against known CAs |
| Enrollment tokens | Agent registration | Settings-based one-time tokens with expiry for initial agent enrollment |
| MQTT enrollment tokens | MQTT service registration | Settings-based tokens (separate key: `mqtt_enrollment.token_hash`) managed through the unified services API; one-time with optional expiry and use limits |

Authorization is permission-based (typed `Permission` enum), not role-string-based. See [AGENTS.md](AGENTS.md) section "Permissions model" for the full RBAC design.

## Reverse Proxy Security

When the controller is behind a reverse proxy, agent identity is extracted from forwarded headers. Security measures:

- **Trusted proxies required**: Only requests from IP addresses listed in `--trusted-proxy` / `network.trusted_proxies` are trusted for forwarded headers. Requests from untrusted sources have all cert-related and proxy headers stripped.
- **CA CN verification**: The issuer CN in forwarded certificates is verified against known CA common names (active CA and non-expired previous CA). Mismatched issuers are rejected.
- **Header stripping**: `X-Forwarded-Proto`, `X-Forwarded-Host`, `Origin`, `X-Tenant-Id`, and configured cert headers are removed from non-proxy requests to prevent spoofing.
- **PEM and info header support**: Both structured info headers (Traefik, Nginx, HAProxy) and raw PEM headers (Caddy, Envoy) are supported, with info preferred when both are available.

For deployment guides, see [docs/reverse-proxy/](docs/reverse-proxy/).

## Secrets Handling

- Sensitive database credentials (MQTT passwords, OIDC client secrets, CA private keys) are encrypted at rest using AES-256-GCM. See the "Encryption at Rest" section below.
- Passwords are hashed with Argon2id before storage; plaintext is never persisted.
- Session tokens (refresh tokens) are SHA-256 hashed in the database and **rotated on each use** — the old token is atomically revoked and a new one issued, preventing replay attacks.
- JWT signing keys are held in memory only.
- Refresh tokens are stored in `HttpOnly; Secure; SameSite=Strict` cookies scoped to `/api/v1/auth`, preventing JavaScript access (XSS-resistant). Access tokens are held in-memory only and never written to localStorage or sessionStorage.
- Agent and MQTT service private keys are generated and stored locally on each agent/service; they never leave the host.
- CA private keys are stored on the controller filesystem and held in a separate in-memory key store (`CaKeyStore`) with `zeroize` memory protection. Only signing operations (OCSP, CRL, certificate issuance) access the key store; public CA data (certificates, fingerprints) is shared separately.
- No secrets appear in log output, error messages, or API responses.
- MQTT credential-bearing messages (`TenantAssignments`, `TenantConfigUpdated`, `TenantRevoked`) are delivered locally only and never written to the cross-controller notification outbox, preventing plaintext credential persistence in the database.

## Encryption at Rest

Sensitive credentials stored in the database are encrypted using AES-256-GCM via the `EncryptedString` SeaORM custom type (`crates/shared/db/src/crypto.rs`). `EncryptedString` wraps `SecretString` (from `uptrakit-shared-types`) which redacts values in `Debug` and `Display` output to prevent accidental logging of secrets. `SecretString` is also used in wire protocol types for enrollment tokens and MQTT credentials.

### Master Key Management

A 256-bit master encryption key is mandatory for production use. For development only, the controller can start without a key by passing `--allow-plaintext-secrets`, which disables encryption at rest and logs a warning. If the flag is set while a key is provided, the controller logs a warning and continues with encryption enabled.

| Provisioning method | Details |
| --- | --- |
| `UPTRAKIT_MASTER_KEY` environment variable | 64-character hex string (32 bytes) |
| `--master-key-file` CLI argument | Path to a file containing the hex key |

The master key is loaded once at startup via `init_master_key()` and held in a global `OnceLock`. It is never logged, never exposed in API responses, and never persisted by the application itself.

### Encrypted Fields

| Table | Column | Description |
| --- | --- | --- |
| `mqtt_clients` | `password` | MQTT broker password |
| `oidc_providers` | `client_secret` | OIDC client secret |
| `ca_certificates` | `key_pem` | CA private key PEM |

### Ciphertext Format

Encrypted values are stored as `"ENC:v1:<hex(nonce || ciphertext || tag)>"`. The `v1` marker enables future algorithm migration. A 96-bit random nonce is generated per encryption operation.

### Legacy Plaintext Passthrough

On read, `EncryptedString` detects values without the `ENC:v1:` prefix and returns them as-is. This allows existing plaintext data to be read transparently. On the next write, the value will be encrypted. No bulk migration is required.

## TOFU Hardening

The Trust-On-First-Use (TOFU) mechanism for initial CA certificate bootstrap has been hardened with proper TLS signature verification and optional fingerprint pinning.

### TofuVerifier

The previous `AcceptAnyCert` TLS verifier (which bypassed all certificate validation) has been replaced with `TofuVerifier` (`crates/shared/enrollment/src/tls.rs`). `TofuVerifier` delegates TLS signature verification to the installed cryptographic provider (aws-lc-rs via Rustls), ensuring that the server presents a properly structured certificate even during TOFU. It only skips the CA chain validation -- it does **not** disable signature checks.

The `CaTlsMode::Insecure` variant has been renamed to `CaTlsMode::Tofu` to accurately reflect its purpose.

### Fingerprint Pinning

The `--tofu-fingerprint` CLI flag (requires `--tofu`) accepts a SHA-256 fingerprint (hex, with or without colons). When provided, `bootstrap_ca()` computes the fingerprint of the fetched CA certificate via `ca_pem_fingerprint()` and compares it to the expected value before trusting the certificate. A mismatch causes a hard failure.

This allows operators to verify the identity of the controller on the very first connection, closing the TOFU trust gap for environments where the fingerprint can be distributed out-of-band.

### CLI `--insecure` Flag

The CLI client (`crates/ui/cli/src/client.rs`) previously hardcoded `tls_danger_accept_invalid_certs(true)`, silently bypassing all TLS verification. This has been removed. An explicit `--insecure` flag is now required to skip TLS certificate verification, making the security trade-off visible to the operator.

## Filesystem Security

All sensitive files and directories are created with secure permissions:

| Type | Permission | Octal | Description |
| --- | --- | --- | --- |
| Directories | Owner rwx only | 0o700 | Config and state directories |
| Files | Owner rw only | 0o600 | Private keys, certificates, database, configuration |

The `uptrakit-directories` crate provides helper functions (`create_secure_dir`, `write_secure_file`) that enforce these permissions on Unix systems. This applies to:
- Controller: external CA / TLS cert files (if configured), database files (including managed CA history), JWT key files
- Agent/MQTT Service: State directories, private keys, certificates, CA certificate

## Dependency Security

- **[cargo-deny](https://github.com/EmbarkStudios/cargo-deny)**: Runs in CI to check for known vulnerabilities (RustSec advisory database), license compliance, and dependency issues.
- **[Dependabot](https://docs.github.com/en/code-security/dependabot)**: Monitors Cargo, npm, and GitHub Actions dependencies weekly with automatic pull requests for updates.
- Dependencies affecting command execution, untrusted input parsing, cryptography, or networking receive extra scrutiny during review.

## Disclosure Policy

- We follow coordinated disclosure practices.
- Security fixes will be released as soon as practical after verification.
- Reporters will be credited in release notes (unless they prefer anonymity).
