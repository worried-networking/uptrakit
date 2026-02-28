# Code Review: uptrakit-agent-ssh

- **Review date**: 2026-02-28
- **Reviewer**: AI code review (architecture | security | quality | HA | standards | extensibility)
- **Branch**: docs/codereview-backend

## Summary

`uptrakit-agent-ssh` is the SSH-backed agent binary (~9.1K LoC). It manages a local SQLite
credential store (SSH private keys, AES-256-GCM encrypted at rest), establishes per-operation
SSH connections to enrolled remote hosts, and delegates software version checking, updates, and
discovery to the shared `uptrakit-agent-core` crate. The crate is structurally heavier than
`uptrakit-agent` — it adds `russh`, `ssh-key`, `sea-orm`, and a local migration stack — and
the extra weight is justified: the credential management problem it solves genuinely requires
it.

The code is clean in the areas it controls directly. The main actionable concerns are the SSH
pool's TOCTOU race under concurrent connection acquisition, the unbounded
`report_enrolled_hosts` blocking during `on_connected`, and the absence of test coverage for
the protocol-to-transport bridge in `client.rs`.

## Architecture

### Strengths

- `src/main.rs:44-172` -- Thin `SshAgentHandler` boundary. `main.rs` is 388 lines total
  (including tests), with the `ServiceHandler` lifecycle driven by `service-sdk`.
- Clean module decomposition: `ssh_transport.rs` (russh session + auth), `ssh_executor.rs`
  (`CommandExecutor` adapter), `host_ops.rs` (DB operations), `host_info.rs` (remote info
  collection), `commands/bootstrap.rs` (multi-step provisioning), `commands/host.rs` (CLI
  dispatch). No module is doing two jobs.
- `src/ssh_executor.rs` -- `SshCommandExecutor` wraps `Arc<SshSession>` and implements
  `uptrakit_command::CommandExecutor`. The agent-core handlers receive
  `Arc<dyn CommandExecutor>` and are transport-blind — same code path for local and SSH agents.
- `in_flight_update: Option<InFlightUpdate>` enforces one-update-at-a-time invariant, mirroring
  the same pattern in `uptrakit-agent`.
- `src/error.rs` -- Typed `Error` enum with `impl_report_conversion!` and a variant for every
  failure domain (`SshConnection`, `SshAuth`, `HostKeyMismatch`, `BootstrapVerification`, etc.).
  No `Box<dyn Error>` or `anyhow` leakage.
- `src/commands/bootstrap.rs` -- Nine-step remote provisioning sequence (validate, connect,
  create user, deploy key, write sudoers, validate sudoers, disconnect, re-connect-as-target,
  verify, save) with each step represented as a named sub-function and each error annotated with
  context about the partial state left on the remote host.
- `src/ssh_transport.rs:26` -- 10 MB output cap in `LineBuffer` prevents OOM from runaway
  remote commands. Tested at `ssh_transport.rs:576-617`.
- `Cargo.toml:36-38` -- Uses `sea-orm` with `sqlx-sqlite` for local state, cleanly separating
  local persistence from the controller's shared database.
- `db/mod.rs:19` -- Migration runner applies schema updates automatically on startup.
  `m20260222_000002_add_machine_id.rs` adds column with safe default, preserving existing rows.
  `m20260215_000001_initial.rs:16-19` -- `unique_key()` on `ssh_hosts.name` enforced at DB
  level.
- `host_ops.rs:44` -- UUID v7 primary keys consistent with the rest of the codebase.

### Issues

**[MEDIUM]** `src/main.rs:1-14` -- At 9,141 LoC, agent-ssh contains its own DB module, error
types, SSH pooling, SSH transport, commands, and host management. The SSH connection pool, key
handling, and transport layers could be extracted into a shared `ssh-transport` crate.

**[LOW]** `Cargo.toml:12` -- `dirs` not in workspace dependencies. Declared inline (`dirs = "6"`
rather than in `[workspace.dependencies]`). Currently sole consumer, but inconsistent with
workspace convention and risks version drift.

## Security and Safety

### Strengths

- `src/host_ops.rs:52`, `src/commands/bootstrap.rs:328`, `src/commands/host.rs:113` --
  AES-256-GCM encryption at rest for all SSH private keys. Every private key written to the
  local SQLite database is wrapped in `EncryptedString::new` before insertion. The master key
  is loaded from file or environment and stored as `Zeroizing<[u8; 32]>` inside
  `uptrakit-crypto`.
- `src/main.rs:297` -- Master key required by default; `--allow-plaintext-secrets` requires
  explicit opt-in. Hard-fails (`bail!`) if no master key present. The flag is documented as
  development-only in the help text.
- `src/ssh_transport.rs:56-85` -- Host key pinning + TOFU with logged fingerprint.
  `BootstrapHandler` either validates against a pinned SHA-256 fingerprint or accepts-and-records
  via TOFU with a tracing log. Observed fingerprint persisted to `host_key_fingerprint` in the
  database for subsequent pinned connections.
- `--strict-host-key-checking` requires `--host-key-fingerprint` to be provided (validated at
  `bootstrap.rs:47-51` and `host.rs:105-109`), disabling TOFU entirely for security-conscious
  deployments.
- `src/ssh_transport.rs:435-444` -- RSA SHA-1 fallback handled correctly. Tries SHA-512, then
  SHA-256, then legacy `ssh-rsa` (SHA-1) for RSA keys, matching OpenSSH 8.8+ server
  requirements while preserving compatibility with older servers.
- Zero `unsafe` blocks.
- `src/commands/host.rs:176` -- Private key redacted in CLI output (`"***REDACTED***"` for
  `private_key` in `host show`). The `Model` does not derive `Display`.
- `src/commands/bootstrap.rs:353-387` -- POSIX username validation before remote execution.
  Validates `[a-z_][a-z0-9_-]*` with max 32 chars before building any remote command string.
  Prevents unexpected shell behavior from adversarial input even after shell-escaping.
- Shell injection prevention throughout `bootstrap.rs` via `uptrakit_command::shell_escape`.
- `src/commands/bootstrap.rs:423` -- `authorized_keys` restrictions applied
  (`no-pty,no-agent-forwarding,no-X11-forwarding`), limiting the managed account to
  non-interactive command execution.

### Issues

**[MEDIUM]** `src/main.rs:191-194` -- Master key initialization path does not prevent CLI
subcommand access without a key when `--allow-plaintext-secrets` is absent. In the
`Host { command }` branch (CLI mode), `init_master_key` is called at `main.rs:191`. The guard
is currently correct but not enforced at compile time. A developer adding a future subcommand
that does not require DB access could inadvertently skip the key check if they insert the
branch before `init_master_key`. Consider embedding the check in `db::init_db`.

**[LOW]** `src/ssh_transport.rs:247-248` -- `exit_code.unwrap_or(u32::MAX)` on missing exit
status. When the SSH server closes the channel without sending `ExitStatus`, the exit code is
silently treated as `u32::MAX`, which passes through `i32::try_from(...).unwrap_or(-1)` in
`ssh_executor.rs:50`, producing `-1`. Both are sentinel values absent from POSIX exit code
semantics. Consider using `Option<u32>` through the call chain and converting only at the
point of error reporting.

## Code Quality

### Strengths

- Consistent `rootcause` error propagation. `bail!`, `report!`, and `context_to::<Error>()`
  used throughout. No `unwrap()` in production paths.
- `src/ssh_transport.rs:252` -- `SshSession::disconnect` consumes `self`, making
  use-after-disconnect a compile-time error. `client.rs:112-116` uses `Arc::try_unwrap` to
  obtain ownership before disconnecting.
- `src/ssh_transport.rs:92-186` -- `LineBuffer` streaming/accumulation dual-mode design cleanly
  handles both real-time output forwarding and quiet accumulation. Truncation logic correct:
  streaming continues past the 10 MB cap even after accumulation is truncated.
- `src/ssh_executor.rs:93-108` and `src/commands/bootstrap.rs:391-468` -- Command builders
  produce testable strings as pure functions returning `String`. Test suite covers all builders
  (`bootstrap.rs:516-597`, `ssh_executor.rs:111-183`).
- `src/commands/host.rs:435-442` -- `format_timestamp` uses `time::OffsetDateTime` with RFC
  3339 formatting for stored Unix timestamps.
- `src/client.rs:53-68` -- `SecureKeyFile` RAII wrapper ensures temporary SSH key files are
  cleaned up on drop.
- `src/host_ops.rs:209-212` and `db/mod.rs:29-52` -- In-memory / temp-dir SQLite for all DB
  tests. No shared state, full migration semantics.
- `src/host_ops.rs` -- Comprehensive coverage: add, find by name/ID, duplicate name rejection,
  list empty/populated, remove by name, remove nonexistent, update fields, update rename
  conflict, rename to same name (idempotency), machine_id lifecycle, machine_id empty-string
  guard, and machine_id for nonexistent ID.
- `src/ssh_transport.rs` -- `BootstrapHandler` TOFU/pin/mismatch paths tested with real
  in-process Ed25519 key generation. `LineBuffer` has nine tests covering partial lines, flush,
  streaming-past-truncation, and no-sender mode.
- `src/cli.rs` -- 26 named parse tests covering defaults, SSH target parsing, and conflict
  detection. All `panic!` calls in test-only match fallback arms.
- `src/commands/bootstrap.rs:567-571` -- Bootstrap injection prevention test verifies
  `user'; rm -rf /; echo '` is correctly escaped.
- `src/ssh_executor.rs:162-174` -- SSH executor injection prevention tests verify `$(whoami)`,
  `; rm -rf /`, and `` `id` `` are safely wrapped in single quotes.

### Issues

**[MEDIUM]** No test coverage for `report_enrolled_hosts` or any `client.rs` handler. The three
handlers (`handle_check_versions_ssh`, `handle_execute_update_ssh`,
`handle_discover_software_ssh`) and `report_enrolled_hosts` have no unit tests. The handler
logic — response construction, `send_best_effort` invocation, SSH session lifecycle — is
untested. These are the primary protocol-to-transport bridge and merit at least error-path
tests using a mock `ControllerConnection` and an in-memory DB.

**[MEDIUM]** No test coverage for `commands/host.rs` CLI dispatch. `run_add`, `run_list`,
`run_show`, `run_update`, `run_remove` are untested. The `host_ops` layer beneath them is well
tested, but the CLI-to-ops translation (key reading, encryption, field mapping, stdout
formatting) is exercised only by manual invocation.

**[MEDIUM]** `src/ssh_transport.rs:457,480,502,525,540,559,575,595,619,630` and
`host_ops.rs:237-509` and `db/mod.rs:28,44` -- All 24 agent-ssh async tests use
`#[tokio::test]` without `start_paused = true`. Several `ssh_transport.rs` tests exercise
timeout and connection logic. Per `testing.md`, all async tests require `start_paused = true`.

**[MEDIUM]** `src/commands/update_sudoers.rs:72` -- `.expect("length checked above")` after a
`match matches.len()` guard. Logically safe but fragile to refactoring. Replace with
`.into_iter().next().ok_or_else(...)`.

**[MEDIUM]** `src/commands/update_sudoers.rs:114` -- `.expect("checked above")` on
`url_username.as_deref()`. Same fragile pattern.

**[LOW]** `src/client.rs:158-179` -- `establish_ssh_session` returns
`Result<Arc<SshSession>, String>`. All call sites immediately convert to tracing log + wire
error payload. Returning `crate::error::Result<Arc<SshSession>>` would integrate with the
project error model.

**[LOW]** `src/client.rs` -- Error response construction duplicated across three handlers
(`handle_check_versions_ssh`, `handle_execute_update_ssh`, `handle_discover_software_ssh`).
Each contains two near-identical blocks for "host not found" and "SSH connection failed"
(~120 lines of structural duplication). Extracting a helper would reduce noise.

**[LOW]** `src/commands/bootstrap.rs` -- Input validation and command builder tests do not
cover the full `run_bootstrap` orchestration path. Adding a trait seam for
`connect_and_authenticate` would make the orchestration testable without a live SSH server.

## High Availability

### Strengths

- `src/main.rs:283-376` -- Comprehensive shutdown: waits for in-flight updates with timeout,
  reports failures, aborts forwarders, disconnects SSH pool.
- `src/main.rs:572-573` -- Bounded aggregate channel with capacity 256 provides backpressure
  for per-host update events.
- `src/ssh_pool.rs:32-33` -- SSH connection pool with 300s idle TTL and proper eviction on
  config changes.
- `src/ssh_pool.rs:82-127` -- Pool acquires lock briefly for lookup, then releases before
  establishing new connections.
- `in_flight_update` prevents concurrent updates. `SshAgentHandler` enforces one-update-at-a-
  time by holding `Option<InFlightUpdate>`.
- `src/client.rs:60-70` and `src/client.rs:86-93` -- Per-host SSH connection errors are
  non-fatal during `report_enrolled_hosts`. Logs `tracing::warn!` and `continue`, so one
  unreachable host does not prevent the rest from being reported.
- `on_connected` DB initialization failure is surfaced as `LoopError::Other(...)` rather than
  panicking, allowing the service-sdk to handle the error via its reconnect loop.
- `src/main.rs:471-499` -- Host snapshot diffing prevents unnecessary re-reporting.

### Issues

**[HIGH]** `src/ssh_pool.rs:82-127` -- TOCTOU race in SSH connection pool `acquire`. Between
releasing the lock (line 106) and re-acquiring it (line 119), another concurrent `acquire` for
the same host could start a parallel connection. The second `insert` overwrites the first,
leaving an orphaned SSH connection. Under concurrent version check requests, this could leak
connections.

**[MEDIUM]** `src/main.rs:72` / `src/client.rs:26-143` -- `report_enrolled_hosts` has no upper
bound on total blocking time during `on_connected`. Iterates over every enrolled host
sequentially with a 10-second timeout (`client.rs:47: Duration::from_secs(10)`). For N hosts,
worst-case is N x 10 seconds. During this time `on_connected` has not returned, so the
service-sdk ping keepalive timer has not started. If the report phase exceeds the controller's
inactivity timeout, the controller will close the WebSocket, triggering a reconnect loop.
Mitigation: spawn as a background task, apply a total-scan timeout, or move to a recurring
service event.

**[MEDIUM]** `src/ssh_pool.rs:36` -- SSH connect timeout is 30 seconds but there is no overall
acquire timeout. If multiple hosts are unreachable simultaneously, many tokio tasks could be
blocked for 30 seconds each.

**[LOW]** `service-sdk/src/lifecycle.rs:263-275` (shared) -- Enrollment retry only catches
`ReceiveClosed`, not transient network errors. DNS timeout, TCP connection failure, or TLS
handshake error during enrollment causes process exit rather than retry with backoff.

## Coding Standards

### Strengths

- `src/ssh_target.rs:58` -- Correctly implements `FromStr for SshTarget`.
- `src/cli.rs:242` -- `parse_ssh_target` returns `Result<SshTarget, String>` as a Clap value
  parser (approved exception).
- `edition = "2024"` with workspace-pinned versions. All dependencies declared via
  `{ workspace = true }` or `{ path = "..." }` except `dirs`.
- Zero `#[allow(clippy::...)]` suppressions.

### Issues

**[MEDIUM]** `src/host_ops.rs:28-32,132` -- `Entity::find().all(db)` without tenant scoping.
While architecturally correct (local SQLite, not multi-tenant), a comment explaining why no
tenant filter is needed would improve clarity.

**[MEDIUM]** `db/entity/ssh_host.rs:70` -- `machine_id` stored as empty string sentinel
instead of `NULL`. `String` type with empty-string sentinel conflates "not yet reported" with
"explicitly empty". Using `Option<String>` with `None` for "not yet connected" would be
type-safe.

## Extensibility

### Strengths

- Module structure allows adding new SSH-related features without modifying existing modules.
- `CommandExecutor` trait adapter pattern allows substituting sandboxed or remote executors
  without modifying `main.rs`.

### Issues

No extensibility issues found.
