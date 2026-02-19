# Test Coverage: uptrakit-agent-ssh

> Generated: 2026-02-19 | Tool: cargo-llvm-cov 0.8.4 | Rust: 1.93.1

## Summary

| Metric | Value |
| -------- | ------- |
| Line coverage | 60.6% (2,070 / 3,414) |
| Function coverage | 69.4% (267 / 385) |
| Test count | 161 |

## Coverage by Module

| File | Line % | Lines | Function % | Functions |
| ------ | -------- | ------- | ------------ | ----------- |
| db/entity/ssh_host.rs | 100.0% | 40/40 | 100.0% | 7/7 |
| db/migration/mod.rs | 100.0% | 7/7 | 100.0% | 3/3 |
| host_ops.rs | 98.8% | 338/342 | 100.0% | 35/35 |
| cli.rs | 97.2% | 445/458 | 96.9% | 31/32 |
| error.rs | 96.9% | 94/97 | 85.7% | 18/21 |
| ssh_target.rs | 94.5% | 260/275 | 95.1% | 39/41 |
| ssh_config.rs | 83.7% | 72/86 | 75.0% | 9/12 |
| ssh_executor.rs | 78.2% | 61/78 | 64.3% | 9/14 |
| ssh_key.rs | 67.5% | 191/283 | 68.6% | 24/35 |
| ssh_transport.rs | 50.2% | 207/412 | 56.9% | 29/51 |
| db/migration/m20260215_000001_initial.rs | 50.0% | 2/4 | 50.0% | 1/2 |
| host_info.rs | 46.6% | 82/176 | 57.1% | 16/28 |
| commands/bootstrap.rs | 38.8% | 150/387 | 70.6% | 24/34 |
| main.rs | 26.3% | 59/224 | 41.7% | 10/24 |
| client.rs | 15.4% | 32/208 | 46.2% | 6/13 |
| commands/host.rs | 0.0% | 0/307 | 0.0% | 0/27 |

## Uncovered Critical Paths

### Tier 2 — Business-Logic

- **Host management commands** (`commands/host.rs`, 0% coverage, 307 lines): Host add, remove, list, and credential management
  operations. Risk: untested host operations could corrupt the local SSH host database.
- **SSH transport** (`ssh_transport.rs`, 50.2% coverage, 412 lines): SSH connection establishment, session management,
  authentication negotiation, and host key verification. 205 uncovered lines include error recovery, timeout handling, and
  keyboard-interactive auth fallback. Risk: transport failures could cause silent connection drops.
- **SSH key management gaps** (`ssh_key.rs`): Remaining uncovered lines include the `read_private_key` stdin path and
  encrypted key handling.
- **Bootstrap command** (`commands/bootstrap.rs`, 38.8% coverage, 387 lines): Agent bootstrap flow including SSH host
  discovery, credential setup, and initial enrollment. Risk: bootstrap failures could leave the agent in an inconsistent state.
- **Client lifecycle** (`client.rs`, 15.4% coverage, 208 lines): Controller communication, authenticated loop, and message
  handling. Risk: client failures could cause the SSH agent to disconnect silently.
- **Host info collection gaps** (`host_info.rs`): Remaining uncovered lines are in the async SSH-dependent collection
  functions (`collect_remote_host_info`, `read_remote_*`). The parsing helpers are now fully tested.

### Tier 3 — Supporting

- **Main entry point** (`main.rs`, 26.3% coverage): Service startup and shutdown orchestration.
- **SSH executor gaps** (`ssh_executor.rs`, 78.2% coverage): Command execution timeout and error handling edge cases.
- **SSH config gaps** (`ssh_config.rs`, 83.7% coverage): SSH config file parsing edge cases.

## Test Recommendations

1. **Host management command tests** — Test host add/remove/list with mock database, credential CRUD, and error handling. Covers
   `commands/host.rs` (Tier 2). Use in-memory SQLite.
2. **SSH transport connection tests** — Test SSH connection with mock server, auth negotiation, host key verification, and
   timeout handling. Covers `ssh_transport.rs` gaps (Tier 2). Use `russh` test server or mock.
3. **Bootstrap flow integration tests** — Test the full bootstrap sequence with mock SSH server and controller. Covers
   `commands/bootstrap.rs` (Tier 2). Requires mock SSH and HTTP endpoints.
4. **Client authenticated loop tests** — Test message handling, reconnection, and graceful shutdown. Covers `client.rs` (Tier 2).
   Use in-memory WebSocket pairs.
