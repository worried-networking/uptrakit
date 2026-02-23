# CODEREVIEW — uptrakit-agent-ssh

> Reviewed: 2026-02-23
> Reviewer: senior-rust-engineer (Phase 2 automated review)
> Source branch: `docs/codereview-backend`

---

## Summary

`uptrakit-agent-ssh` is the SSH-backed agent binary. It manages a local SQLite
credential store (SSH private keys, AES-256-GCM encrypted at rest), establishes
per-operation SSH connections to enrolled remote hosts, and delegates software
version checking, updates, and discovery to the shared `uptrakit-agent-core`
crate. The crate is structurally heavier than `uptrakit-agent` — it adds
`russh`, `ssh-key`, `sea-orm`, and the local migration stack on top of the
standard agent plumbing — and the extra weight is justified: the credential
management problem it solves genuinely requires it.

The code is clean in the areas it controls directly. The main actionable
concerns are a single high-availability issue in `on_connected`, two
medium-severity issues in the database schema, and a workspace-level
dependency management gap shared with the controller crate.

---

## Architecture

### Strengths

- **Thin `main.rs` / `SshAgentHandler` boundary.** `main.rs` is 388 lines total
  (including tests), with the `SshAgentHandler` impl confined to
  `src/main.rs:44-172`. The `ServiceHandler` lifecycle is driven by
  `service-sdk`, keeping the binary itself near the 200-LoC target established
  across the agent family.

- **Clean module decomposition.** Responsibilities are well separated:
  `ssh_transport.rs` (russh session + auth), `ssh_executor.rs`
  (`CommandExecutor` adapter), `host_ops.rs` (DB operations), `host_info.rs`
  (remote info collection), `commands/bootstrap.rs` (multi-step provisioning
  workflow), `commands/host.rs` (CLI dispatch). No module is doing two jobs.

- **`CommandExecutor` trait adapter pattern.** `SshCommandExecutor` wraps an
  `Arc<SshSession>` and implements `uptrakit_command::CommandExecutor`. The
  agent-core handlers (`handle_check_versions`, `handle_execute_update`,
  `handle_discover_software`) receive a `Arc<dyn CommandExecutor>` and are
  therefore blind to transport — the same code path runs on local and SSH
  agents.

- **`in_flight_update` one-update-at-a-time invariant.** `SshAgentHandler`
  holds `in_flight_update: Option<InFlightUpdate>`. `handle_execute_update_ssh`
  is only called when `in_flight_update` is `None`; a new `ExecuteUpdate`
  message arriving while one is in flight would find the slot occupied. This
  mirrors the same invariant in `uptrakit-agent` and prevents concurrent update
  chaos on a single managed host. Re-exported from `uptrakit-agent-core`.

- **Typed `Error` enum with `impl_report_conversion!`.** `error.rs` defines
  `Error` with a variant for every failure domain (`SshConnection`, `SshAuth`,
  `HostKeyMismatch`, `BootstrapVerification`, etc.) and uses the project-wide
  `rootcause` pattern throughout. No `Box<dyn Error>` or `anyhow` leakage.

- **Bootstrap workflow is locally auditable.** `commands/bootstrap.rs`
  orchestrates a nine-step remote provisioning sequence (validate, connect,
  create user, deploy key, write sudoers, validate sudoers, disconnect,
  re-connect-as-target, verify, save) with each step represented as a named
  sub-function and each error annotated with context about the partial state
  left on the remote host.

- **Shell injection prevention.** Every remote command in `bootstrap.rs` routes
  user-supplied strings through `uptrakit_command::shell_escape`. Injection
  prevention tests exist and verify the escaping directly
  (`cmd_shell_escape_prevents_injection` at `bootstrap.rs:567`).

- **`authorized_keys` restrictions applied.** The deployed public key is
  prefixed with `no-pty,no-agent-forwarding,no-X11-forwarding` (constant
  `AUTHORIZED_KEYS_RESTRICTIONS` at `bootstrap.rs:423`), limiting the managed
  account to non-interactive command execution.

- **10 MB output cap in `LineBuffer`.** `ssh_transport.rs:26` caps accumulated
  output at 10 MB per stream to prevent OOM from runaway remote commands. Tested
  at `ssh_transport.rs:576-617`.

### Issues

**[SEVERITY: Medium]** `Cargo.toml:37` — `sea-orm-migration` not in workspace dependencies

Both `uptrakit-agent-ssh` and `uptrakit-controller` declare
`sea-orm-migration = { version = "2.0.0-rc.32" }` inline. During the RC series
`sea-orm` and `sea-orm-migration` must stay on the same RC patch. Any
independent version bump in one crate (e.g., `2.0.0-rc.33`) will silently
diverge from the other. Add to `[workspace.dependencies]` alongside the
`sea-orm` entry.

**[SEVERITY: Low]** `Cargo.toml:10` — `base64` and `dirs` not in workspace dependencies

`base64 = "0.22"` and `dirs = "6"` are declared inline. `base64` is used
elsewhere in the workspace; a diverging major version in one consumer will not
be caught until link time. `dirs` appears to be a sole consumer here so the
risk is lower, but workspace pinning is the established convention.

---

## Security & Safety

### Strengths

- **AES-256-GCM encryption at rest for all SSH private keys.** Every private
  key written to the local SQLite database is wrapped in `EncryptedString::new`
  before insertion (`host_ops.rs:52`, `bootstrap.rs:328`, `host.rs:113`). The
  master key is loaded from file or environment and stored as
  `Zeroizing<[u8; 32]>` inside `uptrakit-crypto`.

- **Master key required by default; `--allow-plaintext-secrets` requires
  explicit opt-in.** `main.rs:297` hard-fails (`bail!`) if no master key is
  present and `--allow-plaintext-secrets` is absent. The plaintext-secrets path
  emits a `tracing::warn!` at `main.rs:293-295` naming the risk. The flag is
  documented as development-only in the help text.

- **Host key pinning + TOFU with logged fingerprint.** `BootstrapHandler` in
  `ssh_transport.rs:56-85` either validates against a pinned SHA-256 fingerprint
  or accepts-and-records via TOFU with a tracing log. The observed fingerprint
  is persisted to `host_key_fingerprint` in the database so subsequent
  connections are pinned.

- **Strict host key checking mode.** `--strict-host-key-checking` requires
  `--host-key-fingerprint` to be provided (validated at `bootstrap.rs:47-51` and
  `host.rs:105-109`), disabling TOFU entirely for security-conscious
  deployments.

- **RSA SHA-1 fallback handled correctly.** `ssh_transport.rs:435-444` tries
  SHA-512, then SHA-256, then legacy `ssh-rsa` (SHA-1) for RSA keys, matching
  OpenSSH 8.8+ server requirements while preserving compatibility with older
  servers.

- **Zero `unsafe` blocks.** Confirmed by workspace-level audit.

- **Private key redacted in CLI output.** `host.rs:176` prints
  `"***REDACTED***"` for `private_key` in `host show`. The `Model` does not
  derive `Display`, so the raw PEM cannot accidentally appear in log messages.

- **POSIX username validation before remote execution.** `bootstrap.rs:353-387`
  validates `[a-z_][a-z0-9_-]*` with max 32 chars before building any remote
  command string containing the username. Prevents unexpected shell behavior
  from adversarial input even after shell-escaping.

### Issues

**[SEVERITY: Medium]** `main.rs:191-194` — Master key initialization path does not prevent CLI
subcommand access without a key when `--allow-plaintext-secrets` is absent

In the `Host { command }` branch (CLI mode), `init_master_key` is called at
`main.rs:191`. If it fails, the process exits with an error — this is correct.
However, the `--allow-plaintext-secrets` guard is evaluated inside
`init_master_key` but only after `resolve_state_dir_from_common` in the
non-CLI path. In the CLI path the call order is: `init_master_key` then
`resolve_state_dir_from_common`. A developer adding a future subcommand that
does not require DB access (e.g., `host keygen`) could inadvertently skip the
key check if they insert the branch before `init_master_key`. The guard is
currently correct but is not enforced at compile time. Consider making the
master key initialization a prerequisite embedded in `db::init_db` so the
check cannot be bypassed by call-order mistakes.

**[SEVERITY: Low]** `ssh_transport.rs:247-248` — `exit_code.unwrap_or(u32::MAX)` on missing exit status

When the SSH server closes the channel without sending an `ExitStatus` message,
the exit code is silently treated as `u32::MAX`. This value passes through
`i32::try_from(...).unwrap_or(-1)` in `ssh_executor.rs:50`, producing `-1`.
Both are sentinel values that exist nowhere in POSIX exit code semantics. The
caller receives a `CommandFailed(-1)` error, which is correct behavior, but the
choice of `u32::MAX` (which differs from the `-1` sentinel the executor
ultimately reports) makes the conversion logic harder to reason about. Consider
using `Option<u32>` through the call chain and converting only at the point of
error reporting.

---

## Code Quality

### Strengths

- **Consistent `rootcause` error propagation.** `bail!`, `report!`, and
  `context_to::<Error>()` are used throughout. No `unwrap()` in production
  paths. `host_ops.rs:38,60,71` and throughout `bootstrap.rs` all use `?` with
  contextual error types.

- **`SshSession::disconnect` consumes `self`.** The `disconnect` method at
  `ssh_transport.rs:252` takes ownership, making use-after-disconnect a
  compile-time error. Session cleanup in `client.rs` uses `Arc::try_unwrap` to
  obtain ownership before disconnecting; the pattern is commented at
  `client.rs:112-116`.

- **`LineBuffer` is well-abstracted.** The streaming/accumulation dual-mode
  design (`ssh_transport.rs:92-186`) cleanly handles both real-time output
  forwarding (for update jobs) and quiet accumulation (for version checks). The
  truncation logic is correct: streaming continues past the 10 MB cap even after
  accumulation is truncated.

- **Command builders produce testable strings.** `build_remote_command_string`
  in `ssh_executor.rs:93-108` and all `cmd_*` helpers in `bootstrap.rs:391-468`
  are pure functions returning `String`, making them unit-testable without
  mocking the SSH layer. The test suite covers all builders
  (`bootstrap.rs:516-597`, `ssh_executor.rs:111-183`).

- **`format_timestamp` in `host.rs:435-442` uses `time::OffsetDateTime`.**
  Display rendering of stored Unix timestamps correctly uses the project-wide
  `time` crate and RFC 3339 formatting.

### Issues

**[SEVERITY: Medium]** `host_ops.rs:197-202` — `now_unix_timestamp()` uses `std::time::SystemTime` instead of `time::OffsetDateTime`

```rust
// host_ops.rs:197-202
fn now_unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
```

The rest of the codebase uses `time::OffsetDateTime::now_utc()`. This function
introduces a second clock source. The silent `unwrap_or(0)` on error (time
running backwards, which can happen on VMs) sets `created_at` / `updated_at` to
the Unix epoch — January 1, 1970 — with no log line. The correct implementation
is:

```rust
fn now_unix_timestamp() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}
```

This matches the crate's declared `time` workspace dependency, removes the
silent-zero failure mode, and eliminates the inconsistent clock source.

**[SEVERITY: Low]** `client.rs:158-179` — `establish_ssh_session` error type is `String`

`establish_ssh_session` returns `Result<Arc<SshSession>, String>`. All call
sites immediately convert the `String` into a tracing log line and a wire error
payload. The function is `async fn` in an already-typed-error context; returning
`crate::error::Result<Arc<SshSession>>` would integrate with the project error
model and remove the `.map_err(|e| e.to_string())` at the boundary.

**[SEVERITY: Low]** `client.rs` — Error response construction duplicated across three handlers

`handle_check_versions_ssh`, `handle_execute_update_ssh`, and
`handle_discover_software_ssh` each contain two near-identical blocks: one for
"host not found" and one for "SSH connection failed." Each block constructs a
per-assignment error response and calls `conn.send_best_effort(...)`. The
three-handler pattern accounts for roughly 120 lines of structural duplication.
Extracting `fn make_version_check_errors(assignments, msg)` (and equivalents for
the other payloads) would reduce noise and make future changes (e.g., adding
structured error codes) apply once.

---

## Tests

### Strengths

- **In-memory / temp-dir SQLite for all DB tests.** `host_ops.rs:209-212` and
  `db/mod.rs:29-52` use `tempfile::TempDir` + `init_db` as the test fixture.
  No shared state, no external process, full migration semantics.

- **Comprehensive `host_ops` coverage.** The test suite covers: add, find by
  name, find by ID, duplicate name rejection, list empty, list populated, remove
  by name, remove nonexistent, update fields, update rename conflict, rename to
  same name (idempotency), machine_id lifecycle, machine_id empty-string guard,
  and machine_id for nonexistent ID. All happy and error paths are covered.

- **`ssh_transport` tested at unit level.** `BootstrapHandler` TOFU/pin/mismatch
  paths are tested with real in-process Ed25519 key generation. `LineBuffer`
  has nine tests covering partial lines, flush, streaming-past-truncation, and
  the no-sender mode.

- **`cli.rs` is entirely tests.** The 952-line `cli.rs` file contains only
  `Args` / `Commands` type definitions (173 lines) and a `#[cfg(test)]` block
  (779 lines) with 26 named parse tests. All `panic!` calls appear exclusively
  inside test match fallback arms, which is the idiomatic Rust pattern for
  exhaustive enum assertions in tests. The phase 1 note about "30+ `panic!`
  calls in non-test dispatch code" does not apply to this crate — the panics are
  test-only.

- **Bootstrap command builders tested for shell injection.** The
  `cmd_shell_escape_prevents_injection` test at `bootstrap.rs:567-571`
  verifies that `user'; rm -rf /; echo '` is correctly escaped to
  `'user'\\''`.

- **`ssh_executor` injection prevention tests.** `ssh_executor.rs:162-174`
  tests that `$(whoami)`, `; rm -rf /`, and `` `id` `` in command arguments are
  safely wrapped in single quotes and do not expand.

### Issues

**[SEVERITY: Medium]** No test coverage for `report_enrolled_hosts` or any `client.rs` handler

The three handlers in `client.rs` (`handle_check_versions_ssh`,
`handle_execute_update_ssh`, `handle_discover_software_ssh`) and
`report_enrolled_hosts` have no unit tests. The host-lookup branches ("host not
found", "DB error") are exercised indirectly by `host_ops` tests but the handler
logic itself — response construction, `send_best_effort` invocation, SSH session
lifecycle — is untested. These handlers are the primary protocol-to-transport
bridge and merit at least error-path tests using a mock
`ControllerConnection` and an in-memory DB.

**[SEVERITY: Medium]** No test coverage for `commands/host.rs` CLI dispatch

`run_add`, `run_list`, `run_show`, `run_update`, `run_remove` in
`commands/host.rs` are untested. The `host_ops` layer beneath them is well
tested, but the CLI-to-ops translation (key reading, encryption, field mapping,
stdout formatting) is exercised only by manual invocation. An integration test
using `tempfile::TempDir` as the state dir and calling `run(state_dir, command)`
directly would provide meaningful coverage.

**[SEVERITY: Low]** `bootstrap.rs` input validation and command builder tests do not cover the
full `run_bootstrap` orchestration path

The unit tests in `bootstrap.rs` cover `validate_posix_username` and all
`cmd_*` builders. The `run_bootstrap` function itself (which sequences
connect, user creation, key deployment, sudoers, verification, DB save) is not
tested — doing so would require a mock SSH target. Adding a trait seam for
`connect_and_authenticate` (analogous to how `CommandExecutor` is injected in
the daemon handlers) would make the orchestration testable without a live SSH
server.

---

## High Availability

### Strengths

- **`in_flight_update` prevents concurrent updates.** `SshAgentHandler` enforces
  one-update-at-a-time by holding `Option<InFlightUpdate>`. A second
  `ExecuteUpdate` message from the controller cannot start while a job is
  running; the controller is responsible for not issuing overlapping requests.

- **Graceful shutdown delegates to `uptrakit-agent-core`.** `on_shutdown` calls
  `client::handle_graceful_shutdown` which waits up to `shutdown_timeout_seconds`
  for the in-flight update to complete before sending `Disconnect`. The SSH
  session for the running update remains open until the spawned task completes.

- **Per-host SSH connection errors are non-fatal during `report_enrolled_hosts`.**
  `client.rs:60-70` and `client.rs:86-93` log a `tracing::warn!` and `continue`
  on connection or executor failure, so one unreachable host does not prevent the
  rest from being reported.

- **`on_connected` DB initialization failure is surfaced as `LoopError::Other`.**
  If `init_db` fails, `on_connected` returns `Err(LoopError::Other(...))` rather
  than panicking, allowing the service-sdk to handle the error via its reconnect
  loop.

### Issues

**[SEVERITY: Medium]** `main.rs:72` / `client.rs:26-143` — `report_enrolled_hosts` has no upper bound on
total blocking time during `on_connected`

```rust
// main.rs:71-72
client::report_enrolled_hosts(&local_db, conn).await;
self.local_db = Some(local_db);
```

`report_enrolled_hosts` iterates over every enrolled host sequentially, opening
a new SSH connection to each with a 10-second timeout
(`client.rs:47: Duration::from_secs(10)`). For N hosts, the worst-case blocking
time is N × 10 seconds. During this time `on_connected` has not returned, so the
service-sdk ping keepalive timer has not started. If the report phase takes
longer than the controller's inactivity timeout, the controller will mark the
agent as stale and close the WebSocket, forcing a reconnect — which triggers
another `on_connected`, another full scan, and another potential timeout.

Mitigation options:
1. Spawn `report_enrolled_hosts` as a background task and return from
   `on_connected` immediately, sending the `ReportHosts` message when complete.
2. Apply a total-scan timeout (e.g., 30 seconds) that fires `ReportHosts` with
   whatever data was collected up to that point.
3. Move host scanning to a recurring service event rather than the connection
   lifecycle hook.

**[SEVERITY: Low]** `service-sdk/src/lifecycle.rs:263-275` (shared) — Enrollment retry only catches
`ReceiveClosed`, not transient network errors

This is a shared-crate issue that affects `uptrakit-agent-ssh` along with the
other agent binaries. If a DNS timeout, TCP connection failure, or TLS handshake
error occurs during enrollment, the process exits immediately rather than
retrying. For the SSH agent specifically, this means a transient network blip
during initial startup requires an external process manager to restart the agent.
A retry loop with exponential backoff (consistent with the backoff pattern
already present in `uptrakit-service-sdk`) should wrap the enrollment attempt.

---

## Database

### Strengths

- **Local SQLite credential store is the correct design.** The SSH agent manages
  credentials that are local to the machine it runs on. Using a local SQLite
  database avoids adding a round-trip to the controller for every SSH operation,
  allows the agent to function during controller unavailability, and keeps
  private key material off the network entirely. The migration runner
  (`db/mod.rs:19`) applies schema updates automatically on startup.

- **Migrations follow the additive-column pattern.** `m20260222_000002_add_machine_id.rs`
  adds `machine_id TEXT NOT NULL DEFAULT ''` with a safe default, preserving
  existing rows on upgrade. The `down()` implementation drops the column.

- **`unique_key()` on `ssh_hosts.name`.** The initial migration at
  `m20260215_000001_initial.rs:16-19` adds a unique constraint on `name` at the
  DB level, making `host_ops::add_host`'s application-level uniqueness check a
  defense-in-depth layer rather than the only guard.

- **UUID v7 primary keys.** `host_ops.rs:44` uses `uuid::Uuid::now_v7()`, giving
  time-ordered, globally unique IDs consistent with the rest of the codebase.

### Issues

**[SEVERITY: Medium]** `db/entity/ssh_host.rs:71-72` and `db/migration/m20260215_000001_initial.rs:32-33` —
`created_at` and `updated_at` stored as `INTEGER` (Unix epoch seconds) instead of typed TIMESTAMP columns

```rust
// ssh_host.rs:71-72
pub created_at: i64,
pub updated_at: i64,
```

```rust
// m20260215_000001_initial.rs:32-33
.col(ColumnDef::new(SshHosts::CreatedAt).integer().not_null())
.col(ColumnDef::new(SshHosts::UpdatedAt).integer().not_null())
```

All other entities in the workspace use `time::OffsetDateTime` with SeaORM's
`DateTimeWithTimeZone` column type. Using raw `i64` epoch seconds:

1. Breaks consistency with the rest of the schema — generic tools and future
   contributors will not recognize these columns as timestamps.
2. Prevents SeaORM from performing timestamp-aware comparisons or ordering
   without manual casting.
3. Is directly linked to the `now_unix_timestamp()` / `SystemTime` issue in
   `host_ops.rs` (see Code Quality section): had the column type been
   `OffsetDateTime`, the compiler would have guided the implementer toward
   `time::OffsetDateTime::now_utc()`.

Migration path: add a new migration that alters the column type to
`timestamp_with_time_zone`, reads existing `INTEGER` rows, converts them to
RFC 3339 strings, and updates the entity model field types to
`time::OffsetDateTime`.

**[SEVERITY: Medium]** `db/entity/ssh_host.rs:70` — `machine_id` stored as empty string sentinel instead
of `NULL`

```rust
// ssh_host.rs:70
/// Machine ID of the remote host, populated from `ReportHosts` data.
/// Empty string until the host has been connected to at least once.
pub machine_id: String,
```

The column is `TEXT NOT NULL DEFAULT ''`. An empty string as a sentinel for
"not yet populated" is semantically ambiguous — a valid `machine_id` string
(however unlikely) could also be empty. `NULL` is the semantically correct
representation for "not yet populated." The `find_host_by_machine_id` guard at
`host_ops.rs:187-189` compensates for this by treating empty string as not-found,
but the invariant is enforced only in application code, not at the DB level.

A `NULL`-able column with a filtered unique index
(`WHERE machine_id IS NOT NULL`) would enforce at the DB level that no two
hosts can have the same populated `machine_id`, while allowing multiple
un-populated hosts.

**[SEVERITY: Low]** `db/migration/m20260215_000001_initial.rs` — No index on `machine_id` column

`find_host_by_machine_id` (`host_ops.rs:190-195`) runs a full table scan
(`Entity::find().filter(Column::MachineId.eq(machine_id))`). For the current
scale (tens to hundreds of hosts per agent) this is acceptable, but an index on
`machine_id` would make the intent explicit and future-proof the lookup. This is
particularly relevant given that every `CheckVersions` and `ExecuteUpdate`
message triggers this query.

---

## Coding Standards

### Strengths

- **Edition 2024, consistent with workspace.** `Cargo.toml:2`.
- **`bail!` / `report!` / `context_to` used consistently.** No `unwrap()` in
  production paths. `host_ops.rs` tests use `expect()` only in test fixtures,
  which is approved usage.
- **`rpassword` for password prompts.** `host.rs:307` uses `rpassword::prompt_password`
  rather than reading `stdin` directly, preventing the password from appearing
  in shell history or process argument lists.
- **`zeroize` on master key bytes.** `main.rs:283` wraps parsed key bytes in
  `zeroize::Zeroizing::new` before passing to `uptrakit_crypto::init_master_key`.
- **No `#[allow(clippy::...)]` suppressions.** Confirmed by source inspection.
- **`--allow-plaintext-secrets` argument is documented as development-only** in
  both the help text and the warning log line. The dual-check (both master key
  present and flag set) at `main.rs:275-281` logs an additional caveat that
  encryption remains enabled.

### Issues

**[SEVERITY: Low]** `db/mod.rs:14` — SQLite URL constructed with `format!` and `display()`

```rust
let url = format!("sqlite:{}?mode=rwc", db_path.display());
```

`Path::display()` uses lossy UTF-8 encoding on non-UTF-8 paths. On systems
where `state_dir` contains non-UTF-8 path components (uncommon but valid on
Linux), the constructed URL will silently contain replacement characters and
the connection will fail with a confusing error. `db_path.to_str()` with an
explicit error on `None` would surface the issue as a clear message at
initialization time.

**[SEVERITY: Low]** `cli.rs:55-56` — `port: i32` in `HostCommands::Add` and `HostCommands::Update`

The `port` argument is declared as `i32` in the Clap definition. SSH port
numbers are unsigned 16-bit integers (0–65535). Clap will accept negative values
and values above 65535 at parse time; the only enforcement is in `bootstrap.rs:109`
which validates at `u16::try_from`. The `Add` subcommand lacks an equivalent
check — if `run_add` receives `port: -1`, it is stored as `-1` in the database
and later cast to `u16` via `host.port as u16` in `client.rs:44`, wrapping
silently. Use `u16` in the Clap definition (Clap supports `u16` directly) or
add a range validator to the argument.

---

## Extensibility

### Strengths

- **`AuthMethod` enum is an open abstraction.** `ssh_transport::AuthMethod` has
  three variants (`Password`, `PrivateKey`, `Agent`) and is used at every
  connection point. Adding a new authentication method (e.g.,
  `Certificate(&'a str)`) requires a single new variant and a match arm in
  `connect_and_authenticate` — no interface changes elsewhere.

- **`SshKeyType` follows the workspace `FromStr` pattern.** Typed enum with
  `ParseSshKeyTypeError`, `FromStr`, `Display`, `as_str()`, and `DeriveActiveEnum`
  for SeaORM. Consistent with `AlertSeverity` and other typed enums in the
  workspace.

- **Bootstrap is parameterized, not CLI-coupled.** `BootstrapParams` in
  `bootstrap.rs:26-40` decouples the workflow from Clap. CLI translation happens
  in `commands/host.rs:264-365` and feeds `BootstrapParams` to
  `bootstrap::run_bootstrap`. The workflow itself can be driven from other
  sources (e.g., an API handler) without touching `bootstrap.rs`.

### Issues

**[SEVERITY: Low]** `ssh_transport.rs:278` — `client::Config::default()` used without timeout configuration

```rust
let ssh_config = Arc::new(client::Config::default());
```

The `russh::client::Config` default configures keepalive and negotiation
timeouts. The connect timeout is handled correctly via `tokio::time::timeout`
at `ssh_transport.rs:281-296`, but the russh-level keepalive interval and
inactivity timeout are left at whatever `russh` defaults to. Explicit
configuration of `keepalive_interval` and `keepalive_max` in
`SshConnectionConfig` would make the session lifecycle behavior visible and
tunable rather than inherited from the dependency's defaults.

**[SEVERITY: Low]** No abstraction for "SSH connection factory"

All three operation handlers (`handle_check_versions_ssh`,
`handle_execute_update_ssh`, `handle_discover_software_ssh`) call the same
`establish_ssh_session` helper. There is no session pool or connection reuse:
every operation opens a new TCP connection, performs a TLS handshake, and
authenticates. For deployments with many hosts receiving frequent version checks,
this is measurably more expensive than reusing sessions. A
`SshSessionFactory` trait (with a real implementation and a test double) would
enable future connection reuse without changing the handler interfaces.
