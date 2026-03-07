# SSH-Backed Agent Architecture

The SSH-backed agent (`uptrakit-agent-ssh`) is a service that connects to the controller over WebSocket (like the regular agent) but executes
version detection and updates on remote hosts over SSH instead of locally. It is identified by its capability set, which includes
`SshRemote` alongside `SoftwareDiscovery`, `UpdateHooks`, and `GracefulShutdown`.

## Current Scope

The SSH agent is feature-complete for version checks and updates. The implementation provides:

- The `SshRemote` capability in the `Capability` enum (wire string: `ssh_remote`)
- Controller-side enrollment and WebSocket dispatch for SSH agents (identified by `Capability::SshRemote`)
- A standalone binary (`uptrakit-agent-ssh`) with the `ServiceHandler` trait
- A local SQLite database for storing SSH host credentials (encrypted at rest)
- CLI subcommands for managing SSH host entries locally (`host add/list/show/update/remove/bootstrap/update-sudoers`)
- SSH transport layer (`russh`) for the bootstrap workflow (connect, authenticate, execute remote commands)
- Ed25519 keypair generation for automated key deployment
- A `host bootstrap` command that automates remote host setup (user creation, key deployment, sudoers configuration)
- A `host update-sudoers` command to regenerate minimal per-command sudoers entries as plugins change
- A `bootstrap-proxmox` extension action that bootstraps SSH hosts inside Proxmox VE guests
  (LXC/QEMU) via `pct exec`/`qm guest exec` through an already-bootstrapped PVE node
- Per-host sudo state tracking (`is_root`, `sudo_available`, `sudo_policy`) in the local database
- Runtime `SudoAwareCommandExecutor` wrapping that applies `sudo` based on stored host context — no hard-coded `sudo` in plugin commands
- Host reporting via `ReportHosts` — on authenticated connect, the SSH agent collects system info
  from each enrolled host over SSH and reports it to the controller
- Dynamic host reload — when the local `ssh_hosts` database changes (host added, updated, or
  removed via CLI), the running daemon detects the change within 10 seconds and sends an updated
  `ReportHosts` without requiring a restart (see [Dynamic Host Reload](#dynamic-host-reload))
- Full version check and update execution over SSH, with in-flight update tracking and graceful shutdown (see [Version Check and Update Execution](#version-check-and-update-execution))

Host management (list, bootstrap, remove) is also available via the [UI extension framework](#ui-extension).

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
| `machine_id` | TEXT | Remote host machine ID (populated by `ReportHosts`; used for routing `CheckVersions` and `ExecuteUpdate`) |
| `sudo_available` | BOOLEAN | NULL = unknown; TRUE = passwordless sudo works for this host |
| `is_root` | BOOLEAN | NULL = unknown; TRUE = agent user is UID 0 |
| `sudo_policy` | TEXT | `"auto"` / `"force_with"` / `"force_without"` — runtime sudo execution policy |
| `is_pve_node` | BOOLEAN | Whether this host is a Proxmox VE node (default: false) |
| `pve_plugin_config_id` | TEXT | Plugin config ID for PVE credentials, nullable |
| `created_at` | INTEGER | Unix timestamp |
| `updated_at` | INTEGER | Unix timestamp |

The `name` column has a UNIQUE index to prevent duplicate host names.

The three sudo columns are populated by `host bootstrap` and `host update-sudoers`. When `NULL`,
`Model::resolved_sudo_context()` applies backward-compatible defaults (`sudo_available = true`,
`is_root = false`, `policy = auto`) so hosts enrolled before the sudo tracking migration continue to work.

## Capability Registration

The SSH agent is identified by its capability set rather than a dedicated `ServiceType` variant. The
relevant capabilities are:

| Capability | Wire string | Purpose |
| --- | --- | --- |
| `SshRemote` | `ssh_remote` | Identifies this service as SSH-backed (vs. local agent) |
| `SoftwareDiscovery` | `software_discovery` | Supports `CheckVersions` / `DiscoverSoftware` flows |
| `UpdateHooks` | `update_hooks` | Supports pre-/post-update hook commands |
| `GracefulShutdown` | `graceful_shutdown` | Participates in the graceful-shutdown protocol |
| `UiExtensions` | `ui_extensions` | Provides UI extensions (host management page) |

Integration points:

- `Capability` enum (`crates/shared/wire/src/lib.rs`) -- `SshRemote` variant
- `ServiceProfile::from_capabilities()` -- derives `Agent` profile; `SshRemote` presence distinguishes SSH agents for labeling
- `SettingKey::EnrollmentTokenHash` -- single shared enrollment token (`service_enrollment.token_hash`)
- Controller dispatch (`crates/ui/web-api/src/routes/service_ws/`) -- routes based on capabilities
- Connection registry (`crates/ui/web-api/src/service_connections.rs`) -- unified `register()` accepting `BTreeSet<Capability>`
- AsyncAPI spec (`crates/shared/wire/asyncapi.yaml`) -- `ssh_remote` in Capability enum

## CLI Host Management

The SSH agent binary includes subcommands for managing SSH host entries locally,
without requiring a connection to the controller. These subcommands operate
directly on the local SQLite database.

### Subcommands

| Command | Description |
| --- | --- |
| `host add` | Register a new SSH host with connection details and private key |
| `host list` | List all registered SSH hosts in tabular format (includes sudo policy) |
| `host show <name_or_id>` | Display detailed information for a specific host (includes sudo state) |
| `host update <name_or_id>` | Update one or more fields of an existing host (includes `--sudo-policy`) |
| `host remove <name_or_id>` | Remove an SSH host from the local database |
| `host bootstrap` | Automate remote host setup and save the host entry (see [Bootstrap](#bootstrap-workflow)) |
| `host update-sudoers <name_or_id>` | Regenerate the sudoers drop-in file on an enrolled host (see [Sudo Context](#sudo-context-and-dynamic-execution)) |

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
5. DETECT PRIVILEGES (root check via id -u; sudo -n true)
6. REMOTE SETUP
   - Create user with /bin/sh shell (if different from auth user)
   - Read existing authorized_keys
   - Auto-remove keys matching uptrakit-svc:<this-service-uuid>-host:* (always)
   - Remove all Uptrakit-managed keys (only with --remove-stale-keys)
   - Deploy new key with no-pty/no-agent-forwarding/no-X11-forwarding
   - Resolve plugin commands (command -v per SudoCommandEntry)
   - Write minimal /etc/sudoers.d/uptrakit-<username> (or NOPASSWD: ALL with --allow-all)
   - Validate with visudo -cf
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

### Proxmox Guest Bootstrap

The `bootstrap-proxmox` extension action bootstraps SSH hosts inside Proxmox VE guests without
requiring direct SSH access to the guest. Instead, it uses an already-bootstrapped PVE node as
a gateway.

```text
1. LOAD PVE HOST from local DB (must be marked is_pve_node = true)
2. CONNECT TO PVE NODE via SSH (using stored credentials)
3. CREATE PveGuestExecutor (wraps SSH session + vmid + guest_type)
4. GENERATE Ed25519 KEYPAIR
5. REMOTE SETUP INSIDE GUEST (via pct exec / qm guest exec)
   - Create user (useradd --create-home --shell /bin/sh)
   - Detect home directory (getent passwd)
   - Deploy authorized_keys with restrictions
   - Resolve plugin sudo commands
   - Write /etc/sudoers.d/uptrakit-<username>
6. GET GUEST IP (hostname -I for LXC, network-get-interfaces for QEMU)
7. VERIFY SSH (connect directly to guest IP with deployed key)
8. SAVE TO DATABASE (hostname = guest IP, port = 22)
```

The `PveGuestExecutor` implements `RemoteExecutor` (from `uptrakit-command`) and delegates each
command to `guest_exec::exec_in_guest()`, which builds the appropriate `pct exec` or `qm guest exec`
invocation and runs it on the PVE node via SSH.

For a full list of commands executed inside the guest and the PVE privileges required, see
[Proxmox Bootstrap Privileges](../development/proxmox-bootstrap.md).

### PVE Detection During SSH Bootstrap

When bootstrapping a host via regular SSH (the `bootstrap` action), the agent automatically
detects whether the target is a Proxmox VE node by checking for `pveversion`. If detected,
the agent performs a **cluster deduplication check** before creating credentials:

1. Checks for existing Uptrakit PVE tokens on the cluster via `pveum user list`
2. If a token owned by the **same tenant** already exists: reuses the existing
   `pve_plugin_config_id` from a previously bootstrapped host (skips credential creation
   and `ReportPluginConfig`)
3. If a token owned by a **different tenant** exists: fails with an error (cluster already
   claimed by another tenant)
4. If no token exists: creates a tenant-scoped PVE API user (`uptrakit-{tenant_id}@pve`)
   and token via `pveum` commands, marks the host as `is_pve_node = true`, and sends
   `ReportPluginConfig` to register a Proxmox plugin configuration
5. If `tenant_id` is not yet available (service not enrolled): skips PVE credential creation
   with a warning

The tenant ID is received via `ServiceSettingsPayload.tenant_id` from the controller.

This enables the `bootstrap-proxmox` action to appear in the UI with the PVE node available
as a gateway option.

## Sudo Context and Dynamic Execution

The SSH agent tracks per-host sudo state in the local database and uses it to
dynamically prepend `sudo` to privileged commands at runtime — without
hard-coding `sudo` in plugin command specs.

### How it works

Plugins declare which commands they need `sudo` for via `required_sudo_commands()` (see
[Plugin Guidelines](../development/plugin-guidelines.md#declaring-privileged-commands-with-required_sudo_commands)).
Each command has a `privileged: bool` flag on its `CommandSpec`.

At runtime, the SSH agent wraps `SshCommandExecutor` with `SudoAwareCommandExecutor`:

```text
SudoAwareCommandExecutor { inner: SshCommandExecutor, context: SudoContext }
     │
     │  spec.privileged && context.should_use_sudo()
     ▼
CommandSpec { Exec { program: "sudo", args: ["apt-get", "install", ...] } }
```

`SudoContext` is built from the database columns:

| DB column | `SudoContext` field | Unknown default |
| --- | --- | --- |
| `is_root` | `is_root: bool` | `false` (conservative) |
| `sudo_available` | `sudo_available: bool` | `true` (backward compat) |
| `sudo_policy` | `policy: SudoPolicy` | `Auto` |

### Updating sudo state

- **`host bootstrap`** — detects and stores `is_root` and `sudo_available` during the bootstrap workflow.
- **`host update-sudoers`** — re-detects `is_root` and `sudo_available` on every run (always
  refreshes), then writes or refreshes the sudoers drop-in file.
- **Regular operations** (`CheckVersions`, `ExecuteUpdate`) — read from the database without any SSH detection round-trip.

### `host update-sudoers` workflow

```text
1. Load SSH host from DB by name or UUID
2. Connect & authenticate (using stored credentials)
3. Detect is_root (id -u) and sudo_available (sudo -n true)
4. Persist detected values to DB
5. Collect plugin commands (PluginRegistry::all_required_sudo_commands())
6. Resolve absolute paths on remote host (command -v per entry)
7. Build SudoersContent::SpecificCommands or AllCommands (with --allow-all)
8. Write /etc/sudoers.d/uptrakit-<username>, chmod 440, validate with visudo -cf
9. Update DB: sudo_available = true
10. Print summary
```

Supports `--dry-run` to preview the sudoers file without writing it.

See [Sudoers Management](../security/sudoers-management.md) for the security model and operator guidance.

## Command Execution over SSH

The `SshCommandExecutor` (`ssh_executor.rs`) implements the `CommandExecutor` trait from
`uptrakit-command`, enabling plugins to run version detection and update commands on remote hosts
transparently.

### Architecture

```text
┌────────────────────┐      CommandSpec      ┌─────────────────────┐
│  Plugin            │ ────────────────────► │  SshCommandExecutor │
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

### StdioTunnel support

`SshCommandExecutor` implements `supports_stdio_tunnel()` (returns `true`) and
`open_stdio_tunnel(command)`, which opens a dedicated russh channel and runs the given command on
the remote host. The returned `SshStdioTunnel` wraps `russh::ChannelStream` and implements
`AsyncRead + AsyncWrite`, providing a bidirectional byte stream connected to the remote command's
stdin/stdout.

This is used by the Docker plugin to tunnel Docker API traffic via
`docker system dial-stdio` over the existing SSH session, avoiding a second SSH connection. See
[Command Executor — StdioTunnel](../development/command-executor.md#stdiotunnel) for the generic
abstraction.

For usage details, see [Command Executor](../development/command-executor.md).

## Version Check and Update Execution

The SSH agent handles `CheckVersions`, `DiscoverSoftware`, `ExecuteBatchHostPackageUpdate`, and `ExecuteUpdate`
messages from the controller using the shared `uptrakit-agent-core` crate. The core crate provides both
compute-only `run_*` functions (return `ServiceMessage` without needing a connection) and thin `handle_*`
wrappers (compute + send) for common version-check, discovery, and update logic.

### Background Task Spawning

Long-running operations (`CheckVersions`, `DiscoverSoftware`, `ExecuteBatchHostPackageUpdate`) are executed
as background tokio tasks rather than inline in the `on_message` handler. This prevents a slow or stuck SSH
operation from blocking the event loop, which would make the agent unresponsive to pings, signals, and other
controller messages.

The pattern uses a dedicated `bg_tx`/`bg_rx` mpsc channel (capacity 64):

```text
on_message(CheckVersions)
    │
    ├── spawn_check_versions_ssh(payload, db, pool, bg_tx)
    │       │
    │       └── tokio::spawn ──► run_check_versions_ssh()
    │                                    │
    │                                    └── bg_tx.send(ServiceMessage)
    │
    └── return Ok(None)   ◄── event loop continues immediately

poll_service_event()
    │
    ├── bg_rx.recv() ──► SshAgentEvent::BackgroundResult(msg)
    │
    └── on_service_event() ──► conn.send(msg)
```

The `run_*_ssh` functions in `client.rs` perform host lookup, SSH session acquisition, executor creation,
and delegate to `uptrakit_agent_core::run_*` which returns a `ServiceMessage`. The `spawn_*_ssh` wrappers
clone the necessary state, spawn a tokio task, and send the result through `bg_tx`. The event loop picks
up results via `bg_rx` in `poll_service_event` and sends them to the controller.

`ExecuteUpdate` continues to use the existing forwarder-task pattern (streaming output through the
aggregate channel) because it requires real-time output forwarding rather than a single result message.

### SSH Session Model

The SSH agent maintains a **persistent connection pool** (`SshConnectionPool` in `ssh_pool.rs`) — one
`Arc<SshSession>` per enrolled host. Because `SshSession::exec_command_streaming` takes `&self`, multiple concurrent
callers can open independent SSH channels on the **same TCP connection** simultaneously (multiplexing).

#### Pool lifecycle

- **Acquire**: `pool.acquire(&host)` returns a cached session if one has been used within the last 300 seconds
  (the idle TTL). If no session exists, or the existing one has expired, a new TCP+SSH handshake is performed and the
  session is stored in the pool.
- **Evict**: If a caller detects a connection-level error, it calls `pool.evict(&host_id)` so the next request
  establishes a fresh connection rather than reusing the stale one.
- **Disconnect**: On shutdown, `pool.disconnect_all()` sends a clean SSH disconnect to every remote host instead of
  silently dropping the sockets.

#### Session sharing with `Arc`

`pool.acquire()` returns `Arc<SshSession>`. The pool keeps one `Arc` internally; callers hold a second clone for the
duration of their operation. For update tasks spawned by `handle_execute_update()`, the executor carries its own `Arc`
clone so the SSH connection stays alive until streaming output completes — independently of the pool's own reference.

See [SSH Agent Secrets — SSH Session Lifecycle](../security/ssh-agent-secrets.md#ssh-session-lifecycle) for the
security implications of this design.

### Host Routing via `host_machine_id`

Both `CheckVersionsPayload` and `ExecuteUpdatePayload` carry a required `host_machine_id` field. The SSH agent uses
this field to look up which SSH host to connect to from its local `ssh_hosts` database
(`find_host_by_machine_id()`). When `ReportHosts` completes, `update_host_machine_id()` persists each remote host's
`machine_id` so the lookup is available for future operations.

See [Wire Protocol — host_machine_id Field](../api/wire-protocol.md#host_machine_id-field) for the full routing
specification.

### Version Checks

1. Controller sends `CheckVersions` with `host_machine_id` and a list of `VersionCheckAssignment` items.
   Each assignment carries role-based `PluginAssignment` entries (`detect_version` and optionally
   `fetch_releases`).
2. `on_message` calls `spawn_check_versions_ssh()`, which spawns a background tokio task.
3. The background task looks up the matching host in `ssh_hosts`, acquires a session from
   `SshConnectionPool`, constructs an `SshCommandExecutor` (wrapped with `SudoAwareCommandExecutor`),
   and calls `uptrakit_agent_core::run_check_versions()`.
4. The result (`ServiceMessage::VersionCheckResults`) is sent through `bg_tx` to the event loop,
   which forwards it to the controller via `conn.send()`.
5. The caller's `Arc<SshSession>` clone is dropped; the pool retains its own clone for future reuse.

### Updates

1. Controller sends `ExecuteUpdate` with `host_machine_id` and role-based plugin assignments:
   `execute_update_plugin` (required `PluginAssignment`) and optionally `detect_version_plugin`
   (for before/after installed-version detection).
2. SSH agent looks up the matching host by `host_machine_id`. If an update is already in-flight
   **for that specific host**, the request is rejected immediately. Updates for **different hosts**
   proceed concurrently.
3. A session is acquired from `SshConnectionPool` and passed (via `Arc<SshSession>`) to
   `SshCommandExecutor` (wrapped with `SudoAwareCommandExecutor`).
4. `uptrakit_agent_core::start_update()` spawns an async task that uses the
   `execute_update_plugin` to perform the update and the `detect_version_plugin` (if present) for
   before/after version detection. It streams `update_output` lines to an `mpsc` channel.
5. A **forwarder task** is spawned. It owns the `InFlightUpdate` (the channel receiver and task
   handle) and forwards all `(host_machine_id, UpdateEvent)` tuples to the shared aggregate channel
   on `SshAgentHandler`.
6. The spawned task's `Arc<SshSession>` clone keeps the SSH connection alive until streaming
   completes. The pool retains its own clone so the same connection can be reused for subsequent
   operations once the update task finishes.

### In-Flight Update Tracking

The SSH agent enforces a **per-host** concurrency invariant: at most one update may execute at a
time for a given `host_machine_id`, but different hosts may update simultaneously.

`SshAgentHandler` stores `in_flight_updates: HashMap<String, SshInFlightUpdate>` keyed by
`host_machine_id`. When an `ExecuteUpdate` arrives for a host already in the map, the request is
rejected immediately with an error response. When no conflicting update exists, the SSH agent:

1. Calls `uptrakit_agent_core::start_update()` to spawn the update task and obtain an `InFlightUpdate`.
2. Spawns a lightweight **forwarder task** that owns the `InFlightUpdate` and forwards all
   `output` and `completion` events to a shared `aggregate_tx: mpsc::Sender<(String, UpdateEvent)>` channel.
3. Inserts a `SshInFlightUpdate { update_history_id, forwarder }` entry into the map.

The `poll_service_event()` loop awaits `aggregate_rx.recv()` (the receiving side of the shared
channel) to drive output delivery and completion handling for all concurrent updates. When the map
is empty, `poll_updates()` parks indefinitely so the reload-ticker arm is not starved.

### Graceful Shutdown

On shutdown, the SSH agent drains all in-flight updates using the aggregate channel with a shared
deadline. All updates share the same `shutdown_timeout_seconds` deadline (not per-update):

1. The handler loops over `aggregate_rx.recv()` until `in_flight_updates` is empty or the deadline
   is exceeded.
2. On each received event, output is forwarded and completed updates are removed from the map.
3. If the deadline is reached before all updates finish, each remaining entry receives a `Failed`
   `UpdateResult` message and its forwarder task is aborted.
4. After the drain loop, remaining buffered output events are flushed via `try_recv()`.
5. A `Disconnecting` message is sent, and `SshConnectionPool::disconnect_all()` closes all pooled
   SSH sessions cleanly.

A SIGHUP signal triggers a graceful restart (drain + reconnect) rather than a hard stop.

## Host Reporting

The SSH agent sends `ReportHosts` to the controller in two situations:

- **On connect** — immediately after the authenticated WebSocket session is established.
- **On host-config change** — within 10 seconds of a local `ssh_hosts` database change (see [Dynamic Host Reload](#dynamic-host-reload)).

### Collection flow (on connect)

1. The SSH agent iterates over all hosts in its local SQLite database.
2. For each host, it acquires a session from `SshConnectionPool` (establishing a new connection if none is
   cached) and executes remote commands to collect:
   - `machine_id` — `/etc/machine-id` (Linux) or `IOPlatformUUID` (macOS)
   - `os_type` — `uname -s`
   - `os_version` — `/etc/os-release` `PRETTY_NAME` (Linux) or `sw_vers` (macOS)
   - `architecture` — `uname -m`
   - `hostname` — `hostname` command on the remote host
3. `ip_address` is set to the SSH target's hostname/address from the local database (not collected via a remote command).
4. The collected `HostInfo` structs are assembled into a `ReportHostsPayload` and sent to the controller as a `ReportHosts` message.
5. A lightweight snapshot of `(id, updated_at)` pairs is saved to `host_snapshot` for change detection, and the periodic reload ticker is started.

### Controller processing

- The controller calls `find_or_create_host_and_link()` for each `HostInfo` in the payload.
- Host entities are created or updated (matched by `machine_id`) and linked to the SSH agent service via the `service_hosts` junction table.
- The `ip_address` and `hostname` fields on the `Host` entity are populated from the `HostInfo` if present.
- The operation is **idempotent** — sending `ReportHosts` multiple times during a session is safe and does not duplicate records.

### Error handling

Errors connecting to or collecting info from individual hosts are logged and skipped. A failure
on one host does not prevent reporting for the remaining hosts. If all hosts fail, the agent
sends a `ReportHosts` message with an empty host list.

## Dynamic Host Reload

When the local `ssh_hosts` database changes (host added, removed, or updated via CLI), the
running daemon detects the change within 10 seconds and sends an updated `ReportHosts` message
to the controller — without requiring a restart or reconnection.

### Mechanism

A `reload_ticker` (`tokio::time::Interval`, 10 s, first tick deferred by 10 s after connect) is
stored on `SshAgentHandler`. On each tick, the daemon queries the `ssh_hosts` table for
`(id, updated_at)` pairs and compares them to a stored `host_snapshot`. Any difference (new
row, removed row, or updated `updated_at`) triggers the reload sequence.

The ticker is implemented as `SshAgentEvent::HostConfigChanged`, an arm in the same
`poll_service_event` / `on_service_event` loop that drives in-flight update events:

```rust
enum SshAgentEvent {
    Update(String, client::UpdateEvent),  // (host_machine_id, event) from aggregate channel
    BackgroundResult(ServiceMessage),     // result from background check/discovery/batch task
    HostConfigChanged,                    // reload tick fired
}
```

Two static helper methods (`poll_updates`, `poll_reload_tick`) borrow separate fields of
`SshAgentHandler` so the `select!` can poll both without a double-borrow of `self`.
`poll_updates` parks indefinitely when `in_flight_updates` is empty so the reload-ticker arm
is never starved.

### Reload sequence

When the snapshot differs:

1. Compute the diff: `deleted_ids` (in previous snapshot but not current), `changed_ids`
   (new or same id with updated `updated_at`).
2. `pool.evict(host_id)` for each deleted or changed host — forces a fresh SSH connection on
   next use, discarding stale sessions.
3. Update `self.host_snapshot` to the current state.
4. Load the full host list from the database.
5. Call `client::build_reload_host_infos()` to build `Vec<HostInfo>`:
   - **Known unchanged hosts** (non-empty `machine_id`, not in `changed_ids`): built
     directly from DB values (`machine_id`, `ip_address = hostname`). No SSH needed.
   - **New or changed hosts** (`machine_id` empty or in `changed_ids`): SSH-connect via
     pool, collect OS info, persist `machine_id`, include in list. Hosts that fail SSH are
     skipped with a warning.
6. Send `ReportHosts` to the controller via the existing `conn.send()` path.

### Timing

The first reload tick fires `HOST_RELOAD_INTERVAL` (10 s) after `on_connected` completes, so
it never overlaps with the initial full report. On reconnect, `on_connected` resets the ticker.

### Host deletion on the controller side

The controller's `handle_report_hosts` function only **adds/updates** hosts from the reported
list — it does **not** remove `service_host` junction-table entries for hosts absent from the
new payload. After the agent removes a host locally:

- The pool session for that host is evicted immediately.
- The agent's next `ReportHosts` no longer includes the deleted host, so the agent will not
  service future `CheckVersions`/`ExecuteUpdate` messages for it (returning graceful errors
  if the controller routes them anyway).
- **The host record in the controller's database is not automatically deleted.** The operator
  must separately delete the host via `DELETE /api/v1/hosts/{id}` (web UI or API) to fully
  deactivate it on the controller side.

This is intentional separation of concerns: the agent's local database and the controller's
database are independent. The dynamic reload makes the agent "forget" the host quickly; the
controller-side cleanup is a separate operator action.

## Crate Structure

The SSH agent depends on `uptrakit-agent-core` (`crates/shared/agent-core/`) for shared version check and update
execution logic. See the [agent-core shared crate](#shared-uptrakit-agent-core-crate) section below.

```text
crates/core/agent-ssh/
├── Cargo.toml
├── build.rs
└── src/
    ├── main.rs          # SshAgentHandler (ServiceHandler impl, in_flight_updates HashMap,
    │                    # aggregate_rx/tx channel, bg_rx/bg_tx channel, reload_ticker,
    │                    # host_snapshot), SshAgentEvent enum (Update, BackgroundResult,
    │                    # HostConfigChanged), poll_updates(), diff_host_snapshots(),
    │                    # entry point, master key init
    ├── cli.rs           # CLI args (Commands, HostCommands, CommonServiceArgs integration)
    ├── client.rs        # Authenticated loop; spawn_check_versions_ssh(), spawn_discover_software_ssh(),
    │                    # spawn_execute_batch_host_package_update_ssh() (background-spawned),
    │                    # handle_execute_update_ssh() (per-host guard + forwarder task),
    │                    # SshInFlightUpdate struct, build_reload_host_infos(),
    │                    # report_hosts_after_config_change() — all wrap SshCommandExecutor with
    │                    # SudoAwareCommandExecutor
    ├── extension.rs     # UI extension: manifest builder, action dispatch (list-hosts, bootstrap,
    │                    # remove-host, bootstrap-proxmox, list-pve-hosts, list-discovered-guests,
    │                    # bootstrap-proxmox-guest), ECIES decryption of sensitive params,
    │                    # ServiceExtensionProxy invocation helpers
    ├── error.rs         # Error types (rootcause + thiserror)
    ├── ssh_config.rs    # SSH config resolution (~/.ssh/config defaults for User, Port, HostName)
    ├── ssh_executor.rs  # SshCommandExecutor (CommandExecutor impl over SSH, StdioTunnel support)
    ├── ssh_stdio_tunnel.rs  # SshStdioTunnel (AsyncRead + AsyncWrite wrapper around russh ChannelStream)
    ├── ssh_key.rs       # SSH private key reading, key type auto-detection, and Ed25519 keygen
    ├── ssh_target.rs    # SshTarget type with FromStr (parses [user@]host[:port] and ssh:// URLs, validates hostname syntax)
    ├── ssh_transport.rs # SSH client wrapper (russh): connect, authenticate, exec_command, LineBuffer
    ├── host_info.rs     # Remote host info collection over SSH (machine_id, os_type, os_version, architecture, hostname)
    ├── host_ops.rs      # CRUD operations for SSH hosts (add, find, list, update, remove,
    │                    # update_host_sudo_state, update_host_pve_state, find_pve_hosts,
    │                    # list_host_snapshots, ...)
    ├── remote_exec.rs   # SshRemoteExecutor, PveGuestExecutor (RemoteExecutor impls)
    ├── commands/
    │   ├── mod.rs         # Command module declarations
    │   ├── host.rs        # Host subcommand handlers (dispatch, SSH config resolution, formatting)
    │   ├── bootstrap.rs   # Bootstrap workflow (remote setup, verification, DB save; PVE detection, BootstrapResult; uses sudoers.rs)
    │   ├── bootstrap_proxmox.rs  # Proxmox guest bootstrap via PVE exec
    │   ├── sudoers.rs     # Shared sudoers helpers: detect_is_root, detect_sudo_available,
    │   │                  # resolve_command_path, generate_sudoers_content, write_sudoers_file;
    │   │                  # uses &dyn RemoteExecutor
    │   └── update_sudoers.rs  # update-sudoers command (re-detect, resolve, write, persist)
    └── db/
        ├── mod.rs       # SQLite init (init_db) + tests
        ├── entity/
        │   ├── mod.rs   # Entity module declarations
        │   └── ssh_host.rs  # SeaORM entity (Model + resolved_sudo_context(), SshKeyType enum)
        └── migration/
            ├── mod.rs   # Migration runner
            ├── m20260215_000001_initial.rs         # ssh_hosts table (with UNIQUE index on name)
            ├── m20260222_000002_add_machine_id.rs  # Adds machine_id TEXT NOT NULL DEFAULT ''
            ├── m20260224_000003_add_sudo_columns.rs  # Adds sudo_available, is_root, sudo_policy
            └── m20260306_000001_add_pve_columns.rs  # Adds is_pve_node, pve_plugin_config_id
```

## Shared `uptrakit-agent-core` Crate

Version check and update execution logic shared between `uptrakit-agent` and `uptrakit-agent-ssh` lives in
`crates/shared/agent-core/` (`uptrakit-agent-core`). Public API:

| Function / Type | Description |
| --- | --- |
| `check_version(plugin_assignment, executor)` | Runs a single version check for a role-based plugin assignment using the given executor |
| `execute_update(payload, executor, output_tx)` | Executes an update using role-based plugin assignments and streams output lines |
| `run_check_versions(payload, executor)` | Compute-only: runs version checks, returns `ServiceMessage::VersionCheckResults` |
| `run_discover_software(payload, executor)` | Compute-only: runs discovery, returns `ServiceMessage::DiscoveryResults` |
| `run_execute_batch_host_package_update(payload, executor)` | Compute-only: runs batch host package update, returns `ServiceMessage::BatchHostPackageUpdateResult` |
| `handle_check_versions(payload, executor, conn)` | Thin wrapper: calls `run_check_versions()` then `conn.send()` |
| `handle_discover_software(payload, executor, conn)` | Thin wrapper: calls `run_discover_software()` then `conn.send()` |
| `handle_execute_batch_host_package_update(payload, executor, conn)` | Thin wrapper: calls `run_execute_batch_host_package_update()` then `conn.send()` |
| `start_update(payload, executor, conn, ctx)` | Applies ctx overrides, spawns update task, sends `UpdateStarted`, returns `InFlightUpdate` |
| `handle_execute_update(payload, executor, in_flight, conn)` | Rejects if update already in flight (global guard for single-host agent); delegates to `start_update()` |
| `handle_graceful_shutdown(conn, in_flight, timeout, reason, outcome)` | Drains a single in-flight update before disconnecting (used by the regular agent) |
| `InFlightUpdate` | Handle for a running update task (holds `JoinHandle` and output `mpsc::Receiver`) |
| `UpdateEvent` | Enum of events emitted by an in-flight update (output line, completion) |
| `send_update_output()` | Sends an `update_output` message to the controller |
| `send_update_result()` | Sends an `update_result` message to the controller |

The `run_*` functions are designed for background spawning: they take owned data (no `&mut conn`
reference), perform all computation, and return a `ServiceMessage` that the caller can send through
any channel. The `handle_*` functions are thin wrappers for callers that want compute + send in one step
(used by the regular agent which processes messages inline).

## Connection Resilience

The WebSocket connection from agent-ssh to the controller is protected by two complementary mechanisms implemented in `uptrakit-service-sdk`:

### Write timeout (`SEND_TIMEOUT`)

Every `conn.send()` call is wrapped in a 30-second `tokio::time::timeout`. If the controller stops
consuming data (e.g. after an unclean restart), the TCP send buffer fills and the write would
otherwise block indefinitely — as observed in the 85-minute freeze bug. After 30 seconds, `send()`
returns `ProtocolError::SendTimeout`, which is classified as a transient error
(`is_transient_network() == true`) and triggers reconnect.

This bounds the worst-case Ctrl+C delay to 30 seconds (vs. ~85 minutes with the OS-level TCP
retransmit timeout).

Ping send failures (arm 2 of the event loop) are handled gracefully: instead of propagating as a
fatal error, a warn log is emitted and the outcome is set to `LoopOutcome::Disconnected`, triggering
a reconnect cycle.

### TCP keepalive

After the TCP connection is established (before TLS), `SockRef::set_tcp_keepalive()` from the
`socket2` crate configures:

- **Idle time**: 30 seconds — keepalive probes begin after 30 seconds of inactivity.
- **Probe interval**: 10 seconds — probes are sent every 10 seconds until a reply arrives or the
  OS gives up.

Without keepalive, OS defaults (macOS: 2 hours, Linux: ~2 hours) apply during idle `recv()` periods,
allowing a dead connection to go undetected for hours. With these settings, a dead connection is
detected in ≤ 30 s + 10 s × (OS retry count) ≈ 120–150 s, well before the next ping or the next
send would time out anyway.

### Close timeout (`CLOSE_TIMEOUT`)

After the event loop exits (shutdown or disconnect), the SDK sends a WebSocket close frame with a
5-second timeout. Without this, the `conn.close()` call can block indefinitely when the controller
is unresponsive (e.g. during its own shutdown), preventing the service process from exiting.

See `crates/shared/service-sdk/src/ws.rs` (`connect_ws`),
`crates/shared/service-sdk/src/connection.rs`, and
`crates/shared/service-sdk/src/event_loop.rs` for the implementation.

## UI Extension

The SSH agent registers a `ssh-agent.hosts` UI extension on connect, enabling host management
from the Web UI and CLI without using the agent's local CLI directly.

### ServiceExtensionProxy

The SSH agent uses `ServiceExtensionProxy` (from `uptrakit-service-sdk`) to invoke
controller-side plugin actions. This enables the `bootstrap-proxmox-guest` workflow to
query the Proxmox plugin for discovered guests and auto-match after bootstrap — without
a compile-time dependency on the Proxmox plugin crate.

The proxy is stored as `Arc<ServiceExtensionProxy>` in `SshAgentHandler` and passed to
`ExtensionContext`. When the controller sends `ControllerMessage::ExtensionResponse`, the
handler's `on_extension_response` method calls `proxy.complete()` to deliver the response.

### Extension manifest

| Property | Value |
| --- | --- |
| ID | `ssh-agent.hosts` |
| Label | SSH Hosts |
| Placement | Page (nav_section: `management`, icon: `server`) |
| Permission | `manage_hosts` |
| Targeting | Targeted (user selects which SSH agent instance) |
| UI | DataTable with host columns + row/primary actions |

### Actions

| Action | Type | Timeout | Description |
| --- | --- | --- | --- |
| `list-hosts` | data_action | 30s | Query local DB for all SSH hosts |
| `bootstrap` | primary_action (form) | 120s | Bootstrap a new remote host |
| `remove-host` | row_action (destructive) | 30s | Remove a host from local DB |
| `list-pve-hosts` | select_source (action) | 10s | List PVE-marked hosts for select dropdown |
| `bootstrap-proxmox` | primary_action (form) | 120s | Bootstrap a guest inside a Proxmox VE node |
| `list-discovered-guests` | select_source (action) | 15s | List unmatched Proxmox guests (via ServiceExtensionProxy) |
| `bootstrap-proxmox-guest` | primary_action (form) | 120s | Bootstrap a discovered Proxmox guest with auto-matching |

### E2E encryption for sensitive parameters

The bootstrap action accepts sensitive credentials (SSH password, private key) that must not be
visible to the controller. The SSH agent uses ECIES sealed-box encryption:

1. On connect, the agent base64-encodes its mTLS P-256 public key and includes it in the
   `ExtensionRegister` payload as `encryption_public_key`.
2. The controller surfaces this key in the `GET /api/v1/extensions/{id}/providers` response.
3. Clients encrypt sensitive form fields using the ECIES sealed-box scheme (ephemeral-static
   ECDH on P-256 + SHA-256 KDF + AES-256-GCM) and send the ciphertext in
   `ExtensionRequestPayload.sensitive_params`.
4. The controller passes the ciphertext through opaquely — it cannot decrypt.
5. The SSH agent decrypts using its mTLS private key and extracts the credentials.

See [Extensions Security](../security/extensions.md) for the trust model and
[ECIES Sealed-Box](../security/secrets-encryption.md) for the cryptographic details.

### Execution model

- **`list-hosts`** and **`remove-host`**: Handled inline — fast DB operations, response sent
  immediately from `on_extension_request`.
- **`bootstrap`**: Spawned as a background task via the `bg_tx` channel. The
  `ExtensionResponse` is sent asynchronously when the task completes. The extension proxy's
  120-second timeout handles the case where the task runs too long.
- **`bootstrap-proxmox`**: Spawned as a background task. Connects to the PVE node via SSH,
  executes commands inside the guest via `pct exec` (LXC) or `qm guest exec` (QEMU), verifies
  SSH connectivity, and saves the host.
- **`list-discovered-guests`**: Invokes `proxmox.hosts/list-all-unmatched` via
  `ServiceExtensionProxy`. Returns empty options if the Proxmox plugin is not installed.
- **`bootstrap-proxmox-guest`**: Spawned as a background task. Resolves guest metadata
  from the Proxmox plugin via `ServiceExtensionProxy`, bootstraps the guest (same as
  `bootstrap-proxmox`), then auto-matches the Proxmox host mapping via
  `proxmox.hosts/match`.

### CLI usage

With the dynamic extension subcommands, the SSH host bootstrap extension can be invoked as:

```sh
# List hosts
uptrakit extensions ssh-agent.hosts --service-id <UUID> list-hosts

# Bootstrap a new host
uptrakit extensions ssh-agent.hosts --service-id <UUID> bootstrap \
  --target root@192.168.1.100 \
  --name my-server \
  --auth-method password

# Remove a host
uptrakit extensions ssh-agent.hosts --service-id <UUID> remove-host <host-id>

# Show available actions and their arguments
uptrakit extensions ssh-agent.hosts --service-id <UUID> bootstrap --help
```

## Related Documentation

- [SSH Agent Host Management](../end-user/ssh-agent-host-management.md) — end-user guide for CLI host management, including dynamic reload behaviour
- [SSH Agent Bootstrap](../end-user/ssh-agent-bootstrap.md) — detailed bootstrap workflow and troubleshooting
- [Service Lifecycle](../development/service-lifecycle.md) — `ServiceHandler` trait
- [SSH Agent Secrets](../security/ssh-agent-secrets.md) — secret storage, SSH session lifecycle, and threat model
- [Sudoers Management](../security/sudoers-management.md) — sudoers generation, sudo policy, and operator guidance
- [Wire Protocol](../api/wire-protocol.md) — `ssh_remote` capability in enrollment; `host_machine_id` routing field; `ReportHosts` mid-session semantics
- [Services and Operations](../api/services-operations.md) — shared service management API
- [Command Executor](../development/command-executor.md) — `CommandExecutor` trait, `privileged` flag, and `SudoAwareCommandExecutor`
- [Plugin Guidelines](../development/plugin-guidelines.md) — `required_sudo_commands()` contract
