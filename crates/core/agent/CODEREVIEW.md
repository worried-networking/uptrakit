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
| 8 | Medium | Architecture | Duplicate update dispatch (provider trait vs hardcoded) | Open |
| 11a | Low | Completeness | `detect_current_version` is a no-op | Open |

---

## Detailed Findings

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

### 11a. `detect_current_version` is a no-op (update.rs)

```rust
async fn detect_current_version(_payload: &ExecuteUpdatePayload) -> Option<String> {
    // TODO: Implement actual version detection based on provider_type
    None
}
```

Both `from_version` and `to_version` detection always return `None`, making `UpdateResultPayload.from_version` and `.to_version` useless. This is acknowledged with a TODO.

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
- Shell injection prevention via `shell_escape()` and direct `Command::new().args()` (no shell)
- Async file I/O using `tokio::fs` instead of blocking `std::fs`
- Exponential backoff with jitter for reconnection (base 2s, max 60s)
- Bounded output accumulation (10 MB cap) to prevent OOM
- Explicit logging of swallowed task panics in stdout/stderr readers
- Comprehensive test coverage for `client.rs` and `error.rs`
- Robust TLS error detection via structured downcast (no string matching)
- Concurrent version checks with bounded parallelism (`buffer_unordered(8)`)
- `extract_service_id` returns error instead of silent "unknown" fallback
- `DisconnectingPayload::new()` decouples MQTT-specific fields from agent
- Minimal dependency footprint (unused `serde` and `provider-core` removed)

---

## Remaining Fix Plans

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
