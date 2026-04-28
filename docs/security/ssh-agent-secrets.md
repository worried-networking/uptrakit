---
title: SSH Agent Secret Storage
weight: 100
description: How the SSH-backed agent stores and protects SSH credentials using independent envelope encryption and AES-256-GCM.
---

# SSH Agent Secret Storage

This document describes how the SSH-backed agent (`uptrakit-agent-ssh`) stores and protects SSH credentials.

## Encryption Model

The SSH agent uses the same encryption infrastructure as the controller
(`uptrakit-crypto`), but with an **independent master key** and its own local
`data_encryption_keys` table.

### Key Hierarchy

```text
SSH Agent Master Key (KEK, 32 bytes, provided by operator)
  └── wraps → DEK (32 bytes, stored encrypted in local data_encryption_keys table)
                └── AES-256-GCM (via aws-lc-rs)
                    └── SSH private keys in local SQLite (EncryptedString)
```

### Storage Format

SSH private keys are stored in the `ssh_hosts.private_key` column as `EncryptedString` values. The current format is:

```text
ENC:v3:<key_id>:<hex(nonce || ciphertext || tag)>
```

- **key_id**: First 8 hex chars of SHA-256 of the DEK (identifies which DEK encrypted the value)
- **Nonce**: 12 bytes, randomly generated per encryption (unique per value)
- **Ciphertext**: AES-256-GCM encrypted plaintext with AAD `"uptrakit:ssh_hosts:private_key"`
- **Tag**: 16-byte authentication tag

Legacy formats (`ENC:v1:`, `ENC:v2:`) are automatically upgraded to `ENC:v3:` on startup.

When no master key is configured (development mode with `--allow-plaintext-secrets`), the private key is stored as plaintext. This mode logs a warning
and must never be used in production.

## Master Key Management

### Providing the Master Key

The master key can be provided via:

1. **File**: `--master-key-file /path/to/key` — a file containing a 64-character hex string
1. **Environment**: `UPTRAKIT_MASTER_KEY=<64-char-hex>`

The key must be exactly 32 bytes (64 hex characters). The SSH agent refuses to start without a master key unless `--allow-plaintext-secrets` is
passed.

### Key Independence

The SSH agent's master key is completely independent of the controller's master key. They:

- Are generated separately by their respective operators
- Are stored on different machines
- Encrypt different data (SSH keys vs. CA keys / OIDC secrets)
- Have no cryptographic relationship

This design ensures that compromise of the controller's master key does not expose SSH credentials, and vice versa.

## Threat Model

### Protected Against

| Threat | Mitigation |
| --- | --- |
| Database file theft | AES-256-GCM encryption with per-value random nonces |
| Memory dump of DB values | Encrypted at rest; plaintext only in application memory during use |
| Key reuse attacks | Random 12-byte nonce per encryption; same plaintext produces different ciphertext |
| Tampering | GCM authentication tag detects any modification |
| Controller compromise | Independent master key; controller never sees SSH private keys |

### Not Protected Against (Requires Additional Controls)

| Threat | Required Control |
| --- | --- |
| Master key file theft | OS-level file permissions (0o600), restrict access to service user |
| Root access on SSH agent host | Full-disk encryption, host hardening, access controls |
| Memory dump of running process | Process isolation, no core dumps in production |

## File Permissions

The SSH agent follows the same secure file permission model as other Uptrakit services:

- **State directory**: 0o700 (owner read/write/execute only)
- **Database file**: Created within the state directory, inheriting secure permissions
- **Master key file**: Should be 0o600, owned by the service user

See [Filesystem and Dependency Security](filesystem-dependency-security.md) for details on the `uptrakit-directories` crate's secure file operations.

## Development Mode

For development and testing, pass `--allow-plaintext-secrets` to disable encryption:

```bash
uptrakit-agent-ssh --url https://controller:8443 --allow-plaintext-secrets
```

This stores SSH private keys as plaintext in the database and logs a warning. It must **never** be used in production.

## Bootstrap Security Model

The bootstrap operation introduces additional security considerations.

### Transient credentials

All credential fields in `BootstrapParams` and `SyncParams`
(`auth_password`, `auth_private_key_pem`, `target_private_key_pem`) are typed as
`Option<SecretString>` rather than `Option<String>`. `SecretString` prevents these values from
appearing in `Debug` output, log messages, or panic traces. Use `.expose_secret()` only at the
point where the raw value is needed (e.g., passing to the SSH transport layer).

- **Auth passwords** are held only in process memory for the duration of the
  bootstrap. They are never written to disk or stored in the database. When
  submitted via the web UI, passwords are encrypted end-to-end via ECIES and
  decrypted only on the SSH agent.
- **SSH agent keys** are used transiently via the `SSH_AUTH_SOCK` Unix socket.
  Private key material never leaves the SSH agent process — the bootstrap
  command only receives signatures. The agent connection is dropped when
  authentication completes. See
  [Secure Development](secure-development.md) for related guidance.
- **Generated Ed25519 keys** exist only in process memory until they are
  encrypted with the master key and stored in the database. No key file is
  written to disk.

### Host key verification

The `host add` and bootstrap operations support two host key verification
modes, controlled by the `--strict-host-key-checking` flag:

| Mode | Flags | Security Level |
| --- | --- | --- |
| **Strict** | `--strict-host-key-checking --host-key-fingerprint SHA256:...` | Strong — rejects mismatched keys; TOFU disabled |
| **TOFU** (trust-on-first-use) | (neither flag, or `--host-key-fingerprint` alone) | Weaker — accepts any key on first connection |

When `--strict-host-key-checking` is set, `--host-key-fingerprint` becomes
**required**. The command exits with an error if the fingerprint is not provided:

```text
error: --strict-host-key-checking requires --host-key-fingerprint to be provided
```

Without `--strict-host-key-checking`, TOFU mode is used by default. In TOFU
mode, the SSH agent accepts and records the remote host's key on first
connection. A `tracing::info!` log message is emitted when a key is accepted
via TOFU:

```text
INFO accepting host key via trust-on-first-use (TOFU), fingerprint: SHA256:...
```

The observed fingerprint is displayed to the operator and stored in the database
for subsequent connections. The verification step (step 7 in the bootstrap
workflow) always uses strict pinning with the fingerprint from step 3.

**Recommendation**: Use `--strict-host-key-checking` with
`--host-key-fingerprint` in production and high-security deployments where host
keys should be pre-verified out-of-band. This prevents man-in-the-middle attacks
during the initial connection. TOFU is acceptable for development and trusted
networks.

### Shell injection prevention

All remote commands are constructed using `uptrakit_command::shell_escape()`,
which wraps arguments in single quotes with proper escaping. Username validation
enforces POSIX rules (`[a-z_][a-z0-9_-]*`, max 32 characters), further reducing
injection risk.

### Hostname validation

The `SshTarget` parser validates hostnames at parse time (not just at connection
time). Validation rejects:

- Whitespace and control characters
- Hostnames exceeding 253 characters (DNS limit)
- DNS labels exceeding 63 characters
- Labels starting or ending with hyphens
- Invalid characters in DNS labels (only alphanumeric, hyphens, and underscores
  are allowed)

IPv4 and IPv6 addresses pass through without DNS label validation. This prevents
malformed or malicious hostname strings from reaching the SSH transport layer.

### Authorized keys hardening

The public key deployed to `~target/.ssh/authorized_keys` is prefixed with SSH
restrictions:

```text
no-pty,no-agent-forwarding,no-X11-forwarding <key>
```

These prevent interactive terminal allocation (`no-pty`), SSH agent forwarding
(`no-agent-forwarding`), and X11 forwarding (`no-X11-forwarding`) through the
managed account. Non-interactive command execution (what the agent uses) remains
fully functional.

The target user is created with `/bin/sh` as its login shell (not `/bin/bash`),
further limiting the attack surface for a service account that only needs
non-interactive command execution.

### Sudoers configuration

The bootstrap operation writes `/etc/sudoers.d/uptrakit-<target_username>` with
minimal per-command entries derived from registered plugins. For example:

```text
# Managed by Uptrakit - DO NOT EDIT MANUALLY
# Regenerate: uptrakit surfaces ssh-agent.hosts --target-provider-id <PROVIDER_ID> sync-host
# /usr/bin/apt-get: Package installation and index refresh require root privileges
uptrakit ALL=(root) NOPASSWD: /usr/bin/apt-get
```

This grants the target user passwordless sudo access only for the specific
absolute paths required by the registered plugins — not unrestricted root
access.

The `--allow-all` flag writes `NOPASSWD: ALL` instead (legacy behavior; less
secure). Avoid using `--allow-all` in production.

Refresh the sudoers file after adding new plugins using the **Sync Host** action
in the web UI or by running:

```bash
uptrakit surfaces ssh-agent.hosts --target-provider-id <PROVIDER_ID> sync-host <host-id>
```

See [Sudoers Management](sudoers-management.md) for the full security model,
sudo policy configuration, and operator guidance.

## SSH Key Type Auto-Detection

When a private key is provided via `--private-key-file`, the SSH agent
automatically detects the key algorithm from the PEM content. Supported key
types are **Ed25519** (preferred), **RSA**, and **ECDSA**. Detection works for:

- **PKCS#1** (`BEGIN RSA PRIVATE KEY`) — RSA
- **SEC1** (`BEGIN EC PRIVATE KEY`) — ECDSA
- **OpenSSH** (`BEGIN OPENSSH PRIVATE KEY`) — algorithm extracted from the binary payload
- **PKCS#8** (`BEGIN PRIVATE KEY`) — algorithm determined by OID inspection

The detected key type is stored alongside the encrypted private key and displayed in host listings. Unrecognized key formats are rejected with an error.

For details on how the CLI host management subcommands work, see [SSH Agent Host Management](../end-user/ssh-agent-host-management.md).

## SSH Session Lifecycle

The SSH agent maintains a **persistent connection pool** (`SshConnectionPool`) — one authenticated session per
enrolled host. Sessions are reused across `CheckVersions`, `ExecuteUpdate`, `DiscoverSoftware`, and `ReportHosts`
operations, eliminating repeated TCP+SSH handshakes.

### Pool behaviour

- **Session acquisition**: `pool.acquire(&host)` returns a cached `Arc<SshSession>` if the session was used within
  the last 300 seconds. An expired or absent session triggers a new TCP+SSH handshake.
- **Session eviction**: If a caller detects a connection-level error, it calls `pool.evict(&host_id)` to remove the
  stale entry. The next `acquire` for that host establishes a fresh connection.
- **Shutdown**: `pool.disconnect_all()` sends a clean SSH disconnect to every pooled session during service shutdown,
  avoiding silent socket drops on remote hosts.

### Key material exposure

The SSH private key is decrypted from the local `ssh_hosts` database **only when establishing a new session**.
Plaintext key material exists only in process memory during the TCP+SSH handshake and is never written to disk or
logged. Pooled sessions do not hold any plaintext key material — only the established cryptographic session state.

Docker plugin operations over SSH use a `StdioTunnel` proxy (see
[Command Executor — StdioTunnel](../development/command-executor.md#stdiotunnel)) that runs
`docker system dial-stdio` over the existing russh session. No temporary key files are created and no
second SSH connection is established — Docker API traffic is tunnelled through the already-authenticated
session, eliminating a previous plaintext key exposure vector.

### Tradeoff: persistent connections vs. minimal exposure window

The persistent pool increases the window during which a live SSH connection is present in process memory (sessions
stay alive up to 300 seconds after last use). In exchange, it eliminates repeated plaintext key decryption and
handshake overhead per operation. The key material exposure window is unchanged — keys are decrypted only on new
connections, not on pool reuse.

See [SSH Agent Architecture — Version Check and Update Execution](../architecture/ssh-agent.md#version-check-and-update-execution)
for the full dispatch flow.

## Related Documentation

- [SSH Agent Architecture](../architecture/ssh-agent.md) — overall architecture, database schema, CLI subcommands, and version check/update execution
- [SSH Agent Host Management](../end-user/ssh-agent-host-management.md) — end-user guide for managing SSH hosts
- [SSH Agent Bootstrap](../end-user/ssh-agent-bootstrap.md) — automated remote host setup and troubleshooting
- [Proxmox Bootstrap Privileges](../development/proxmox-bootstrap.md) — PVE privilege chain for guest bootstrap
- [Secrets and Encryption](secrets-and-encryption.md) — controller's encryption model
- [Cryptography](cryptography.md) — cryptographic primitives used across Uptrakit
- [Filesystem and Dependency Security](filesystem-dependency-security.md) — secure file operations
