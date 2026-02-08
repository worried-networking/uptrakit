# Agent Crate Code Review

**Date**: 2026-02-08
**Reviewer**: Claude Opus 4.6
**Branch**: `refactor/codereview-agent`
**Scope**: Full review of `crates/core/agent/` and key shared dependencies

## Files Reviewed

| File | Lines | Purpose |
|------|-------|---------|
| `src/main.rs` | 274 | Entry point, enrollment flow, reconnect loop |
| `src/client.rs` | 677 | Authenticated event loop (mTLS) |
| `src/update.rs` | 765 | Update execution, hooks, shell commands |
| `src/error.rs` | 62 | Error types and conversions |
| `src/host_info.rs` | 107 | Host information collection |
| `src/version_check.rs` | 74 | Version detection dispatch |
| `src/cli.rs` | 184 | CLI argument struct + tests |
| `Cargo.toml` | 33 | Dependencies |

Plus shared crates: `wire`, `enrollment`, `directories`, `provider-registry`, `provider-core`.

---

## Summary of Findings

| # | Severity | Category | Issue | Status |
|---|----------|----------|-------|--------|
| 1 | **Critical** | Security | Shell injection in update command construction (4 locations) | Open |
| 2 | Medium | Safety | Blocking sync I/O in async context (4 locations) | Open |
| 3 | Medium | Reliability | No reconnection backoff (thundering herd risk) | Open |
| 4 | Medium | Safety | Unbounded output accumulation (OOM risk) | Open |
| 5 | Low | Performance | Sequential version checks | Open |
| 6 | Low | Maintenance | String-based error detection | Open |
| 7 | Low | Debugging | Silently swallowed task panics | Open |
| 8 | Medium | Architecture | Duplicate update dispatch (provider trait vs hardcoded) | Open |
| 9 | Low | Cleanup | Unused direct dependencies (`serde`, `provider-core`) | Open |
| 10 | Medium | Quality | No tests for `client.rs`, `main.rs`, `error.rs` | Open |
| 11a | Low | Completeness | `detect_current_version` is a no-op | Open |
| 11b | Low | Safety | `extract_service_id` silently falls back to "unknown" | Open |
| 11c | Low | Design | Agent sends empty MQTT-specific field | Open |

---

## Detailed Findings

### 1. CRITICAL: Shell Injection Vulnerabilities in `update.rs`

**Severity: Critical** | Violates AGENTS.md rule #5: *"No shell injection. Any path that constructs or executes shell commands must validate inputs."*

#### 1a. `execute_docker_registry_update` (line 387)

```rust
let pull_cmd = format!("docker pull {image}:{tag}");
```

Both `image` (`payload.package_identifier`) and `tag` (`payload.to_version`) come from the wire protocol. If `to_version` were `"2.0.0; rm -rf /"`, the resulting command becomes `docker pull nginx:2.0.0; rm -rf /`. The command is passed to `run_command` -> `run_command_with_shell` -> `Command::new("bash").arg("-c").arg(...)`, which evaluates shell metacharacters.

#### 1b. `execute_github_releases_update` (lines 297-301)

```rust
let cmd = cmd_str
    .replace("{version}", &payload.to_version)
    .replace("{tag}", &release_info.tag)
    .replace("{package_identifier}", &payload.package_identifier);
```

Variable substitution into a shell command without escaping. If any substituted value contains shell metacharacters (`;`, `$()`, backticks, `|`, `&&`), arbitrary code execution occurs.

#### 1c. `execute_proxmox_helper_scripts_update` (line 355)

```rust
let cmd = format!("bash -c \"$(curl -fsSL {script_url})\" -- --update");
```

`script_url` comes from `provider_config`, which is controlled by controller admins. But the URL is interpolated directly into a shell string with double-quote escaping that can be broken.

#### 1d. `restart_command` substitution (lines 401-404)

```rust
let cmd = cmd_str
    .replace("{image}", image)
    .replace("{tag}", tag)
    .replace("{version}", &payload.to_version);
```

Same pattern as 1b.

**Note:** The trust boundary here is the mTLS-authenticated controller, so these are not exploitable by external attackers. However, the AGENTS.md rule is unconditional and exists for defense-in-depth. A compromised controller database or admin account could leverage these to escalate beyond the sudo allowlist model. All substituted values should be shell-escaped or commands should use `Command::new("docker").args(["pull", &format!("{image}:{tag}")])` (no shell) instead of shell interpolation.

---

### 2. Blocking Synchronous I/O in Async Context

**Severity: Medium** | Can stall the tokio runtime.

#### 2a. `save_renewed_cert` (`client.rs:649-652`)

```rust
std::fs::write(&cert_path, &payload.cert_pem).context_to::<Error>()?;
// ...
std::fs::write(&key_path, key_pem).context_to::<Error>()?;
```

#### 2b. `save_ca_cert_sync` (`client.rs:659`)

```rust
std::fs::write(&path, pem).context_to::<Error>()?;
```

#### 2c. `compute_local_ca_hash` (`client.rs:623`)

```rust
match std::fs::read(&ca_path) {
```

#### 2d. `set_secure_permissions` (`client.rs:668`)

```rust
std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
```

All these are called from inside the main `tokio::select!` loop. On slow filesystems (NFS, encrypted volumes), these block the entire async task. They should use `tokio::fs` equivalents or be wrapped in `tokio::task::spawn_blocking`.

---

### 3. No Reconnection Backoff

**Severity: Medium** | Can overwhelm the controller.

In `main.rs:149`:

```rust
tokio::time::sleep(Duration::from_secs(2)).await;
```

And `main.rs:260`:

```rust
tokio::time::sleep(Duration::from_secs(2)).await;
```

Both the enrollment retry loop and the `run_authenticated_with_reconnect` loop use a fixed 2-second delay. If the controller is down for an extended period, hundreds of agents hammering every 2 seconds creates a thundering herd. Exponential backoff with jitter (e.g., 2s, 4s, 8s, ..., capped at 60s with random jitter) is standard practice and is already identified in TODO.md Phase 8 under "Error Recovery."

---

### 4. Unbounded Output Accumulation

**Severity: Medium** | Memory exhaustion possible.

In `update.rs`, `accumulated_output` (`String`) and the per-command output buffers grow without limit:

```rust
let mut accumulated_output = String::new();
// ...
accumulated_output.push_str(&output);  // called for every command phase
```

Also in `run_command_with_shell` (lines 494-495, 512-513):

```rust
output.push_str(&line);
output.push('\n');
```

A runaway command producing gigabytes of output would exhaust agent memory. A reasonable cap (e.g., 10 MB) with truncation marker would prevent OOM kills.

---

### 5. Sequential Version Checks

**Severity: Low** | Performance concern.

In `client.rs:328`:

```rust
for assignment in &payload.assignments {
    // ...
    let (installed_version, error) = crate::version_check::check_version(...).await;
    results.push(...);
}
```

Version checks are executed sequentially. For a large number of software items, this could take a long time. Since each check is independent, they could be run concurrently with a bounded concurrency limit (e.g., `futures::stream::iter(...).buffer_unordered(8)`). However, since all current provider local implementations are stubs returning `Ok(None)` immediately, this is a low priority concern for now.

---

### 6. Fragile String-Based Error Detection

**Severity: Low** | Maintenance risk.

In `error.rs:47-48`:

```rust
Error::WebSocket(e) => e.to_string().contains("CertificateExpired"),
Error::Io(e) => e.to_string().contains("CertificateExpired"),
```

This relies on the exact string representation of rustls errors, which could change across library versions. A more robust approach would pattern-match on the underlying rustls error types.

---

### 7. Silently Swallowed Task Panics

**Severity: Low** | Debugging difficulty.

In `run_command_with_shell` (`update.rs:519-520`):

```rust
accumulated.push_str(&stdout_output.unwrap_or_default());
accumulated.push_str(&stderr_output.unwrap_or_default());
```

If the stdout/stderr reader tasks panic, `unwrap_or_default()` silently swallows the error. At minimum, a `tracing::warn!` on `Err` would help debug issues. Similarly, `send_update_result` in `client.rs:509` already handles JoinError correctly for the main update task, which is good.

---

### 8. Duplicate Update Dispatch Architecture

**Severity: Medium** | Architectural concern.

There are two parallel update execution paths:

1. **Provider trait** (`Provider::execute_update` in `provider-core`) - all implementations return `Err("not yet implemented")`
2. **Agent's `update.rs`** - `execute_provider_update` dispatches by `ProviderType` with hardcoded shell commands

This means the provider abstraction (which the version check path uses via `ProviderRegistry::create_local_provider`) is completely bypassed for updates. The update logic is hardcoded in the agent instead of being delegated to the provider implementations. This creates:

- Inconsistent dispatch (version check goes through provider registry; update goes through agent's switch statement)
- The provider registry's local providers aren't used for their intended purpose
- Adding a new provider requires changes in two places

---

### 9. Unnecessary Direct Dependencies

**Severity: Low** | Cleanup.

In `Cargo.toml`:

- `serde = { workspace = true }` - No `Serialize`/`Deserialize` derives or direct serde usage in any agent source file. Only `serde_json` is used.
- `uptrakit-provider-core = { path = "../../providers/core" }` - Not imported in any agent source file. The agent uses `uptrakit-provider-registry` (which re-exports what's needed) and `uptrakit-internal-wire` (which re-exports `ProviderType`).

---

### 10. Missing Test Coverage

**Severity: Medium** | Quality concern.

| Module | Test coverage |
|--------|--------------|
| `client.rs` (677 lines) | **Zero tests** |
| `main.rs` (274 lines) | **Zero tests** |
| `update.rs` (765 lines) | Good coverage for shell wrapping and hooks |
| `error.rs` (62 lines) | **Zero tests** (is_receive_closed, is_cert_expired untested) |
| `host_info.rs` (107 lines) | One test |
| `version_check.rs` (74 lines) | Good coverage |
| `cli.rs` (184 lines) | Good coverage |

The most complex module (`client.rs`) with the main event loop, renewal logic, graceful shutdown, and CA bundle handling has no tests at all. Key untested paths:

- `compute_renewal_delay` with various edge cases
- `handle_graceful_shutdown` (timeout path, output drain)
- Sequence validation failure handling
- Certificate rotation/reconnect flow
- CA bundle hash mismatch handling

---

### 11. Minor Issues

#### 11a. `detect_current_version` is a no-op (update.rs:247-251)

```rust
async fn detect_current_version(_payload: &ExecuteUpdatePayload) -> Option<String> {
    // TODO: Implement actual version detection based on provider_type
    None
}
```

Both `from_version` and `to_version` detection always return `None`, making `UpdateResultPayload.from_version` and `.to_version` useless. This is acknowledged with a TODO.

#### 11b. `extract_service_id` fallback (client.rs:638)

```rust
.unwrap_or_else(|| "unknown".to_string())
```

If called when service_id is `None`, the CSR common name becomes "unknown", which could cause certificate issuance issues. This should be an error rather than a silent fallback, since it's only called during certificate renewal when the identity should always have a service_id.

#### 11c. `active_mqtt_clients: vec![]` hardcoded (client.rs:607)

```rust
let disconnecting_msg = ServiceMessage::Disconnecting(DisconnectingPayload {
    reason: disconnect_reason,
    active_mqtt_clients: vec![],
});
```

This is correct for the agent (it has no MQTT clients), but the payload structure coupling is a minor smell. The wire protocol requires agents to send an empty vec for an MQTT-specific field.

---

## What's Done Well

- Clean separation of enrollment logic into a shared crate
- Proper use of `rootcause` + `thiserror` error handling pattern
- Replay protection via sequence numbers on the wire protocol
- Graceful shutdown with in-flight update awareness and configurable timeout
- mTLS certificate lifecycle (renewal timers, CA bundle updates, expired cert fallback)
- Good test coverage in `cli.rs`, `version_check.rs`, and `update.rs` shell tests
- Proper secure file permissions (0o600) on sensitive files
- The `biased` select prioritization is correct for update output responsiveness
- `InFlightUpdate` struct properly tracks only one update at a time

---

## Fix Plans (Top 5)

### Fix Plan 1: Shell Injection Prevention (Finding #1)

**Goal**: Eliminate all shell injection vectors in `update.rs`.

**Approach**: Replace shell string interpolation with direct `Command::new(...).args(...)` invocations (no shell) for provider-specific commands. For user-defined hook commands that inherently require shell interpretation, add a `shell_escape` utility.

**Changes**:

1. **`execute_docker_registry_update`** - Replace `format!("docker pull {image}:{tag}")` passed through bash with:
   ```rust
   Command::new("docker")
       .args(["pull", &format!("{image}:{tag}")])
       .stdout(Stdio::piped())
       .stderr(Stdio::piped())
       .spawn()
   ```
   Extract a new `run_command_direct` helper that takes a program and args (no shell), streams output the same way as `run_command_with_shell`, and returns the same `(String, i32)` tuple.

2. **`execute_proxmox_helper_scripts_update`** - Replace the `bash -c "$(curl ...)"` interpolation with:
   ```rust
   Command::new("bash")
       .args(["-c", &format!(
           "set -euo pipefail\ncurl -fsSL -- \"$1\" | bash -s -- --update",
       ), "--", script_url])
       .spawn()
   ```
   Pass the URL as a positional argument (`$1`) so it is never interpreted as shell syntax.

3. **`execute_github_releases_update`** - For the `install_command` template substitution, add a `shell_escape()` function that wraps values in single quotes with proper escaping (`'` -> `'\''`). Apply it to all substituted values:
   ```rust
   let cmd = cmd_str
       .replace("{version}", &shell_escape(&payload.to_version))
       .replace("{tag}", &shell_escape(&release_info.tag))
       .replace("{package_identifier}", &shell_escape(&payload.package_identifier));
   ```

4. **`restart_command` substitution** - Same `shell_escape()` treatment as #3.

5. **Add `shell_escape` function** at the top of `update.rs`:
   ```rust
   fn shell_escape(s: &str) -> String {
       format!("'{}'", s.replace('\'', "'\\''"))
   }
   ```

6. **Add tests** for each injection vector:
   - `test_docker_pull_with_semicolon_in_tag`
   - `test_github_install_cmd_with_backtick_in_version`
   - `test_proxmox_script_url_with_shell_metacharacters`
   - `test_shell_escape_function`

**Files modified**: `src/update.rs`

---

### Fix Plan 2: Async File I/O (Finding #2)

**Goal**: Replace all blocking `std::fs` calls with async equivalents.

**Changes**:

1. **Convert `save_renewed_cert` to async** (`client.rs`):
   ```rust
   async fn save_renewed_cert(...) -> Result<()> {
       tokio::fs::write(&cert_path, &payload.cert_pem).await.context_to::<Error>()?;
       set_secure_permissions(&cert_path).await?;
       tokio::fs::write(&key_path, key_pem).await.context_to::<Error>()?;
       set_secure_permissions(&key_path).await?;
       Ok(())
   }
   ```

2. **Convert `save_ca_cert_sync` to `save_ca_cert`** (rename, make async):
   ```rust
   async fn save_ca_cert(config_dir: &std::path::Path, pem: &[u8]) -> Result<()> {
       let path = config_dir.join("ca.pem");
       tokio::fs::write(&path, pem).await.context_to::<Error>()?;
       set_secure_permissions(&path).await?;
       Ok(())
   }
   ```

3. **Convert `set_secure_permissions` to async**:
   ```rust
   async fn set_secure_permissions(path: &std::path::Path) -> Result<()> {
       #[cfg(unix)]
       {
           use std::os::unix::fs::PermissionsExt;
           tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
               .await
               .context_to::<Error>()?;
       }
       Ok(())
   }
   ```

4. **Convert `compute_local_ca_hash` to async**:
   ```rust
   async fn compute_local_ca_hash(config_dir: &std::path::Path) -> String {
       let ca_path = config_dir.join("ca.pem");
       match tokio::fs::read(&ca_path).await {
           Ok(bytes) => { /* same hash logic */ }
           Err(_) => String::new(),
       }
   }
   ```

5. **Update all call sites** in `run_authenticated_loop` to `.await` the now-async functions.

**Files modified**: `src/client.rs`

---

### Fix Plan 3: Reconnection Backoff with Jitter (Finding #3)

**Goal**: Replace fixed 2-second delays with exponential backoff + jitter.

**Changes**:

1. **Add a `Backoff` struct** in a new `src/backoff.rs` module:
   ```rust
   pub struct Backoff {
       current: Duration,
       base: Duration,
       max: Duration,
   }

   impl Backoff {
       pub fn new(base: Duration, max: Duration) -> Self { ... }
       pub fn next_delay(&mut self) -> Duration {
           let delay = self.current;
           self.current = (self.current * 2).min(self.max);
           // Add jitter: random value in [0, delay/4]
           let jitter = Duration::from_millis(
               rand::random::<u64>() % (delay.as_millis() as u64 / 4).max(1)
           );
           delay + jitter
       }
       pub fn reset(&mut self) { self.current = self.base; }
   }
   ```

2. **Update the enrollment retry loop** in `main.rs`:
   ```rust
   let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
   loop {
       match do_enrollment(...).await {
           Ok(()) => break,
           Err(mut e) => {
               if e.current_context_mut().is_receive_closed() {
                   let delay = backoff.next_delay();
                   tracing::info!("disconnected during enrollment, reconnecting in {delay:?}");
                   tokio::time::sleep(delay).await;
                   identity.load().await.context_to::<Error>()?;
                   continue;
               }
               return Err(e);
           }
       }
   }
   ```

3. **Update `run_authenticated_with_reconnect`** in `main.rs`:
   ```rust
   let mut backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
   loop {
       // ... build connector ...
       match client::run_authenticated_loop(...).await? {
           LoopOutcome::Reconnect => {
               backoff.reset(); // cert rotation is expected, reset backoff
               tracing::info!("reconnecting with new certificate");
               tokio::time::sleep(Duration::from_secs(2)).await;
               continue;
           }
           LoopOutcome::Disconnected => {
               let delay = backoff.next_delay();
               tracing::warn!("disconnected, reconnecting in {delay:?}");
               tokio::time::sleep(delay).await;
               continue;
           }
           LoopOutcome::Shutdown | LoopOutcome::Restart => return Ok(()),
       }
   }
   ```
   Note: `Disconnected` currently returns `Ok(())` and exits. This should be changed to retry with backoff instead of silently exiting, since disconnection is often transient.

4. **Add `rand` dependency** to `Cargo.toml` for jitter (or use a simple LCG to avoid the dependency).

5. **Add tests** for the `Backoff` struct: doubling behavior, max cap, reset, jitter range.

**Files modified**: `src/main.rs`, new `src/backoff.rs`, `Cargo.toml`

---

### Fix Plan 4: Bounded Output Accumulation (Finding #4)

**Goal**: Cap accumulated output to prevent OOM from runaway commands.

**Changes**:

1. **Add a constant** in `update.rs`:
   ```rust
   const MAX_OUTPUT_BYTES: usize = 10 * 1024 * 1024; // 10 MB
   const TRUNCATION_MARKER: &str = "\n... [output truncated at 10 MB] ...\n";
   ```

2. **Add a helper** to safely append to a bounded buffer:
   ```rust
   fn append_bounded(buffer: &mut String, text: &str, max: usize) {
       if buffer.len() >= max {
           return;
       }
       let remaining = max - buffer.len();
       if text.len() <= remaining {
           buffer.push_str(text);
       } else {
           buffer.push_str(&text[..remaining]);
           buffer.push_str(TRUNCATION_MARKER);
       }
   }
   ```

3. **Replace all `accumulated_output.push_str(&output)`** calls with `append_bounded(&mut accumulated_output, &output, MAX_OUTPUT_BYTES)`.

4. **Apply the same cap inside `run_command_with_shell`** for the per-command stdout/stderr buffers. Add a length check in the line-reading loop:
   ```rust
   while let Ok(Some(line)) = lines.next_line().await {
       if output.len() < MAX_OUTPUT_BYTES {
           output.push_str(&line);
           output.push('\n');
       }
       // Always stream the line even if accumulation is capped
       let _ = output_tx.send(...).await;
   }
   ```

5. **Add tests**:
   - `test_output_truncation_at_limit` - verify truncation marker appears
   - `test_output_below_limit` - verify full output preserved

**Files modified**: `src/update.rs`

---

### Fix Plan 5: Test Coverage for `client.rs` (Finding #10)

**Goal**: Add unit tests for the critical pure functions and key logic paths in `client.rs`.

**Changes**:

1. **Extract testable pure functions** that are currently private. Make them `pub(crate)` or add a `#[cfg(test)]` module with direct access:

   - `compute_renewal_delay(cert_not_after_ts, window_hours) -> Duration` (already a standalone fn)
   - `compute_local_ca_hash(config_dir) -> String` (already a standalone fn)
   - `extract_service_id(identity) -> String` (already a standalone fn)

2. **Add `#[cfg(test)] mod tests`** block in `client.rs` with:

   ```rust
   #[test]
   fn renewal_delay_future_cert() {
       // cert expires in 30 days, window is 168h (7 days) -> delay ~23 days
   }

   #[test]
   fn renewal_delay_already_in_window() {
       // cert expires in 3 days, window is 168h -> delay is 0
   }

   #[test]
   fn renewal_delay_no_cert() {
       // cert_not_after_ts is None -> FAR_FUTURE
   }

   #[test]
   fn renewal_delay_expired_cert() {
       // cert already expired -> delay is 0 (max(0) clamp)
   }

   #[test]
   fn local_ca_hash_missing_file() {
       // non-existent config dir -> empty string
   }

   #[test]
   fn local_ca_hash_valid_file() {
       // temp dir with a known ca.pem -> verify exact SHA-256 hex
   }

   #[test]
   fn extract_service_id_with_id() {
       // identity with service_id set -> UUID string
   }

   #[test]
   fn extract_service_id_without_id() {
       // identity with no service_id -> "unknown"
   }
   ```

3. **Add tests for `error.rs`**:
   ```rust
   #[test]
   fn is_receive_closed_enrollment_error() { ... }

   #[test]
   fn is_receive_closed_other_error() { ... }

   #[test]
   fn is_cert_expired_websocket_error() { ... }

   #[test]
   fn is_cert_expired_unrelated_error() { ... }
   ```

4. **Add `tempfile` usage** for filesystem-based tests (already in `[dev-dependencies]`).

**Files modified**: `src/client.rs`, `src/error.rs`

---

### Fix Plan 6: Unify Update Dispatch Through Provider Registry (Finding #8)

**Goal**: Eliminate the duplicate dispatch in `update.rs` by routing update execution through the provider registry, consistent with how version checks already work.

**Problem**: Version checks dispatch through `ProviderRegistry::create_local_provider()` -> `Provider::detect_installed_version()`, but updates bypass the registry entirely. The agent has three hardcoded `execute_*_update` functions (`execute_github_releases_update`, `execute_proxmox_helper_scripts_update`, `execute_docker_registry_update`) that duplicate the dispatch, while the actual `Provider::execute_update()` implementations all return `"not yet implemented"` errors.

**Approach**: Move the update execution logic from the agent's `update.rs` into each provider's `execute_update` implementation. The agent's `execute_provider_update` function then becomes a thin dispatch through the registry, identical in shape to the version check path.

**Changes**:

1. **Extend the `Provider` trait** in `crates/providers/core/src/traits.rs` with an update method that accepts the richer `ExecuteUpdatePayload` data. The current `execute_update(&self, release: &UpstreamRelease)` signature is too narrow -- it doesn't carry `provider_config`, `package_identifier`, or `to_version`. Add a new method with a struct parameter:
   ```rust
   /// Context for executing a software update on the local system.
   pub struct UpdateContext {
       pub to_version: String,
       pub package_identifier: String,
       pub provider_config: serde_json::Value,
       pub release_info: Option<ReleaseInfo>,  // re-export from wire or define locally
   }

   /// Execute an update with full context (local operation).
   async fn execute_update_with_context(
       &self,
       ctx: &UpdateContext,
       output_tx: &tokio::sync::mpsc::Sender<UpdateOutputLine>,
   ) -> Result<String>;
   ```
   Keep the existing `execute_update(&self, _release: &UpstreamRelease)` as a deprecated default for backward compatibility during migration.

2. **Move `execute_github_releases_update` logic** into `GitHubLocalProvider::execute_update_with_context()` in `crates/providers/github/src/local_provider.rs`. The logic for variable substitution, `install_command` lookup, and fallback stays the same but lives in the provider crate where it belongs.

3. **Move `execute_docker_registry_update` logic** into `DockerRegistryLocalProvider::execute_update_with_context()` in `crates/providers/docker-registry/src/local_provider.rs`. The `docker pull` command, `restart_command` handling all move here.

4. **Move `execute_proxmox_helper_scripts_update` logic** into `ProxmoxHelperScriptsLocalProvider::execute_update_with_context()` in `crates/providers/proxmox-helper-scripts/src/local_provider.rs`. The `curl | bash` script execution moves here.

5. **Simplify `execute_provider_update` in agent's `update.rs`** to:
   ```rust
   async fn execute_provider_update(
       payload: &ExecuteUpdatePayload,
       output_tx: &mpsc::Sender<UpdateOutputMessage>,
   ) -> UpdateResult<String> {
       let provider = ProviderRegistry::create_local_provider(
           payload.provider_type,
           &payload.package_identifier,
           &payload.provider_config,
       ).map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))?;

       let ctx = UpdateContext {
           to_version: payload.to_version.clone(),
           package_identifier: payload.package_identifier.clone(),
           provider_config: payload.provider_config.clone(),
           release_info: payload.release_info.clone(),
       };
       provider.execute_update_with_context(&ctx, output_tx)
           .await
           .map_err(|e| report!(UpdateError::InstallFailed(e.to_string())))
   }
   ```

6. **Move `run_command`, `run_command_with_shell`, `shell_escape`** helpers into either `provider-core` (as a `command` utility module) or keep them in the agent and re-export. The provider crates need access to shell execution. A shared `uptrakit-provider-core::command` module is cleanest since multiple providers need it.

7. **Move existing tests** from agent's `update.rs` to the respective provider crate test modules.

**Files modified**: `crates/providers/core/src/traits.rs`, `crates/providers/github/src/local_provider.rs`, `crates/providers/docker-registry/src/local_provider.rs`, `crates/providers/proxmox-helper-scripts/src/local_provider.rs`, `crates/core/agent/src/update.rs`, potentially new `crates/providers/core/src/command.rs`

**Note**: This is the largest refactor and should be combined with Fix Plan 1 (shell injection) since the command construction logic is being moved anyway.

---

### Fix Plan 7: Robust TLS Error Detection (Finding #6)

**Goal**: Replace fragile `e.to_string().contains("CertificateExpired")` with structured error matching.

**Problem**: Both `crates/shared/enrollment/src/error.rs:85-91` and `crates/core/agent/src/error.rs:44-51` detect certificate expiry by stringifying errors and searching for the substring `"CertificateExpired"`. This is:
- Fragile: the string representation could change across `rustls` or `tungstenite` versions.
- Slow: allocates a String and performs a substring search on every error check.
- Incorrect if the string appears in a different context (e.g., an error message mentioning `CertificateExpired` as a diagnostic).

**Root cause chain**: When the TLS handshake fails with a `CertificateExpired` alert:
1. `rustls` produces `rustls::Error::AlertReceived(AlertDescription::CertificateExpired)`
2. This is wrapped in `std::io::Error` (custom kind)
3. `tungstenite` wraps it in `tungstenite::Error::Io(io_err)`

**Changes**:

1. **Add a helper function** in `crates/shared/enrollment/src/error.rs` that downcasts through the error chain:
   ```rust
   /// Check if an `std::io::Error` wraps a rustls `CertificateExpired` alert.
   fn is_rustls_cert_expired(io_err: &std::io::Error) -> bool {
       // rustls wraps its errors as io::Error via io::Error::new(io::ErrorKind::*, rustls_err)
       if let Some(inner) = io_err.get_ref() {
           if let Some(rustls_err) = inner.downcast_ref::<rustls::Error>() {
               return matches!(
                   rustls_err,
                   rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired)
               );
           }
       }
       false
   }
   ```

2. **Update `EnrollmentError::is_cert_expired`** to use the structured check:
   ```rust
   pub fn is_cert_expired(&self) -> bool {
       match self {
           EnrollmentError::Rustls(rustls::Error::AlertReceived(
               rustls::AlertDescription::CertificateExpired,
           )) => true,
           EnrollmentError::WebSocket(tungstenite::Error::Io(io_err)) => {
               is_rustls_cert_expired(io_err)
           }
           EnrollmentError::Io(io_err) => is_rustls_cert_expired(io_err),
           _ => false,
       }
   }
   ```

3. **Update agent's `Error::is_cert_expired`** to delegate properly:
   ```rust
   pub fn is_cert_expired(&self) -> bool {
       match self {
           Error::Enrollment(e) => e.is_cert_expired(),
           Error::WebSocket(tungstenite::Error::Io(io_err)) => {
               is_rustls_cert_expired(io_err)
           }
           Error::Io(io_err) => is_rustls_cert_expired(io_err),
           _ => false,
       }
   }
   ```
   Import or duplicate the `is_rustls_cert_expired` helper. Since both crates need it, add it as a `pub fn` in the enrollment crate and import it in the agent.

4. **Add tests** covering:
   - Construct a `rustls::Error::AlertReceived(CertificateExpired)`, wrap it in `io::Error`, wrap that in `tungstenite::Error::Io`, wrap that in `EnrollmentError::WebSocket` -- verify `is_cert_expired()` returns `true`.
   - Same chain with a different alert (e.g., `HandshakeFailure`) -- verify `false`.
   - A plain `io::Error` (not wrapping rustls) -- verify `false`.
   - `EnrollmentError::Rustls(AlertReceived(CertificateExpired))` -- verify `true`.

**Files modified**: `crates/shared/enrollment/src/error.rs`, `crates/core/agent/src/error.rs`

---

### Fix Plan 8: Log Swallowed Task Panics in Output Readers (Finding #7)

**Goal**: Make stdout/stderr reader task panics visible in logs instead of silently dropping them.

**Problem**: In `run_command_with_shell` (`update.rs:517-520`):
```rust
let (stdout_output, stderr_output) = tokio::join!(stdout_handle, stderr_handle);
accumulated.push_str(&stdout_output.unwrap_or_default());
accumulated.push_str(&stderr_output.unwrap_or_default());
```

If a spawned reader task panics (e.g., due to a bug in line processing), the `JoinError` is silently converted to an empty string. The update appears to succeed with missing output, making debugging extremely difficult.

**Changes**:

1. **Replace `unwrap_or_default()` with explicit match** that logs on `Err`:
   ```rust
   let (stdout_output, stderr_output) = tokio::join!(stdout_handle, stderr_handle);

   match stdout_output {
       Ok(out) => accumulated.push_str(&out),
       Err(e) => {
           tracing::error!(error = %e, "stdout reader task failed");
           accumulated.push_str("[stdout reader failed]\n");
       }
   }
   match stderr_output {
       Ok(out) => accumulated.push_str(&out),
       Err(e) => {
           tracing::error!(error = %e, "stderr reader task failed");
           accumulated.push_str("[stderr reader failed]\n");
       }
   }
   ```

2. **Add a test** that verifies normal operation still works (existing tests already cover this, but explicitly confirm the match arms don't change behavior for the success path).

**Files modified**: `src/update.rs`

---

### Fix Plan 9: Remove Unused Direct Dependencies (Finding #9)

**Goal**: Clean up `Cargo.toml` to remove dependencies not directly used by agent source code.

**Problem**: Two dependencies are listed but never imported in agent source files:
- `serde = { workspace = true }` -- no `#[derive(Serialize, Deserialize)]` or `use serde::*` in any agent `.rs` file. The agent only uses `serde_json` for serialization/deserialization of wire protocol types (which are defined in the `wire` crate, not in the agent).
- `uptrakit-provider-core = { path = "../../providers/core" }` -- no `use uptrakit_provider_core::*` anywhere. The agent uses `uptrakit_provider_registry` (which re-exports `Provider`, `ProviderType`, etc.) and `uptrakit_internal_wire` (which re-exports `ProviderType`).

**Verification steps before removal**:

1. **`serde`**: Run `grep -r "use serde" crates/core/agent/src/` and `grep -r "Serialize\|Deserialize" crates/core/agent/src/` to confirm zero direct usage. Then remove and verify `cargo check -p uptrakit-agent` passes.

2. **`uptrakit-provider-core`**: Run `grep -r "uptrakit_provider_core" crates/core/agent/src/` to confirm zero direct imports. Then remove and verify `cargo check -p uptrakit-agent` passes.

**Changes**:

1. **Remove from `Cargo.toml`**:
   ```diff
   -serde = { workspace = true }
   -uptrakit-provider-core = { path = "../../providers/core" }
   ```

2. **Run quality gates**:
   ```sh
   cargo check -p uptrakit-agent --all-features
   cargo clippy -p uptrakit-agent --all-targets --all-features -- -D warnings
   cargo test -p uptrakit-agent --all-features
   ```

**Files modified**: `Cargo.toml`

---

### Fix Plan 10: Fail on Missing Service ID During Renewal (Finding #11b)

**Goal**: Replace the silent `"unknown"` fallback in `extract_service_id` with an explicit error, since a missing service ID during certificate renewal is a logic error that should not silently produce an invalid CSR.

**Problem**: In `client.rs:634-639`:
```rust
fn extract_service_id(identity: &uptrakit_enrollment::ServiceIdentityState) -> String {
    identity
        .service_id()
        .map(|id| id.to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
```

This function is called in exactly two places -- both during certificate renewal (`renewal_sleep` branch at line 189 and `RequestCertRenewal` handler at line 304). At these points in the lifecycle, the agent is in the authenticated mTLS loop, meaning it **must** have a valid service ID (it was assigned during enrollment and is required to reach this state). If `service_id()` returns `None`, something is fundamentally broken.

Generating a CSR with common name `"unknown"` would:
- Produce a certificate that doesn't match the agent's identity
- Potentially be rejected by the controller's certificate signing logic
- Cause confusing failures later when the mismatched cert is used

**Changes**:

1. **Change `extract_service_id` to return `Result<String>`**:
   ```rust
   fn extract_service_id(
       identity: &uptrakit_enrollment::ServiceIdentityState,
   ) -> Result<String> {
       identity
           .service_id()
           .map(|id| id.to_string())
           .ok_or_else(|| report!(Error::Enrollment(
               uptrakit_enrollment::EnrollmentError::NotEnrolled
           )))
   }
   ```

2. **Update call site at renewal timer** (line ~189-191):
   ```rust
   let client_id_str = match extract_service_id(identity) {
       Ok(id) => id,
       Err(e) => {
           tracing::error!(error = %e, "cannot renew certificate: no service ID");
           break LoopOutcome::Disconnected;
       }
   };
   ```

3. **Update call site at `RequestCertRenewal`** (line ~304) -- same pattern, already has an error branch that breaks to `Disconnected`.

4. **Add a test** confirming the function returns an error when identity has no service_id.

**Files modified**: `src/client.rs`

---

### Fix Plan 11 — Concurrent Version Checks (Finding #5)

**Goal**: Replace the serial `for` loop in `check_versions` with concurrent execution so that slow providers don't block the entire batch.

**Approach**:

1. **Refactor `check_versions` in `src/version_check.rs`** to use `futures::stream::iter` + `buffer_unordered`:
   ```rust
   use futures_util::stream::{self, StreamExt};

   pub async fn check_versions(
       configs: &[SoftwareConfig],
       registry: &ProviderRegistry,
   ) -> Vec<VersionCheckResult> {
       stream::iter(configs)
           .map(|config| async move {
               let result = check_version(config, registry).await;
               (config.clone(), result)
           })
           .buffer_unordered(8) // up to 8 checks in parallel
           .collect::<Vec<_>>()
           .await
           .into_iter()
           .map(|(config, result)| match result {
               Ok(version) => VersionCheckResult {
                   software_id: config.software_id,
                   installed_version: version,
                   error: None,
               },
               Err(e) => VersionCheckResult {
                   software_id: config.software_id,
                   installed_version: None,
                   error: Some(e.to_string()),
               },
           })
           .collect()
   }
   ```

2. **Add `futures-util` dependency** to `crates/core/agent/Cargo.toml` (if not already present).

3. **Add tests** verifying that:
   - Multiple configs are checked concurrently (mock provider with artificial delay).
   - Individual failures are isolated and don't abort other checks.

**Files modified**: `src/version_check.rs`, `Cargo.toml`

---

### Fix Plan 12 — Implement `detect_current_version` via Provider Registry (Finding #11a)

**Goal**: Replace the no-op `detect_current_version` stub with a real delegation to the provider registry's `check_version` function.

**Approach**:

1. **In `src/update.rs`**, change `detect_current_version` to delegate to the provider:
   ```rust
   async fn detect_current_version(
       software: &SoftwareConfig,
       registry: &ProviderRegistry,
   ) -> Option<String> {
       match crate::version_check::check_version(software, registry).await {
           Ok(version) => version,
           Err(e) => {
               tracing::warn!(
                   software_id = %software.software_id,
                   error = %e,
                   "failed to detect current version"
               );
               None
           }
       }
   }
   ```

2. **Update all call sites** of `detect_current_version` to pass the `ProviderRegistry` reference (it's already available in the update execution context).

3. **Remove the old stub** body that unconditionally returns `None`.

4. **Add a test** with a mock provider that returns a known version, verifying it flows through correctly.

**Files modified**: `src/update.rs`

---

### Fix Plan 13 — Decouple MQTT-specific Field from Agent's Disconnecting Message (Finding #11c)

**Goal**: Remove the requirement for the agent to populate `active_mqtt_clients: vec![]` when it has nothing to do with MQTT.

**Approach**:

1. **In `crates/shared/wire/src/lib.rs`**, add a convenience constructor to `DisconnectingPayload`:
   ```rust
   impl DisconnectingPayload {
       /// Create a `DisconnectingPayload` for non-MQTT services (agents).
       pub fn new(reason: String) -> Self {
           Self {
               reason,
               active_mqtt_clients: Vec::new(),
           }
       }
   }
   ```

2. **In `src/client.rs`** (agent crate), replace the manual struct literal:
   ```rust
   // Before:
   ServiceMessage::Disconnecting(DisconnectingPayload {
       reason: "graceful shutdown".into(),
       active_mqtt_clients: vec![],
   })

   // After:
   ServiceMessage::Disconnecting(DisconnectingPayload::new("graceful shutdown".into()))
   ```

3. **Long-term consideration** (out of scope for this fix): refactor `DisconnectingPayload` into an enum or use `#[serde(default)]` so MQTT-specific data is in a separate variant. This fix plan addresses the immediate coupling with a minimal, non-breaking change.

**Files modified**: `crates/shared/wire/src/lib.rs`, `src/client.rs`
