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
- Per-host sudo state tracking (`is_root`, `sudo_available`, `sudo_policy`) in the local database
- Runtime `SudoAwareCommandExecutor` wrapping that applies `sudo` based on stored host context — no hard-coded `sudo` in plugin commands
- Host reporting via `ReportHosts` — on authenticated connect, the SSH agent collects system info
  from each enrolled host over SSH and reports it to the controller
- Dynamic host reload — when the local `ssh_hosts` database changes (host added, updated, or
  removed via CLI), the running daemon detects the change within 10 seconds and sends an updated
  `ReportHosts` without requiring a restart (see [Dynamic Host Reload](#dynamic-host-reload))
- Full version check and update execution over SSH, with in-flight update tracking and graceful shutdown (see [Version Check and Update Execution](#version-check-and-update-execution))

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
| `machine_id` | TEXT | Remote host machine ID (populated by `ReportHosts`; used for routing `CheckVersions` and `ExecuteUpdate`) |
| `sudo_available` | BOOLEAN | NULL = unknown; TRUE = passwordless sudo works for this host |
| `is_root` | BOOLEAN | NULL = unknown; TRUE = agent user is UID 0 |
| `sudo_policy` | TEXT | `"auto"` / `"force_with"` / `"force_without"` — runtime sudo execution policy |
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
   - Deploy authorized_keys with no-pty/no-agent-forwarding/no-X11-forwarding
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

The SSH agent handles `CheckVersions` and `ExecuteUpdate` messages from the controller using the shared
`uptrakit-agent-core` crate. The core crate provides `handle_check_versions()`, `handle_execute_update()`, and
`handle_graceful_shutdown()`, which contain the logic common to both the regular agent and the SSH agent.

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
2. SSH agent looks up the matching host in `ssh_hosts`.
3. A session is acquired from `SshConnectionPool` (reusing an existing connection when available) and
   an `SshCommandExecutor` is constructed (wrapped with `SudoAwareCommandExecutor` for privilege elevation).
4. `uptrakit_agent_core::handle_check_versions()` is called with the executor; it dispatches to the
   `detect_version` plugin for installed version detection and the `fetch_releases` plugin (if present)
   for agent-side latest version resolution, then sends `version_check_results` back to the controller.
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
    │                    # aggregate_rx/tx channel, reload_ticker, host_snapshot),
    │                    # SshAgentEvent enum, poll_updates(), diff_host_snapshots(),
    │                    # entry point, master key init
    ├── cli.rs           # CLI args (Commands, HostCommands, CommonServiceArgs integration)
    ├── client.rs        # Authenticated loop; handle_check_versions_ssh(), handle_execute_update_ssh()
    │                    # (per-host guard + forwarder task), handle_discover_software_ssh(),
    │                    # SshInFlightUpdate struct, build_reload_host_infos(),
    │                    # report_hosts_after_config_change() — all wrap SshCommandExecutor with
    │                    # SudoAwareCommandExecutor
    ├── error.rs         # Error types (rootcause + thiserror)
    ├── ssh_config.rs    # SSH config resolution (~/.ssh/config defaults for User, Port, HostName)
    ├── ssh_executor.rs  # SshCommandExecutor (CommandExecutor impl over SSH, StdioTunnel support)
    ├── ssh_stdio_tunnel.rs  # SshStdioTunnel (AsyncRead + AsyncWrite wrapper around russh ChannelStream)
    ├── ssh_key.rs       # SSH private key reading, key type auto-detection, and Ed25519 keygen
    ├── ssh_target.rs    # SshTarget type with FromStr (parses [user@]host[:port] and ssh:// URLs, validates hostname syntax)
    ├── ssh_transport.rs # SSH client wrapper (russh): connect, authenticate, exec_command, LineBuffer
    ├── host_info.rs     # Remote host info collection over SSH (machine_id, os_type, os_version, architecture, hostname)
    ├── host_ops.rs      # CRUD operations for SSH hosts (add, find, list, update, remove,
    │                    # update_host_sudo_state, list_host_snapshots, ...)
    ├── commands/
    │   ├── mod.rs         # Command module declarations
    │   ├── host.rs        # Host subcommand handlers (dispatch, SSH config resolution, formatting)
    │   ├── bootstrap.rs   # Bootstrap workflow (remote setup, verification, DB save; uses sudoers.rs)
    │   ├── sudoers.rs     # Shared sudoers helpers: detect_is_root, detect_sudo_available,
    │   │                  # resolve_command_path, generate_sudoers_content, write_sudoers_file
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
            └── m20260224_000003_add_sudo_columns.rs  # Adds sudo_available, is_root, sudo_policy
```

## Shared `uptrakit-agent-core` Crate

Version check and update execution logic shared between `uptrakit-agent` and `uptrakit-agent-ssh` lives in
`crates/shared/agent-core/` (`uptrakit-agent-core`). Public API:

| Function / Type | Description |
| --- | --- |
| `check_version(plugin_assignment, executor)` | Runs a single version check for a role-based plugin assignment using the given executor |
| `execute_update(payload, executor, output_tx)` | Executes an update using role-based plugin assignments and streams output lines |
| `handle_check_versions(payload, executor, conn)` | Refreshes package indexes, runs version checks, sends `version_check_results` |
| `start_update(payload, executor, conn, ctx)` | Applies ctx overrides, spawns update task, sends `UpdateStarted`, returns `InFlightUpdate` |
| `handle_execute_update(payload, executor, in_flight, conn)` | Rejects if update already in flight (global guard for single-host agent); delegates to `start_update()` |
| `handle_graceful_shutdown(conn, in_flight, timeout, reason, outcome)` | Drains a single in-flight update before disconnecting (used by the regular agent) |
| `InFlightUpdate` | Handle for a running update task (holds `JoinHandle` and output `mpsc::Receiver`) |
| `UpdateEvent` | Enum of events emitted by an in-flight update (output line, completion) |
| `send_update_output()` | Sends an `update_output` message to the controller |
| `send_update_result()` | Sends an `update_result` message to the controller |

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
