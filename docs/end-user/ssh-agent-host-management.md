# SSH Agent Host Management

The `uptrakit-agent-ssh` binary includes CLI subcommands for managing SSH host
entries in the local database. These commands operate independently of the
controller and do not require a WebSocket connection.

## Prerequisites

Host subcommands require a master encryption key for encrypting SSH private keys at rest. Provide one of:

- `--master-key-file /path/to/key` -- file containing a 64-character hex string
- `UPTRAKIT_MASTER_KEY` environment variable -- 64-character hex string
- `--allow-plaintext-secrets` -- development mode only (stores keys unencrypted)

For details on encryption and threat model, see [SSH Agent Secrets](../security/ssh-agent-secrets.md).

## Subcommands

### Add a host

```bash
uptrakit-agent-ssh host add \
  --name my-server \
  --hostname 192.168.1.100 \
  --username root \
  --private-key-file ~/.ssh/id_ed25519 \
  --master-key-file /etc/uptrakit/master.key
```

Optional flags:

| Flag | Default | Description |
| --- | --- | --- |
| `--port` | 22 | SSH port |
| `--host-key-fingerprint` | (none) | Expected host key fingerprint (SHA-256) |
| `--state-dir` | platform default | Override the state directory |

The private key type (Ed25519, RSA, or ECDSA) is auto-detected from the file content. Use `-` as the file path to read the key from stdin.

Host names must be unique. Adding a host with a duplicate name returns an error.

### List hosts

```bash
uptrakit-agent-ssh host list --master-key-file /etc/uptrakit/master.key
```

Outputs a table with ID, name, hostname, port, username, key type, and sudo policy for all registered hosts.

### Show host details

```bash
uptrakit-agent-ssh host show my-server --master-key-file /etc/uptrakit/master.key
```

Accepts either the host name or UUID. Displays all fields including timestamps. The private key is
always shown as `***REDACTED***`. Sudo state fields (`sudo_policy`, `is_root`, `sudo_available`) are
included in the output.

### Update a host

```bash
uptrakit-agent-ssh host update my-server \
  --port 2222 \
  --host-key-fingerprint "SHA256:abc123..." \
  --master-key-file /etc/uptrakit/master.key
```

All fields are optional — only specified fields are updated. The host is identified by name or UUID.

When renaming a host (`--name new-name`), the new name must not conflict with an existing host.

The `--sudo-policy` flag overrides the stored sudo execution policy for this host:

| Value | Description |
| --- | --- |
| `auto` (default) | Prepend `sudo` when agent user is not root and `sudo_available` is true |
| `force-with` | Always prepend `sudo` (unless agent user is root) |
| `force-without` | Never prepend `sudo` |

Example:

```bash
uptrakit-agent-ssh host update my-server \
  --sudo-policy force-with \
  --master-key-file /etc/uptrakit/master.key
```

See [Sudoers Management](../security/sudoers-management.md) for the full policy reference.

### Remove a host

```bash
uptrakit-agent-ssh host remove my-server --master-key-file /etc/uptrakit/master.key
```

Accepts either the host name or UUID. Returns an error if the host is not found.

### Bootstrap a host

For automated remote host setup (user creation, key deployment, sudoers), use
the `host bootstrap` command with a positional target in standard SSH format:

```bash
uptrakit-agent-ssh host bootstrap root@192.168.1.100 \
  --auth-password \
  --master-key-file /etc/uptrakit/master.key
```

The target accepts `[user@]host[:port]` or `ssh://[user@]host[:port]` format.
The host name defaults to the target hostname (overridable with `--name`).
Username, port, and hostname defaults are resolved from `~/.ssh/config` when not
specified in the target.

This connects to the remote host, creates a target user, deploys an SSH key,
configures sudoers, verifies connectivity, and saves the host entry.

For detailed options and troubleshooting, see
[SSH Agent Bootstrap](ssh-agent-bootstrap.md).

### Refresh sudoers for a host

The `host update-sudoers` command regenerates the sudoers drop-in file for an
already-enrolled host. Use this after enabling new plugins or when the
installed command paths on the remote host have changed.

```bash
uptrakit-agent-ssh host update-sudoers my-server \
  --master-key-file /etc/uptrakit/master.key
```

The command:

1. Connects to the remote host using the stored credentials.
2. Detects the agent user's privilege context (`id -u`, `sudo -n true`).
3. Resolves each registered plugin command to its absolute path via `command -v`.
4. Writes a minimal `/etc/sudoers.d/uptrakit-<username>` with one entry per
   resolved command. Validates with `visudo -cf`.
5. Persists the detected sudo state to the database.

Optional flags:

| Flag | Description |
| --- | --- |
| `--allow-all` | Write `NOPASSWD: ALL` instead of specific entries (less secure) |
| `--dry-run` | Preview the sudoers file without writing it to the remote host |

Example (dry-run):

```bash
uptrakit-agent-ssh host update-sudoers my-server \
  --dry-run \
  --master-key-file /etc/uptrakit/master.key
```

For the security model, see [Sudoers Management](../security/sudoers-management.md).

## Supported Key Types

| Key Type | PEM Format | Notes |
| --- | --- | --- |
| Ed25519 | OpenSSH or PKCS#8 | Preferred for new deployments |
| RSA | PKCS#1, OpenSSH, or PKCS#8 | Widely supported |
| ECDSA | SEC1, OpenSSH, or PKCS#8 | Elliptic curve (P-256, P-384) |

The key type is detected automatically from the PEM header and binary content. Unsupported formats are rejected with an error.

## Related Documentation

- [SSH Agent Bootstrap](ssh-agent-bootstrap.md) — automated remote host setup
- [SSH Agent Architecture](../architecture/ssh-agent.md) — architecture, database schema, and crate structure
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — encryption model and threat model
- [Sudoers Management](../security/sudoers-management.md) — sudoers generation, sudo policy, and operator guidance
- [Service Lifecycle](../development/service-lifecycle.md) — `ServiceHandler` trait used by daemon mode
