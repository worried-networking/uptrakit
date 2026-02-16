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

Outputs a table with ID, name, hostname, port, username, and key type for all registered hosts.

### Show host details

```bash
uptrakit-agent-ssh host show my-server --master-key-file /etc/uptrakit/master.key
```

Accepts either the host name or UUID. Displays all fields including timestamps. The private key is always shown as `***REDACTED***`.

### Update a host

```bash
uptrakit-agent-ssh host update my-server \
  --port 2222 \
  --host-key-fingerprint "SHA256:abc123..." \
  --master-key-file /etc/uptrakit/master.key
```

All fields are optional -- only specified fields are updated. The host is identified by name or UUID.

When renaming a host (`--name new-name`), the new name must not conflict with an existing host.

### Remove a host

```bash
uptrakit-agent-ssh host remove my-server --master-key-file /etc/uptrakit/master.key
```

Accepts either the host name or UUID. Returns an error if the host is not found.

## Supported Key Types

| Key Type | PEM Format | Notes |
| --- | --- | --- |
| Ed25519 | OpenSSH or PKCS#8 | Preferred for new deployments |
| RSA | PKCS#1, OpenSSH, or PKCS#8 | Widely supported |
| ECDSA | SEC1, OpenSSH, or PKCS#8 | Elliptic curve (P-256, P-384) |

The key type is detected automatically from the PEM header and binary content. Unsupported formats are rejected with an error.

## Related Documentation

- [SSH Agent Architecture](../architecture/ssh-agent.md) -- architecture, database schema, and crate structure
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) -- encryption model and threat model
- [Service Lifecycle](../development/service-lifecycle.md) -- `ServiceHandler` trait used by daemon mode
