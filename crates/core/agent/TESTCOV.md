# Test Coverage: uptrakit-agent

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 57.4% (686 / 1,196) |
| Function coverage | 78.0% (78 / 100) |
| Test count | 38 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| cli.rs | 100.0% | 139/139 | 100.0% | 14/14 |
| error.rs | 100.0% | 53/53 | 100.0% | 9/9 |
| version_check.rs | 93.9% | 108/115 | 92.9% | 13/14 |
| update.rs | 77.2% | 301/390 | 93.9% | 31/33 |
| host_info.rs | 76.8% | 53/69 | 100.0% | 5/5 |
| client.rs | 9.1% | 32/351 | 33.3% | 6/18 |
| main.rs | 0.0% | 0/79 | 0.0% | 0/7 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Client lifecycle** (`client.rs`, 351 lines): Authenticated WebSocket loop, message dispatching (version check requests,
  update execution commands, settings sync), reconnection logic, and graceful shutdown. The `compute_local_ca_hash` helper is
  now well-tested (6 tests), but the main WebSocket loop and message dispatch remain uncovered. Risk: client bugs could cause
  the agent to silently stop responding to controller commands.
- **Update execution gaps** (`update.rs`, 77.2% coverage, 390 lines): 89 uncovered lines include update hook execution
  (pre/post hooks), provider-specific update dispatch, output streaming, and error status reporting. Risk: update failures could
  leave software in an inconsistent state.
- **Host info collection gaps** (`host_info.rs`, 76.8% coverage): 16 uncovered lines in OS-specific detection edge cases.

### Tier 3 — Supporting

- **Main entry point** (`main.rs`, 0% coverage, 79 lines): Service startup and `AgentHandler` trait implementation.
- **Version check gaps** (`version_check.rs`, 93.9% coverage): 7 uncovered lines in error handling for provider failures.

## Test Recommendations

1. **Client message dispatch tests** — Test each controller message type (VersionCheckRequest, ExecuteUpdate, SettingsSync,
   Ping) handling in isolation. Covers `client.rs` (Tier 2). Use mock WebSocket with pre-built message sequences.
2. **Update hook execution tests** — Test pre/post hook execution, hook failure handling, and hook timeout. Covers `update.rs`
   gaps (Tier 2). Mock `CommandExecutor` to simulate hook outcomes.
3. **Client reconnection tests** — Test reconnection after WebSocket disconnect, backoff timing, and state recovery. Covers
   `client.rs` (Tier 2). Use mock WebSocket server that drops connections.
4. **Update output streaming tests** — Test stdout/stderr line capture and status reporting during update execution. Covers
   `update.rs` gaps (Tier 2). Mock `CommandExecutor` with streaming output.
5. **Host info platform edge cases** — Test OS detection with unusual `/etc/os-release` content and missing commands. Covers
   `host_info.rs` gaps (Tier 2). Mock file system reads.
