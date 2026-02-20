# SSH-Backed Agent Architecture

The SSH-backed agent (`uptrakit-agent-ssh`) is a service type that connects to the controller over WebSocket (like the regular agent) but will
execute version detection and updates on remote hosts over SSH instead of locally.

## Current Scope

The current implementation provides:

- A new `ServiceType::SshAgent` variant in the shared type system
- Controller-side enrollment and WebSocket dispatch for SSH agents
- A standalone binary (`uptrakit-agent-ssh`) with the `ServiceHandler` trait
- A local SQLite database for storing SSH host credentials (encrypted at rest)
- CLI subcommands for managing SSH host entries locally (`host add/list/show/update/remove/bootstrap`)
- SSH transport layer (`russh`) for the bootstrap workflow (connect, authenticate, execute remote commands)
- Ed25519 keypair generation for automated key deployment
- A `host bootstrap` command that automates remote host setup (user creation, key deployment, sudoers configuration)
- Host reporting via `ReportHosts` — on authenticated connect, the SSH agent collects system info
  from each enrolled host over SSH and reports it to the controller

UI configuration beyond the existing services API is not yet implemented.

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

Both use the same `init_master_key()` function from `uptrakit-crypto` and the same `EncryptedString` type (AES-256-GCM), but with
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
| `host bootstrap` | Automate remote host setup and save the host entry (see [Bootstrap](#bootstrap-workflow)) |

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

### Bootstrap Workflow

The `host bootstrap` subcommand automates the full remote host setup in a single
command. It accepts a positional target in standard SSH format
(`[user@]host[:port]` or `ssh://[user@]host[:port]`) and resolves defaults from
`~/.ssh/config`.

```text
1. PARSE TARGET & RESOLVE DEFAULTS (target string → SSH config → $USER/port 22)
2. VALIDATE INPUTS (username format, no DB name conflict)
3. PREPARE KEY MATERIAL (read provided key or generate Ed25519)
4. CONNECT & AUTHENTICATE (password, key file, or SSH agent; TOFU or pinned host key)
5. DETECT PRIVILEGES (root check, sudo -n true)
6. REMOTE SETUP (create user with /bin/sh shell, deploy authorized_keys with
   no-pty/no-agent-forwarding/no-X11-forwarding restrictions, write sudoers)
7. DISCONNECT auth session
8. VERIFY (reconnect as target user, whoami + sudo -n true)
9. SAVE TO DATABASE (encrypt key, store host entry)
```

The resolution chain for each field:

- **Username**: target string → `~/.ssh/config` `User` → `$USER`
- **Port**: target string → `~/.ssh/config` `Port` → 22
- **Hostname**: `~/.ssh/config` `HostName` → target hostname
- **Host name** (`--name`): explicit flag → target hostname (before `HostName`
  resolution)

When the auth username is `root` and `--target-username` is omitted, the target
username defaults to `uptrakit` (instead of reusing `root`) to ensure the
managed account is a dedicated service user.

The bootstrap command supports three authentication methods for step 4:

- **Password** — `--auth-password` accepts an optional inline value
  (`--auth-password mypass`) or prompts interactively when no value is given.
- **Private key file** — `--auth-private-key-file <path>` reads a PEM key.
- **SSH agent** — automatic fallback when neither flag is given and
  `SSH_AUTH_SOCK` is set. Connects to the local SSH agent via
  `russh::keys::agent::client::AgentClient`, enumerates identities, and tries
  each key via `authenticate_publickey_with`.

The bootstrap command uses `russh` (pure Rust async SSH client) for SSH
transport. Host key verification supports two modes:

- **Strict mode** (`--strict-host-key-checking`): requires a pre-verified host
  key fingerprint via `--host-key-fingerprint`. TOFU is disabled; the connection
  is rejected if the fingerprint does not match.
- **TOFU mode** (default): accepts and records the remote host's key on first
  connection. A `tracing::info!` log is emitted when a key is accepted via TOFU.

For detailed security guidance on choosing between these modes, see
[SSH Agent Secrets -- Host key verification](../security/ssh-agent-secrets.md#host-key-verification).

Remote commands are constructed using `uptrakit_command::shell_escape()` to
prevent shell injection. SSH config parsing uses the `ssh2-config` crate with
`ALLOW_UNKNOWN_FIELDS` to gracefully handle non-standard directives.

For detailed usage and troubleshooting, see
[SSH Agent Bootstrap](../end-user/ssh-agent-bootstrap.md).

## Command Execution over SSH

The `SshCommandExecutor` (`ssh_executor.rs`) implements the `CommandExecutor` trait from
`uptrakit-command`, enabling providers to run version detection and update commands on remote hosts
transparently.

### Architecture

```text
┌────────────────────┐      CommandSpec      ┌─────────────────────┐
│  Provider          │ ────────────────────► │  SshCommandExecutor │
│  (transport-       │      CommandOutput    │                     │
│   agnostic)        │ ◄──────────────────── │  build_remote_      │
└────────────────────┘                       │  command_string()   │
                                             │        │            │
                                             │        ▼            │
                                             │  SshSession::       │
                                             │  exec_command_      │
                                             │  streaming()        │
                                             └─────────────────────┘
                                                      │
                                                      │ russh channel
                                                      ▼
                                             ┌─────────────────────┐
                                             │  Remote Host        │
                                             └─────────────────────┘
```

### Key details

- `build_remote_command_string()` shell-escapes all command components using
  `uptrakit_command::shell_escape()` to prevent shell injection.
- `exec_command_streaming()` uses `LineBuffer` to convert arbitrary byte chunks from SSH channel
  data into line-delimited output, supporting real-time streaming via `mpsc::Sender<UpdateOutputLine>`.
- A 10 MB output limit prevents OOM from runaway commands (matching `LocalCommandExecutor`).
- Transport errors map to `CommandError::CommandSpawn` and non-zero exit codes map to
  `CommandError::CommandFailed`, maintaining consistent error semantics across executors.

For usage details, see [Command Executor](../development/command-executor.md).

## Host Reporting

On authenticated WebSocket connect, the SSH agent reports host information for all enrolled SSH hosts to the controller via the `ReportHosts` message.

### Collection flow

1. The SSH agent iterates over all hosts in its local SQLite database.
2. For each host, it opens an SSH connection (using the stored credentials) and executes remote commands to collect:
   - `machine_id` — `/etc/machine-id` (Linux) or `IOPlatformUUID` (macOS)
   - `os_type` — `uname -s`
   - `os_version` — `/etc/os-release` `PRETTY_NAME` (Linux) or `sw_vers` (macOS)
   - `architecture` — `uname -m`
   - `hostname` — `hostname` command on the remote host
3. `ip_address` is set to the SSH target's hostname/address from the local database (not collected via a remote command).
4. The collected `HostInfo` structs are assembled into a `ReportHostsPayload` and sent to the controller as a `ReportHosts` message.

### Controller processing

- The controller calls `find_or_create_host_and_link()` for each `HostInfo` in the payload.
- Host entities are created or updated (matched by `machine_id`) and linked to the SSH agent service via the `service_hosts` junction table.
- The `ip_address` and `hostname` fields on the `Host` entity are populated from the `HostInfo` if present.

### Error handling

Errors connecting to or collecting info from individual hosts are logged and skipped. A failure
on one host does not prevent reporting for the remaining hosts. If all hosts fail, the agent
sends a `ReportHosts` message with an empty host list.

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
    ├── ssh_config.rs    # SSH config resolution (~/.ssh/config defaults for User, Port, HostName)
    ├── ssh_executor.rs  # SshCommandExecutor (CommandExecutor impl over SSH)
    ├── ssh_key.rs       # SSH private key reading, key type auto-detection, and Ed25519 keygen
    ├── ssh_target.rs    # SshTarget type with FromStr (parses [user@]host[:port] and ssh:// URLs)
    ├── ssh_transport.rs # SSH client wrapper (russh): connect, authenticate, exec_command, LineBuffer
    ├── host_info.rs     # Remote host info collection over SSH (machine_id, os_type, os_version, architecture, hostname)
    ├── host_ops.rs      # CRUD operations for SSH hosts (add, find, list, update, remove)
    ├── commands/
    │   ├── mod.rs       # Command module declarations
    │   ├── host.rs      # Host subcommand handlers (dispatch, SSH config resolution, formatting)
    │   └── bootstrap.rs # Bootstrap workflow (remote setup, verification, DB save)
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
- [SSH Agent Bootstrap](../end-user/ssh-agent-bootstrap.md) — detailed bootstrap workflow and troubleshooting
- [Service Lifecycle](../development/service-lifecycle.md) — `ServiceHandler` trait
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — secret storage and threat model
- [Wire Protocol](../api/wire-protocol.md) — `SshAgent` service type in enrollment
- [Services and Operations](../api/services-operations.md) — shared service management API
