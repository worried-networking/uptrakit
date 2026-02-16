# SSH Agent Secret Storage

This document describes how the SSH-backed agent (`uptrakit-agent-ssh`) stores and protects SSH credentials.

## Encryption Model

The SSH agent uses the same encryption infrastructure as the controller (`uptrakit-shared-db::crypto`), but with an **independent master key**.

### Key Hierarchy

```text
SSH Agent Master Key (32 bytes, provided by operator)
  └── AES-256-GCM (via aws-lc-rs)
      └── SSH private keys in local SQLite (EncryptedString)
```

### Storage Format

SSH private keys are stored in the `ssh_hosts.private_key` column as `EncryptedString` values. When encrypted, the stored format is:

```text
ENC:v1:<hex(nonce || ciphertext || tag)>
```

- **Nonce**: 12 bytes, randomly generated per encryption (unique per value)
- **Ciphertext**: AES-256-GCM encrypted plaintext
- **Tag**: 16-byte authentication tag

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

| Threat | Mitigation | |---|---| | Database file theft | AES-256-GCM encryption with per-value random nonces | | Memory dump of DB values | Encrypted
at rest; plaintext only in application memory during use | | Key reuse attacks | Random 12-byte nonce per encryption; same plaintext produces
different ciphertext | | Tampering | GCM authentication tag detects any modification | | Controller compromise | Independent master key; controller
never sees SSH private keys |

### Not Protected Against (Requires Additional Controls)

| Threat | Required Control | |---|---| | Master key file theft | OS-level file permissions (0o600), restrict access to service user | | Root access
on SSH agent host | Full-disk encryption, host hardening, access controls | | Memory dump of running process | Process isolation, no core dumps in
production |

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

## Related Documentation

- [SSH Agent Architecture](../architecture/ssh-agent.md) — overall architecture and database schema
- [Secrets and Encryption](secrets-and-encryption.md) — controller's encryption model
- [Cryptography](cryptography.md) — cryptographic primitives used across Uptrakit
- [Filesystem and Dependency Security](filesystem-dependency-security.md) — secure file operations
