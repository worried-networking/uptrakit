# CODEREVIEW — uptrakit-agent

## Summary

`uptrakit-agent` is a thin binary entry point (~200 LoC across four source files).
It owns the `AgentHandler` struct, wires together the `ServiceHandler` lifecycle
from `uptrakit-service-sdk`, and delegates every domain operation to
`uptrakit-agent-core` and `uptrakit-command`.  The crate carries no direct
database dependency, no web framework, and no heavyweight I/O of its own.  The
majority of substantive logic — update execution, version checks, software
discovery, graceful shutdown — lives in the underlying library crates and is
therefore reviewed there.  Issues surfaced below are either directly observable in
this crate's code or are inherited concerns that manifest at the agent binary
boundary.

---

## Architecture

### Strengths

- **Clean separation of concerns.**  `main.rs` is the only file that touches
  `#[tokio::main]`; it constructs one `AgentHandler` and hands control to
  `run_lifecycle_and_handle_errors`.  The binary has no opinions about retry
  timing, TLS setup, or enrollment — all delegated to `uptrakit-service-sdk`.

- **No direct database dependency.**  `Cargo.toml` lists no `sea-orm`, no
  migration crate, and no raw SQL.  The agent is stateless between connections;
  all persistent state lives on the controller side.

- **`ServiceHandler` trait correctly implemented.**  The three associated constants
  (`DIR_NAME`, `SERVICE_LABEL`, `SERVICE_TYPE`) match the agent domain, and
  `capabilities()` is consistent with the free function `agent_capabilities()` used
  in `on_connected`.

- **`in_flight_update` enforces one-update-at-a-time.**  `AgentHandler` holds a
  single `Option<InFlightUpdate>`.  A second `ExecuteUpdate` message while one is
  running is handled inside `uptrakit-agent-core` which checks this field before
  spawning, preventing concurrent update races on the same host.

- **Correct capability advertisement.**  The three capabilities
  (`SoftwareDiscovery`, `UpdateHooks`, `GracefulShutdown`) are returned both in
  `on_connected`'s `ReportHosts` payload and from `capabilities()`, keeping the
  two call sites in sync via the shared free function `agent_capabilities()`.

- **Lifecycle delegation is correct.**  `on_shutdown` hands `in_flight_update`
  over to `handle_graceful_shutdown` via `.take()`, ensuring the in-flight state
  is consumed exactly once and not double-processed.

### Issues

**[SEVERITY: Low]** `src/client.rs:19,32,46` — `LocalCommandExecutor` allocated per message

A fresh `Arc::new(LocalCommandExecutor)` is constructed inside each of the three
handler functions (`handle_check_versions`, `handle_execute_update`,
`handle_discover_software`).  `LocalCommandExecutor` is stateless and zero-cost to
construct, so this is not a correctness issue, but it is semantically misleading —
it implies the executor carries per-invocation state.  A single
`Arc<LocalCommandExecutor>` stored on `AgentHandler` would make the intent
explicit and eliminate three redundant allocations per message.

---

## Security & Safety

### Strengths

- **Machine-ID validation on every inbound message.**  All three authenticated
  message variants (`CheckVersions`, `ExecuteUpdate`, `DiscoverSoftware`) compare
  `payload.host_machine_id` against the value captured in `on_connected` before
  doing any work.  A mismatch produces a structured `tracing::warn!` (with both
  `expected` and `received` fields) and returns `Ok(None)` — no crash, no
  information leak, no action taken.  This is a meaningful defense-in-depth layer
  even after mTLS enrollment.

- **Zero `unsafe` blocks.**  Consistent with the rest of the workspace.

- **No secret material in this crate.**  The agent binary does not handle
  passwords, tokens, or encryption keys directly; those concerns are encapsulated
  in `uptrakit-service-sdk` and `uptrakit-crypto`.

- **`_` wildcard arm on `on_message` is defensive, not silent.**  Unknown
  `ControllerMessage` variants log at `tracing::debug!` and return `Ok(None)`,
  which is appropriate forward-compatibility behavior.

### Issues

**[SEVERITY: Medium]** `src/host_info.rs:77` — Machine-ID falls back to the literal
string `"unknown"` without warning

`read_machine_id()` returns `"unknown".to_string()` when neither `/etc/machine-id`
(Linux) nor `ioreg` (macOS) succeeds.  This value is then stored in
`self.machine_id` and used as the reference for all subsequent machine-ID
validation.  If any inbound message also carries `host_machine_id = "unknown"` —
plausible if a second agent on the same or a different host also failed ID
detection — validation passes spuriously.  The fallback should at minimum emit
`tracing::warn!`; ideally it should return a `Result` and allow the lifecycle to
abort enrollment with a diagnostic rather than silently accepting a non-unique ID.

**[SEVERITY: Low]** `src/host_info.rs:20,30,60,98` — Hostname and OS-version
collection use synchronous `std::process::Command` inside an async context

`read_hostname()` spawns `hostname -f` and `hostname`, and `read_os_version()` on
macOS spawns `sw_vers`, all via `std::process::Command::output()`.  These are
called from `on_connected`, which is an `async fn`.  Blocking the Tokio thread
while waiting for a subprocess to complete can starve other tasks on the same
worker thread.  The calls are fast in practice (sub-millisecond on a healthy
system), but under system stress or when `hostname -f` triggers a slow DNS lookup
the thread can block for several seconds.  The correct fix is
`tokio::process::Command`.

---

## Code Quality

### Strengths

- **Consistent error propagation.**  `on_connected` uses `.context_to::<LoopError>()?`
  from `rootcause`, matching workspace conventions.  No raw `.unwrap()` or
  `.expect()` in non-test production paths.

- **`biased` selector in `poll_service_event`.**  The `tokio::select!` uses
  `biased`, which ensures output lines from the running update are drained before
  the completion result is processed — the correct ordering for a streaming update
  protocol.

- **`std::future::pending()` as the idle case.**  When no update is in flight,
  `poll_service_event` returns a never-resolving future rather than spinning.  This
  avoids a busy-loop and integrates cleanly with the service-sdk event loop.

- **`env!("CARGO_PKG_VERSION")` for version reporting.**  Compile-time constant;
  no runtime parsing, no possibility of version/binary mismatch.

- **`cli.rs` has good CLI argument tests.**  Eleven tests covering defaults,
  directory overrides, conflict detection (`--tofu` + `--ca-cert`, `--tofu` +
  `--pki-addr`), URL parsing edge cases, and scheme enforcement.  This is
  proportionate coverage for a thin argument parser that wraps `CommonServiceArgs`.

### Issues

**[SEVERITY: Low]** `src/main.rs:96` — `_identity` parameter in `on_connected` is
unused but undocumented

The `_identity: &ServiceIdentityState` parameter is intentionally ignored
(the agent uses its own `host_info` collection rather than the SDK-provided
identity).  A brief inline comment explaining why the identity is not consulted
would prevent future maintainers from wondering if the omission is a bug.

---

## Tests

### Strengths

- **`cli.rs` unit tests are thorough for their scope.**  All eleven tests are
  synchronous and fast; they exercise CLI parsing, flag conflict detection, URL
  validation, and directory resolution without any I/O.

- **`host_info.rs` smoke test.**  `collect_host_info_returns_valid_data` verifies
  that machine ID is non-empty, OS type and architecture match compile-time
  constants, and hostname is present.  Appropriate for a host-inspection function
  that is inherently environment-dependent.

- **`dev-dependencies` are minimal and purposeful.**  `tempfile` and
  `tokio-tungstenite` are present, indicating WebSocket-level integration tests
  exist or are planned at the crate boundary without bloating production builds.

### Issues

**[SEVERITY: Medium]** No unit tests for `AgentHandler::on_message` machine-ID
validation logic

The machine-ID mismatch guard in `src/main.rs:63-69`, `74-80`, and `87-93` is a
security-relevant behavior.  There are no inline tests verifying that a mismatched
`host_machine_id` causes the message to be silently dropped (`Ok(None)`) while a
matching ID allows processing.  This is the most important correctness property
owned entirely by this crate (rather than by the underlying libraries), and it has
no direct test coverage here.

**[SEVERITY: Low]** `host_info.rs:135` — `collect_host_info_returns_valid_data`
asserts `hostname.is_some()` unconditionally

The test will fail on a container or CI environment where `hostname` is not in
`PATH` or returns an empty string.  The production code correctly returns `None` in
that case; the test should tolerate `None` or use `#[cfg]` guards to reflect
platform availability.

---

## High Availability

### Strengths

- **`in_flight_update` serialization prevents concurrent update races.**  Because
  `handle_execute_update` in `uptrakit-agent-core` checks the option before
  spawning, a second controller dispatch while an update is running is rejected at
  the agent without requiring any coordinator on the controller side.

- **`on_shutdown` consumes `in_flight_update` via `.take()`.**  The graceful
  shutdown path hands the in-flight state to `handle_graceful_shutdown` exactly
  once.  The pattern prevents both a double-free and a missed wait on the running
  task.

- **`Signal::Hangup` maps to `LoopOutcome::Restart`.**  SIGHUP triggers a clean
  reconnect cycle rather than full process exit, enabling zero-downtime certificate
  rotation initiated by the controller.

### Issues

**[SEVERITY: Low]** Inherited — `crates/shared/service-sdk/src/lifecycle.rs:263-275`
— Enrollment retry does not catch transient network errors

DNS resolution failures, TCP timeouts, and TLS handshake errors during enrollment
cause the process to exit immediately rather than retrying with backoff.  Only
`ReceiveClosed` triggers a retry loop.  In practice this means a transient network
blip during the enrollment window requires an external process supervisor (systemd,
launchd) to restart the agent.

---

## Database

N/A — thin binary entry point; no database dependency exists in this crate.
All persistence concerns apply to `uptrakit-shared-db`, `uptrakit-web-api`, and
the controller-side query layer.

---

## Coding Standards

### Strengths

- **`edition = "2024"` and workspace-pinned versions.**  All six runtime
  dependencies are declared via `{ workspace = true }` or `{ path = "..." }` with
  no inline version strings, preventing drift.

- **No `#[allow(clippy::...)]` suppressions.**  Consistent with AGENTS.md policy.

- **`disable_version_flag = true` in `cli.rs`.**  The binary implements its own
  `--version` via `print_build_info`, which includes build metadata beyond the
  semver string.  Disabling Clap's default flag prevents a confusing parallel
  `--version` path.

- **`tracing::warn!` structured fields.**  The machine-ID mismatch warnings
  include named fields (`expected`, `received`) rather than interpolated strings,
  which makes them machine-parseable by log aggregators.

### Issues

**[SEVERITY: Low]** No `rust-version` in `Cargo.toml` or `[workspace.package]`

Consistent with the rest of the workspace (none of the 24 crates set MSRV), but
AGENTS.md states that `rust-version = "1.91"` should be present.  Setting it in
`[workspace.package]` would enforce the minimum toolchain and make CI failures
explicit rather than silent.

---

## Extensibility

### Strengths

- **`agent_capabilities()` is a single source of truth for this binary.**  Both
  `on_connected` (which sends capabilities in the wire payload) and
  `capabilities()` (which returns them to the SDK for capability intersection) call
  the same free function.  Adding or removing a capability requires a single edit.

- **`ServiceHandler` trait makes adding a new agent type straightforward.**  The
  pattern is demonstrated here: implement four async methods, declare three
  constants, and pass to `run_lifecycle_and_handle_errors`.  The SSH agent
  (`uptrakit-agent-ssh`) follows the identical pattern.

- **`client.rs` shim layer is a clean extension point.**  Each handler function in
  `client.rs` constructs a `LocalCommandExecutor` and forwards to
  `uptrakit-agent-core`.  A future agent variant (e.g., one using a sandboxed or
  remote executor) can substitute the executor without modifying `main.rs` or the
  `AgentHandler` dispatch logic.

#### 2026-02-24 Review

##### Strengths

- **SSH agent correctly reuses `CommandExecutor` abstraction for remote operations.** `crates/shared/command/src/executor.rs:142-156` — The `CommandExecutor` trait accommodates future SSH-based or container-based executors. All four providers receive `Arc<dyn CommandExecutor>`.
