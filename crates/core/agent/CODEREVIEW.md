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

All findings have been addressed.

| # | Severity | Category | Issue | Status |
|---|----------|----------|-------|--------|
| 8 | Medium | Architecture | Duplicate update dispatch (provider trait vs hardcoded) | **Implemented** |
| 11a | Low | Completeness | `detect_current_version` is a no-op | **Implemented** |

---

## Implemented Fix Plans

### Fix Plan 6: Unify Update Dispatch Through Provider Registry (Finding #8) — Implemented

Update execution now routes through the Provider Registry, consistent with version checks:
- `execute_provider_update()` calls `ProviderRegistry::create_local_provider()` then `provider.execute_update(&ctx, &provider_tx)`
- Command execution utilities (`shell_escape`, `run_command_exec`, `run_command_with_shell`, `run_command`) moved to `provider-core::command` module
- Each provider (`GitHubLocalProvider`, `DockerRegistryLocalProvider`, `ProxmoxHelperScriptsLocalProvider`) implements `execute_update` with the update logic that was previously in the agent
- Agent's `run_hook_command` delegates to `provider_core::command` with a bridge mapping `UpdateOutputLine` -> `UpdateOutputMessage`
- `ReleaseInfo` moved from wire to provider-core (wire re-exports it)

### Fix Plan 12: Implement `detect_current_version` via Provider Registry (Finding #11a) — Implemented

`detect_current_version` now delegates to `crate::version_check::check_version()`, which uses the Provider Registry to detect installed versions. Errors are logged as warnings and gracefully return `None`.

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
- Unified update dispatch through Provider Registry (version checks and updates use the same path)
