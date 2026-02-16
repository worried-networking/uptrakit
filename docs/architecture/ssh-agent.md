# SSH-Backed Agent Architecture

The SSH-backed agent (`uptrakit-agent-ssh`) is a service type that connects to the controller over WebSocket (like the regular agent) but will
execute version detection and updates on remote hosts over SSH instead of locally.

## Current Scope

The current implementation provides:

- A new `ServiceType::SshAgent` variant in the shared type system
- Controller-side enrollment and WebSocket dispatch for SSH agents
- A standalone binary (`uptrakit-agent-ssh`) with the `ServiceHandler` trait
- A local SQLite database for storing SSH host credentials (encrypted at rest)
- CLI subcommands for managing SSH host entries locally (`host add/list/show/update/remove`)

SSH transport, provider execution over SSH, and UI configuration beyond the existing services API are not yet implemented.

## Architecture Overview

```text
┌──────────────────┐         WSS (mTLS)        ┌────────────────────┐
│  uptrakit-agent-  │ ◄────────────────────────► │  uptrakit-         │
│  ssh              │                            │  controller        │
│                   │                            │                    │
│  ┌─────────────┐  │                            │  ┌──────────────┐  │
│  │ Local SQLite │  │                            │  │ Controller DB│  │
│  │ (ssh_hosts)  │  │                            │  │ (services)   │  │
│  └─────────────┘  │                            │  └──────────────┘  │
└──────────────────┘                            └────────────────────┘
```

The SSH agent follows the same enrollment and connection lifecycle as the regular agent and MQTT service, using the shared `ServiceHandler` trait from
`uptrakit-service-sdk`.

## Self-Managed Encryption

The SSH agent manages its own encryption key independently from the controller:

| Component | Master Key | Encrypts |
| --- | --- | --- |
| Controller | Controller's master key (`UPTRAKIT_MASTER_KEY`) | CA keys, OIDC secrets, MQTT passwords |
| SSH Agent | SSH agent's master key (`UPTRAKIT_MASTER_KEY`) | SSH private keys in local SQLite |

Both use the same `init_master_key()` function from `uptrakit-shared-db::crypto` and the same `EncryptedString` type (AES-256-GCM), but with
independent keys. The controller has no knowledge of the SSH agent's master key.

### Master Key Options

The SSH agent supports the same master key configuration as the controller:

- `--master-key-file <path>` — path to a file containing a 64-character hex string
- `UPTRAKIT_MASTER_KEY` environment variable — 64-character hex string
- `--allow-plaintext-secrets` — development mode (disables encryption, logs a warning)

## Local Database Schema

The SSH agent uses a local SQLite database (`agent-ssh.db` in the state directory) with the following table:

### `ssh_hosts`

| Column | Type | Description |
| --- | --- | --- |
| `id` | TEXT (UUID) | Primary key |
| `name` | TEXT | Friendly name for the host (UNIQUE) |
| `hostname` | TEXT | SSH hostname or IP address |
| `port` | INTEGER | SSH port (default: 22) |
| `username` | TEXT | SSH username |
| `private_key` | TEXT | `EncryptedString` — SSH private key (AES-256-GCM) |
| `key_type` | TEXT | Key algorithm: `ed25519`, `rsa`, or `ecdsa` |
| `host_key_fingerprint` | TEXT | Known host key (SHA-256), nullable |
| `created_at` | INTEGER | Unix timestamp |
| `updated_at` | INTEGER | Unix timestamp |

The `name` column has a UNIQUE index to prevent duplicate host names.

## Service Type Registration

The `SshAgent` variant is registered in:

- `ServiceType` enum (`crates/shared/types/src/service_type.rs`) — serializes as `"ssh_agent"`
- `SettingKey::SshAgentEnrollmentTokenHash` — per-tenant enrollment token setting
- Controller dispatch (`crates/ui/web-api/src/routes/service_ws.rs`) — routes to `ssh_agent_ws`
- Connection registry (`crates/ui/web-api/src/service_connections.rs`) — `register_ssh_agent()`
- Services API (`crates/ui/web-api/src/routes/services.rs`) — enrollment token key resolution
- AsyncAPI spec (`crates/shared/wire/asyncapi.yaml`) — `ssh_agent` in ServiceType enum

## CLI Host Management

The SSH agent binary includes subcommands for managing SSH host entries locally,
without requiring a connection to the controller. These subcommands operate
directly on the local SQLite database.

### Subcommands

| Command | Description |
| --- | --- |
| `host add` | Register a new SSH host with connection details and private key |
| `host list` | List all registered SSH hosts in tabular format |
| `host show <name_or_id>` | Display detailed information for a specific host |
| `host update <name_or_id>` | Update one or more fields of an existing host |
| `host remove <name_or_id>` | Remove an SSH host from the local database |

Host identification accepts either the host's friendly name or UUID. The code tries UUID parse first, then falls back to a name lookup.

### SSH Key Type Auto-Detection

When adding or updating a host, the `--private-key-file` argument accepts a path
to a PEM-encoded private key file (or `-` for stdin). The key type is
automatically detected from the file content:

| PEM Header | Detected Type |
| --- | --- |
| `BEGIN RSA PRIVATE KEY` | RSA (PKCS#1) |
| `BEGIN EC PRIVATE KEY` | ECDSA (SEC1) |
| `BEGIN OPENSSH PRIVATE KEY` | Decoded from OpenSSH binary format (Ed25519, RSA, or ECDSA) |
| `BEGIN PRIVATE KEY` | Decoded from PKCS#8 format via OID inspection |

The detected key type is stored in the `key_type` column and displayed in host listings.

### Master Key Requirement

Host subcommands require the same master key configuration as daemon mode
(`--master-key-file` or `UPTRAKIT_MASTER_KEY`) because private keys are
encrypted at rest. Pass `--allow-plaintext-secrets` for development without
encryption.

For detailed usage instructions, see [SSH Agent Host Management](../end-user/ssh-agent-host-management.md).

## Crate Structure

```text
crates/core/agent-ssh/
├── Cargo.toml
├── build.rs
└── src/
    ├── main.rs          # SshAgentHandler (ServiceHandler impl), entry point, master key init
    ├── cli.rs           # CLI args (Commands, HostCommands, CommonServiceArgs integration)
    ├── client.rs        # Authenticated loop (ping/pong, cert renewal, local DB init)
    ├── error.rs         # Error types (rootcause + thiserror)
    ├── ssh_key.rs       # SSH private key reading and key type auto-detection
    ├── host_ops.rs      # CRUD operations for SSH hosts (add, find, list, update, remove)
    ├── commands/
    │   ├── mod.rs       # Command module declarations
    │   └── host.rs      # Host subcommand handlers (dispatch, formatting, output)
    └── db/
        ├── mod.rs       # SQLite init (init_db) + tests
        ├── entity/
        │   ├── mod.rs   # Entity module declarations
        │   └── ssh_host.rs  # SeaORM entity (Model, SshKeyType enum with FromStr/Display)
        └── migration/
            ├── mod.rs   # Migration runner
            └── m20260215_000001_initial.rs  # ssh_hosts table (with UNIQUE index on name)
```

## Related Documentation

- [SSH Agent Host Management](../end-user/ssh-agent-host-management.md) — end-user guide for CLI host management
- [Service Lifecycle](../development/service-lifecycle.md) — `ServiceHandler` trait
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — secret storage and threat model
- [Wire Protocol](../api/wire-protocol.md) — `SshAgent` service type in enrollment
- [Services and Operations](../api/services-operations.md) — shared service management API
